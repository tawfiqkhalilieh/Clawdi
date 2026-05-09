use crate::error::ApiError;
use crate::types::StreamEvent;

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    provider: Option<String>,
    model: Option<String>,
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach the provider name and model to this parser so that JSON
    /// deserialization failures within streamed frames carry enough context
    /// for callers to understand which upstream produced the unparseable
    /// payload.
    #[must_use]
    pub fn with_context(mut self, provider: impl Into<String>, model: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self.model = Some(model.into());
        self
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<StreamEvent>, ApiError> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some(frame) = self.next_frame() {
            if let Some(event) = self.parse_frame_with_context(&frame)? {
                events.push(event);
            }
        }

        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Vec<StreamEvent>, ApiError> {
        if self.buffer.is_empty() {
            return Ok(Vec::new());
        }

        let trailing = std::mem::take(&mut self.buffer);
        match self.parse_frame_with_context(&String::from_utf8_lossy(&trailing))? {
            Some(event) => Ok(vec![event]),
            None => Ok(Vec::new()),
        }
    }

    fn parse_frame_with_context(&self, frame: &str) -> Result<Option<StreamEvent>, ApiError> {
        let provider = self.provider.as_deref().unwrap_or("unknown");
        let model = self.model.as_deref().unwrap_or("unknown");
        parse_frame_with_provider(frame, provider, model)
    }

    fn next_frame(&mut self) -> Option<String> {
        let separator = self
            .buffer
            .windows(2)
            .position(|window| window == b"\n\n")
            .map(|position| (position, 2))
            .or_else(|| {
                self.buffer
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|position| (position, 4))
            })?;

        let (position, separator_len) = separator;
        let frame = self
            .buffer
            .drain(..position + separator_len)
            .collect::<Vec<_>>();
        let frame_len = frame.len().saturating_sub(separator_len);
        Some(String::from_utf8_lossy(&frame[..frame_len]).into_owned())
    }
}

pub fn parse_frame(frame: &str) -> Result<Option<StreamEvent>, ApiError> {
    parse_frame_with_provider(frame, "unknown", "unknown")
}

pub(crate) fn parse_frame_with_provider(
    frame: &str,
    provider: &str,
    model: &str,
) -> Result<Option<StreamEvent>, ApiError> {
    let trimmed = frame.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut data_lines = Vec::new();
    let mut event_name: Option<&str> = None;

    for line in trimmed.lines() {
        if line.starts_with(':') {
            continue;
        }
        if let Some(name) = line.strip_prefix("event:") {
            event_name = Some(name.trim());
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim_start());
        }
    }

    if matches!(event_name, Some("ping")) {
        return Ok(None);
    }

    if data_lines.is_empty() {
        // If it doesn't look like SSE (no data: prefix), but provider is Gemini,
        // it might be a raw JSON stream.
        if provider == "Gemini" {
            let payload = trimmed;
            if payload.starts_with('{') {
                return map_gemini_event(payload, provider, model);
            }
        }
        return Ok(None);
    }

    let payload = data_lines.join("\n");
    if payload == "[DONE]" {
        return Ok(None);
    }

    if provider == "Gemini" {
        return map_gemini_event(&payload, provider, model);
    }

    serde_json::from_str::<StreamEvent>(&payload)
        .map(Some)
        .map_err(|error| ApiError::json_deserialize(provider, model, &payload, error))
}

fn map_gemini_event(
    payload: &str,
    provider: &str,
    model: &str,
) -> Result<Option<StreamEvent>, ApiError> {
    let gemini_chunk = serde_json::from_str::<serde_json::Value>(payload)
        .map_err(|error| ApiError::json_deserialize(provider, model, payload, error))?;

    if let Some(candidates) = gemini_chunk.get("candidates").and_then(|c| c.as_array()) {
        if let Some(candidate) = candidates.first() {
            if let Some(content) = candidate.get("content") {
                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                    if let Some(part) = parts.first() {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            return Ok(Some(StreamEvent::ContentBlockDelta(
                                crate::types::ContentBlockDeltaEvent {
                                    index: 0,
                                    delta: crate::types::ContentBlockDelta::TextDelta {
                                        text: text.to_string(),
                                    },
                                },
                            )));
                        }
                    }
                }
            }
        }
    }

    // Handle usage metadata at the end of the stream
    if let Some(usage) = gemini_chunk.get("usageMetadata") {
        let input_tokens = usage.get("promptTokenCount").and_then(|t| t.as_u64()).unwrap_or(0) as u32;
        let output_tokens = usage.get("candidatesTokenCount").and_then(|t| t.as_u64()).unwrap_or(0) as u32;
        if input_tokens > 0 || output_tokens > 0 {
             return Ok(Some(StreamEvent::MessageDelta(crate::types::MessageDeltaEvent {
                delta: crate::types::MessageDelta {
                    stop_reason: Some("end_turn".to_string()),
                    stop_sequence: None,
                },
                usage: crate::types::Usage {
                    input_tokens,
                    output_tokens,
                    ..Default::default()
                },
            })));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{parse_frame, SseParser};
    use crate::types::{ContentBlockDelta, MessageDelta, OutputContentBlock, StreamEvent, Usage};

    #[test]
    fn parses_single_frame() {
        let frame = concat!(
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"Hi\"}}\n\n"
        );

        let event = parse_frame(frame).expect("frame should parse");
        assert_eq!(
            event,
            Some(StreamEvent::ContentBlockStart(
                crate::types::ContentBlockStartEvent {
                    index: 0,
                    content_block: OutputContentBlock::Text {
                        text: "Hi".to_string(),
                    },
                },
            ))
        );
    }

    #[test]
    fn parses_chunked_stream() {
        let mut parser = SseParser::new();
        let first = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel";
        let second = b"lo\"}}\n\n";

        assert!(parser
            .push(first)
            .expect("first chunk should buffer")
            .is_empty());
        let events = parser.push(second).expect("second chunk should parse");

        assert_eq!(
            events,
            vec![StreamEvent::ContentBlockDelta(
                crate::types::ContentBlockDeltaEvent {
                    index: 0,
                    delta: ContentBlockDelta::TextDelta {
                        text: "Hello".to_string(),
                    },
                }
            )]
        );
    }

    #[test]
    fn ignores_ping_and_done() {
        let mut parser = SseParser::new();
        let payload = concat!(
            ": keepalive\n",
            "event: ping\n",
            "data: {\"type\":\"ping\"}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
            "data: [DONE]\n\n"
        );

        let events = parser
            .push(payload.as_bytes())
            .expect("parser should succeed");
        assert_eq!(
            events,
            vec![
                StreamEvent::MessageDelta(crate::types::MessageDeltaEvent {
                    delta: MessageDelta {
                        stop_reason: Some("tool_use".to_string()),
                        stop_sequence: None,
                    },
                    usage: Usage {
                        input_tokens: 1,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: 0,
                        output_tokens: 2,
                    },
                }),
                StreamEvent::MessageStop(crate::types::MessageStopEvent {}),
            ]
        );
    }

    #[test]
    fn ignores_data_less_event_frames() {
        let frame = "event: ping\n\n";
        let event = parse_frame(frame).expect("frame without data should be ignored");
        assert_eq!(event, None);
    }

    #[test]
    fn parses_split_json_across_data_lines() {
        let frame = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\n",
            "data: \"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n"
        );

        let event = parse_frame(frame).expect("frame should parse");
        assert_eq!(
            event,
            Some(StreamEvent::ContentBlockDelta(
                crate::types::ContentBlockDeltaEvent {
                    index: 0,
                    delta: ContentBlockDelta::TextDelta {
                        text: "Hello".to_string(),
                    },
                }
            ))
        );
    }

    #[test]
    fn parses_thinking_content_block_start() {
        let frame = concat!(
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":null}}\n\n"
        );

        let event = parse_frame(frame).expect("frame should parse");
        assert_eq!(
            event,
            Some(StreamEvent::ContentBlockStart(
                crate::types::ContentBlockStartEvent {
                    index: 0,
                    content_block: OutputContentBlock::Thinking {
                        thinking: String::new(),
                        signature: None,
                    },
                },
            ))
        );
    }

    #[test]
    fn parses_thinking_related_deltas() {
        let thinking = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"step 1\"}}\n\n"
        );
        let signature = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig_123\"}}\n\n"
        );

        let thinking_event = parse_frame(thinking).expect("thinking delta should parse");
        let signature_event = parse_frame(signature).expect("signature delta should parse");

        assert_eq!(
            thinking_event,
            Some(StreamEvent::ContentBlockDelta(
                crate::types::ContentBlockDeltaEvent {
                    index: 0,
                    delta: ContentBlockDelta::ThinkingDelta {
                        thinking: "step 1".to_string(),
                    },
                }
            ))
        );
        assert_eq!(
            signature_event,
            Some(StreamEvent::ContentBlockDelta(
                crate::types::ContentBlockDeltaEvent {
                    index: 0,
                    delta: ContentBlockDelta::SignatureDelta {
                        signature: "sig_123".to_string(),
                    },
                }
            ))
        );
    }

    #[test]
    fn given_message_delta_frame_with_empty_usage_when_parsed_then_usage_defaults_to_zero() {
        // given
        let frame = concat!(
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{}}\n\n"
        );

        // when
        let event = parse_frame(frame).expect("frame should parse");

        // then
        assert_eq!(
            event,
            Some(StreamEvent::MessageDelta(crate::types::MessageDeltaEvent {
                delta: MessageDelta {
                    stop_reason: Some("end_turn".to_string()),
                    stop_sequence: None,
                },
                usage: Usage::default(),
            }))
        );
    }
}
