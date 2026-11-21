use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;

pub const KEYRING_SERVICE_NAME: &str = "video-editing-app-ag";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiProvider {
    Gemini,
    Claude,
}

impl ApiProvider {
    pub fn account_key(self) -> &'static str {
        match self {
            Self::Gemini => "gemini_api_key",
            Self::Claude => "claude_api_key",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini API",
            Self::Claude => "Claude API",
        }
    }
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("OS credential store error: {0}")]
    Backend(String),
    #[error("API key not configured for {0}")]
    NotFound(&'static str),
}

/// APIキーの安全な保管バックエンド（`project.json` には一切保存しない）
pub trait CredentialStore: Send + Sync {
    fn set_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), CredentialError>;
    fn get_api_key(&self, provider: ApiProvider) -> Result<String, CredentialError>;
    fn delete_api_key(&self, provider: ApiProvider) -> Result<(), CredentialError>;

    fn has_api_key(&self, provider: ApiProvider) -> bool {
        self.get_api_key(provider)
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }
}

/// Windows Credential Manager / macOS Keychain / Linux Keyutils を利用する OS 資格情報ストア
#[derive(Debug, Default)]
pub struct OsCredentialStore;

impl OsCredentialStore {
    pub fn new() -> Self {
        Self
    }
}

impl CredentialStore for OsCredentialStore {
    fn set_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), CredentialError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE_NAME, provider.account_key())
            .map_err(|e| CredentialError::Backend(e.to_string()))?;
        entry
            .set_password(key)
            .map_err(|e| CredentialError::Backend(e.to_string()))
    }

    fn get_api_key(&self, provider: ApiProvider) -> Result<String, CredentialError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE_NAME, provider.account_key())
            .map_err(|e| CredentialError::Backend(e.to_string()))?;
        match entry.get_password() {
            Ok(pw) if !pw.is_empty() => Ok(pw),
            Ok(_) | Err(keyring::Error::NoEntry) => {
                Err(CredentialError::NotFound(provider.display_name()))
            }
            Err(e) => Err(CredentialError::Backend(e.to_string())),
        }
    }

    fn delete_api_key(&self, provider: ApiProvider) -> Result<(), CredentialError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE_NAME, provider.account_key())
            .map_err(|e| CredentialError::Backend(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CredentialError::Backend(e.to_string())),
        }
    }
}

/// テスト・ヘッドレス環境向けのインメモリ資格情報ストア
#[derive(Debug, Default)]
pub struct InMemoryCredentialStore {
    keys: Mutex<HashMap<ApiProvider, String>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn set_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), CredentialError> {
        self.keys.lock().unwrap().insert(provider, key.to_string());
        Ok(())
    }

    fn get_api_key(&self, provider: ApiProvider) -> Result<String, CredentialError> {
        self.keys
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .filter(|s| !s.is_empty())
            .ok_or(CredentialError::NotFound(provider.display_name()))
    }

    fn delete_api_key(&self, provider: ApiProvider) -> Result<(), CredentialError> {
        self.keys.lock().unwrap().remove(&provider);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_store_roundtrip_and_delete() {
        let store = InMemoryCredentialStore::new();
        assert!(!store.has_api_key(ApiProvider::Gemini));
        store
            .set_api_key(ApiProvider::Gemini, "secret-123")
            .unwrap();
        assert!(store.has_api_key(ApiProvider::Gemini));
        assert_eq!(
            store.get_api_key(ApiProvider::Gemini).unwrap(),
            "secret-123"
        );
        store.delete_api_key(ApiProvider::Gemini).unwrap();
        assert!(!store.has_api_key(ApiProvider::Gemini));
    }
}
