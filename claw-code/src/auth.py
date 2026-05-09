from __future__ import annotations

import json
import os
from pathlib import Path
from dataclasses import dataclass


@dataclass(frozen=True)
class OAuthTokenSet:
    access_token: str
    refresh_token: str | None = None
    expires_at: int | None = None
    scopes: list[str] = None


def get_gemini_home_dir() -> Path:
    home = os.environ.get('HOME') or os.environ.get('USERPROFILE')
    if not home:
        raise RuntimeError('HOME environment variable not set')
    return Path(home) / '.gemini'


def load_gemini_oauth_credentials() -> OAuthTokenSet | None:
    path = get_gemini_home_dir() / 'oauth_creds.json'
    if not path.exists():
        return None
    
    try:
        with open(path, 'r') as f:
            data = json.load(f)
        
        scopes = data.get('scope', '').split()
        return OAuthTokenSet(
            access_token=data['access_token'],
            refresh_token=data.get('refresh_token'),
            expires_at=data.get('expiry_date'),
            scopes=scopes
        )
    except (json.JSONDecodeError, KeyError, IOError):
        return None
