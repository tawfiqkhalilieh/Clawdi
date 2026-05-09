from __future__ import annotations

import json
import os
import requests
from typing import Any, Generator
from .auth import load_gemini_oauth_credentials

SYNTHETIC_THOUGHT_SIGNATURE = 'skip_thought_signature_validator'


class GeminiClient:
    _cached_project_id: str | None = None

    def __init__(self, model: str = 'gemini-3-flash'):
        self.model = self._resolve_model(model)
        self.base_url = 'https://generativelanguage.googleapis.com'
        self.is_oauth = False
        self.project_id = os.environ.get('GOOGLE_CLOUD_PROJECT') or os.environ.get('GOOGLE_CLOUD_PROJECT_ID')

    def _resolve_model(self, model: str) -> str:
        model = model.lower().strip()
        mapping = {
            'gemini-3': 'gemini-3-flash-preview',
            'gemini-3-flash': 'gemini-3-flash-preview',
            'gemini-3-pro': 'gemini-3-pro-preview',
            'gemini-3.1-pro': 'gemini-3.1-pro-preview',
            'gemini-2.5-flash': 'gemini-2.5-flash',
            'gemini-2.5-pro': 'gemini-2.5-pro',
            'gemini-2.0-flash': 'gemini-2.0-flash',
        }
        return mapping.get(model, model)

    def _get_headers(self) -> dict[str, str]:
        headers = {
            'Content-Type': 'application/json',
            'User-Agent': 'GeminiCLI/0.42.0/port (linux; x64; port)'
        }
        api_key = os.environ.get('GEMINI_API_KEY')
        if api_key:
            headers['x-goog-api-key'] = api_key
            self.is_oauth = False
        else:
            creds = load_gemini_oauth_credentials()
            if creds:
                headers['Authorization'] = f'Bearer {creds.access_token}'
                self.is_oauth = True
        return headers

    def _ensure_project_id(self, headers: dict[str, str]):
        if self.project_id:
            return
        
        if GeminiClient._cached_project_id:
            self.project_id = GeminiClient._cached_project_id
            return

        # Call loadCodeAssist to discover project ID
        url = 'https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist'
        payload = {'metadata': {'ideType': 'IDE_UNSPECIFIED', 'platform': 'PLATFORM_UNSPECIFIED', 'pluginType': 'GEMINI'}}
        try:
            response = requests.post(url, headers=headers, json=payload)
            response.raise_for_status()
            data = response.json()
            self.project_id = data.get('cloudaicompanionProject')
            GeminiClient._cached_project_id = self.project_id
        except Exception:
            pass

    def _prepare_payload(self, contents: list[dict[str, Any]], system_instruction: str | None = None, tools: list[dict[str, Any]] | None = None) -> dict[str, Any]:
        # Process contents to ensure thought signatures are present for function calls in model turns
        for content in contents:
            if content.get('role') == 'model':
                last_sig = None
                for part in content.get('parts', []):
                    if 'thought' in part and 'thoughtSignature' in part:
                        last_sig = part['thoughtSignature']
                    if 'functionCall' in part:
                        if last_sig:
                            part['thoughtSignature'] = last_sig
                        else:
                            part['thoughtSignature'] = SYNTHETIC_THOUGHT_SIGNATURE

        request_payload = {'contents': contents}
        if system_instruction:
            request_payload['systemInstruction'] = {
                'parts': [{'text': system_instruction}]
            }
        if tools:
            request_payload['tools'] = [{'functionDeclarations': tools}]

        if self.is_oauth:
            return {
                'model': self.model,
                'project': self.project_id,
                'user_prompt_id': 'gemini-port-prompt',
                'request': request_payload
            }
        else:
            return request_payload

    def generate_content(self, contents: list[dict[str, Any]], system_instruction: str | None = None, tools: list[dict[str, Any]] | None = None) -> dict[str, Any]:
        headers = self._get_headers()
        if self.is_oauth:
            self._ensure_project_id(headers)
            url = 'https://cloudcode-pa.googleapis.com/v1internal:generateContent'
        else:
            url = f'{self.base_url}/v1beta/models/{self.model}:generateContent'
        
        payload = self._prepare_payload(contents, system_instruction, tools)
        response = requests.post(url, headers=headers, json=payload)
        response.raise_for_status()
        data = response.json()
        
        if self.is_oauth:
            return data.get('response', {})
        return data

    def stream_generate_content(self, contents: list[dict[str, Any]], system_instruction: str | None = None, tools: list[dict[str, Any]] | None = None) -> Generator[dict[str, Any], None, None]:
        headers = self._get_headers()
        if self.is_oauth:
            self._ensure_project_id(headers)
            url = 'https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse'
        else:
            url = f'{self.base_url}/v1beta/models/{self.model}:streamGenerateContent?alt=sse'
        
        payload = self._prepare_payload(contents, system_instruction, tools)
        response = requests.post(url, headers=headers, json=payload, stream=True)
        response.raise_for_status()
        
        for line in response.iter_lines():
            if line:
                decoded_line = line.decode('utf-8')
                if decoded_line.startswith('data: '):
                    chunk = json.loads(decoded_line[6:])
                    if self.is_oauth:
                        yield chunk.get('response', {})
                    else:
                        yield chunk
