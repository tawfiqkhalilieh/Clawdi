use std::collections::VecDeque;
use std::sync::Mutex;
use std::pin::Pin;
use crate::error::ApiError;
use crate::types::{
    InputContentBlock, MessageRequest, MessageResponse, 
    StreamEvent, OutputContentBlock, Usage
};
use super::{Provider, ProviderFuture};
use crate::sse::SseParser;
use serde::Deserialize;
use serde_json::{json, Value};
use futures_util::{Stream, StreamExt};
use once_cell::sync::Lazy;

pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
pub const CLOUDCODE_PA_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com/v1internal";
pub const SYNTHETIC_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

static CACHED_PROJECT_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone)]
pub struct GeminiClient {
    client: reqwest::Client,
    base_url: String,
}

impl GeminiClient {
    pub fn new(client: reqwest::Client, base_url: String) -> Self {
        Self { client, base_url }
    }

    pub fn from_env() -> Result<Self, ApiError> {
        Ok(Self::new(reqwest::Client::new(), DEFAULT_BASE_URL.to_string()))
    }

    fn get_auth(&self) -> Result<(String, bool), ApiError> {
        if let Ok(api_key) = std::env::var("GEMINI_API_KEY") {
            return Ok((api_key, false));
        }
        match runtime::load_gemini_oauth_credentials() {
            Ok(Some(tokens)) => Ok((tokens.access_token, true)),
            _ => Err(ApiError::missing_credentials("Gemini", &["GEMINI_API_KEY"])),
        }
    }

    async fn ensure_project_id(&self, token: &str) -> Option<String> {
        if let Ok(project_id) = std::env::var("GOOGLE_CLOUD_PROJECT").or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT_ID")) {
            return Some(project_id);
        }

        {
            let cache = CACHED_PROJECT_ID.lock().unwrap();
            if let Some(id) = &*cache {
                return Some(id.clone());
            }
        }

        let url = format!("{}:loadCodeAssist", CLOUDCODE_PA_ENDPOINT);
        let payload = json!({
            "metadata": {
                "ideType": "IDE_UNSPECIFIED",
                "platform": "PLATFORM_UNSPECIFIED",
                "pluginType": "GEMINI"
            }
        });

        let res = self.client.post(&url)
            .bearer_auth(token)
            .header("User-Agent", "GeminiCLI/0.42.0/port")
            .json(&payload)
            .send()
            .await;

        if let Ok(response) = res {
            if let Ok(data) = response.json::<Value>().await {
                if let Some(id) = data.get("cloudaicompanionProject").and_then(|id| id.as_str()) {
                    let mut cache = CACHED_PROJECT_ID.lock().unwrap();
                    *cache = Some(id.to_string());
                    return Some(id.to_string());
                }
            }
        }

        None
    }

    fn resolve_model(&self, model: &str) -> String {
        match model {
            "gemini-3" | "gemini-3-flash" => "gemini-3-flash-preview".to_string(),
            "gemini-3-pro" => "gemini-3-pro-preview".to_string(),
            "gemini-2.5-flash" => "gemini-2.5-flash".to_string(),
            "gemini-2.5-pro" => "gemini-2.5-pro".to_string(),
            "gemini-2.0-flash" => "gemini-2.0-flash".to_string(),
            _ => model.to_string(),
        }
    }

    fn normalize_schema(&self, schema: &Value) -> Value {
        match schema {
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    if k == "type" {
                        match v {
                            Value::Array(arr) => {
                                let first_type = arr.iter()
                                    .find(|t| t.as_str() != Some("null"))
                                    .or_else(|| arr.first())
                                    .cloned()
                                    .unwrap_or(Value::String("string".to_string()));
                                new_map.insert(k.clone(), first_type);
                            }
                            _ => {
                                new_map.insert(k.clone(), v.clone());
                            }
                        }
                    } else if k == "properties" {
                        match v {
                            Value::Object(props) => {
                                let mut new_props = serde_json::Map::new();
                                for (pk, pv) in props {
                                    new_props.insert(pk.clone(), self.normalize_schema(pv));
                                }
                                new_map.insert(k.clone(), Value::Object(new_props));
                            }
                            _ => {
                                new_map.insert(k.clone(), v.clone());
                            }
                        }
                    } else if k == "items" {
                        new_map.insert(k.clone(), self.normalize_schema(v));
                    } else {
                        new_map.insert(k.clone(), v.clone());
                    }
                }
                Value::Object(new_map)
            }
            _ => schema.clone(),
        }
    }

    fn map_request(&self, request: &MessageRequest) -> Value {
        let mut contents = Vec::new();
        for msg in &request.messages {
            let mut parts = Vec::new();
            
            let mut last_thinking_signature = None;

            for block in &msg.content {
                match block {
                    InputContentBlock::Text { text } => {
                        parts.push(json!({ "text": text }));
                    }
                    InputContentBlock::Thinking { thinking, signature } => {
                        let mut part = json!({ "thought": thinking });
                        if let Some(sig) = signature {
                            part["thoughtSignature"] = json!(sig);
                            last_thinking_signature = Some(sig.clone());
                        }
                        parts.push(part);
                    }
                    InputContentBlock::ToolUse { id: _, name, input } => {
                        let mut part = json!({
                            "functionCall": {
                                "name": name,
                                "args": input
                            }
                        });
                        // Gemini 3 requires thoughtSignature on the PART if it follows a thought
                        // Error message says "thought_signature" (likely Proto name)
                        // but JSON mapping is usually "thoughtSignature".
                        if let Some(sig) = &last_thinking_signature {
                            part["thoughtSignature"] = json!(sig);
                        } else if msg.role == "assistant" {
                            // Based on gemini-cli core, the first function call in a model turn
                            // should have a thought signature.
                            part["thoughtSignature"] = json!(SYNTHETIC_THOUGHT_SIGNATURE);
                        }
                        parts.push(part);
                    }
                    InputContentBlock::ToolResult { tool_use_id: _, content, is_error: _ } => {
                        for c in content {
                            match c {
                                crate::types::ToolResultContentBlock::Text { text } => {
                                    parts.push(json!({
                                        "functionResponse": {
                                            "name": "TODO",
                                            "response": { "result": text }
                                        }
                                    }));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            contents.push(json!({
                "role": if msg.role == "assistant" { "model" } else { "user" },
                "parts": parts
            }));
        }

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": request.max_tokens,
                "temperature": request.temperature,
                "topP": request.top_p,
                "stopSequences": request.stop,
            }
        });

        if let Some(system) = &request.system {
            payload["systemInstruction"] = json!({
                "parts": [{ "text": system }]
            });
        }

        if let Some(tools) = &request.tools {
            let mut functions = Vec::new();
            for tool in tools {
                functions.push(json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": self.normalize_schema(&tool.input_schema)
                }));
            }
            payload["tools"] = json!([{ "functionDeclarations": functions }]);
        }

        payload
    }
}

impl Provider for GeminiClient {
    type Stream = MessageStream;

    fn send_message<'a>(
        &'a self,
        request: &'a MessageRequest,
    ) -> ProviderFuture<'a, MessageResponse> {
        let (token, is_oauth) = match self.get_auth() {
            Ok(t) => t,
            Err(e) => return Box::pin(async { Err(e) }),
        };
        
        let request_payload = self.map_request(request);
        let model = self.resolve_model(&request.model);
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        Box::pin(async move {
            let (url, payload) = if is_oauth {
                let project_id: Option<String> = {
                    let cache = CACHED_PROJECT_ID.lock().unwrap();
                    cache.as_ref().cloned()
                };
                
                let project_id = if project_id.is_none() {
                     let temp_client = GeminiClient::new(client.clone(), base_url.clone());
                     temp_client.ensure_project_id(&token).await
                } else {
                    project_id
                };

                (
                    format!("{}:generateContent", CLOUDCODE_PA_ENDPOINT),
                    json!({
                        "model": model,
                        "project": project_id,
                        "user_prompt_id": "gemini-port-prompt",
                        "request": request_payload
                    })
                )
            } else {
                (
                    format!("{}/v1beta/models/{}:generateContent", base_url, model),
                    request_payload
                )
            };

            let res = client.post(&url)
                .bearer_auth(token)
                .header("User-Agent", "GeminiCLI/0.42.0/port")
                .json(&payload)
                .send()
                .await
                .map_err(ApiError::Http)?;

            if !res.status().is_success() {
                let status = res.status();
                let body = res.text().await.unwrap_or_default();
                return Err(ApiError::Api {
                    status,
                    error_type: None,
                    message: Some(body.clone()),
                    request_id: None,
                    body,
                    retryable: status.is_server_error(),
                    suggested_action: None,
                });
            }

            let body = res.text().await.map_err(ApiError::Http)?;
            let mut gemini_res_val: Value = serde_json::from_str(&body).map_err(|e| ApiError::Json {
                provider: "Gemini".to_string(),
                model: model.clone(),
                body_snippet: body.chars().take(100).collect(),
                source: e,
            })?;

            if is_oauth {
                gemini_res_val = gemini_res_val.get("response").cloned().unwrap_or(gemini_res_val);
            }

            let gemini_res: GeminiResponse = serde_json::from_value(gemini_res_val).map_err(|e| ApiError::Json {
                provider: "Gemini".to_string(),
                model: model.clone(),
                body_snippet: "TODO".to_string(),
                source: e,
            })?;

            let mut content = Vec::new();
            if let Some(candidate) = gemini_res.candidates.first() {
                for part in &candidate.content.parts {
                    if let Some(text) = &part.text {
                        content.push(OutputContentBlock::Text { text: text.clone() });
                    }
                    if let Some(thinking) = &part.thought {
                        content.push(OutputContentBlock::Thinking {
                            thinking: thinking.clone(),
                            signature: part.thought_signature.clone(),
                        });
                    }
                    if let Some(call) = &part.function_call {
                        content.push(OutputContentBlock::ToolUse {
                            id: "TODO".to_string(),
                            name: call.name.clone(),
                            input: call.args.clone(),
                        });
                    }
                }
            }

            Ok(MessageResponse {
                id: "TODO".to_string(),
                kind: "message".to_string(),
                role: "assistant".to_string(),
                content,
                model: model.clone(),
                stop_reason: Some("end_turn".to_string()),
                stop_sequence: None,
                usage: Usage {
                    input_tokens: gemini_res.usage_metadata.prompt_token_count,
                    output_tokens: gemini_res.usage_metadata.candidates_token_count,
                    ..Default::default()
                },
                request_id: None,
            })
        })
    }

    fn stream_message<'a>(
        &'a self,
        request: &'a MessageRequest,
    ) -> ProviderFuture<'a, Self::Stream> {
        let (token, is_oauth) = match self.get_auth() {
            Ok(t) => t,
            Err(e) => return Box::pin(async { Err(e) }),
        };
        
        let request_payload = self.map_request(request);
        let model = self.resolve_model(&request.model);
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        Box::pin(async move {
            let (url, payload) = if is_oauth {
                let project_id: Option<String> = {
                    let cache = CACHED_PROJECT_ID.lock().unwrap();
                    cache.as_ref().cloned()
                };
                
                let project_id = if project_id.is_none() {
                    let temp_client = GeminiClient::new(client.clone(), base_url.clone());
                    temp_client.ensure_project_id(&token).await
                } else {
                    project_id
                };

                (
                    format!("{}:streamGenerateContent?alt=sse", CLOUDCODE_PA_ENDPOINT),
                    json!({
                        "model": model,
                        "project": project_id,
                        "user_prompt_id": "gemini-port-prompt",
                        "request": request_payload
                    })
                )
            } else {
                (
                    format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", base_url, model),
                    request_payload
                )
            };

            let res = client.post(&url)
                .bearer_auth(token)
                .header("User-Agent", "GeminiCLI/0.42.0/port")
                .json(&payload)
                .send()
                .await
                .map_err(ApiError::Http)?;

            Ok(MessageStream {
                request_id: None,
                stream: Box::pin(res.bytes_stream()),
                parser: SseParser::new().with_context("Gemini", model.clone()),
                pending: VecDeque::new(),
                done: false,
            })
        })
    }
}

pub struct MessageStream {
    request_id: Option<String>,
    stream: Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    parser: SseParser,
    pending: VecDeque<StreamEvent>,
    done: bool,
}

impl std::fmt::Debug for MessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageStream")
            .field("request_id", &self.request_id)
            .field("parser", &self.parser)
            .field("pending", &self.pending)
            .field("done", &self.done)
            .finish()
    }
}

impl MessageStream {
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub async fn next_event(&mut self) -> Result<Option<StreamEvent>, ApiError> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }

        if self.done {
            return Ok(None);
        }

        while let Some(chunk_res) = self.stream.next().await {
            let chunk = chunk_res.map_err(ApiError::Http)?;
            let frames = self.parser.push(&chunk)?;
            for event in frames {
                self.pending.push_back(event);
            }

            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
        }

        self.done = true;
        let final_events = self.parser.finish()?;
        self.pending.extend(final_events);
        Ok(self.pending.pop_front())
    }
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<Candidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: UsageMetadata,
}

#[derive(Deserialize)]
struct Candidate {
    content: Content,
}

#[derive(Deserialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct Part {
    text: Option<String>,
    thought: Option<String>,
    #[serde(rename = "thoughtSignature")]
    thought_signature: Option<String>,
    #[serde(rename = "functionCall")]
    function_call: Option<FunctionCall>,
}

#[derive(Deserialize)]
struct FunctionCall {
    name: String,
    args: Value,
}

#[derive(Deserialize)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: u32,
}
