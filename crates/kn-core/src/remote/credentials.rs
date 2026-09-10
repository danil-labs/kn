//! Credentials for MCP servers. They live in the operating system's secure store,
//! never in documents, arguments, receipts or KN_HOME.
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Mutex};

/// Environment variable through which an integrating application hands over an
/// access token for the MCP server it already authorized. Never read from files.
pub const ACCESS_TOKEN_ENV: &str = "KN_MCP_ACCESS_TOKEN";

/// No Debug: a panic message must never print a token.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds; None when the server did not declare an expiry.
    pub expires_at: Option<u64>,
    pub token_endpoint: String,
    pub client_id: String,
    /// Canonical MCP server URI the token is bound to (RFC 8707 resource).
    pub resource: String,
}
impl Tokens {
    pub fn expires_soon(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|at| at <= now + 60)
    }
}

pub trait CredentialStore {
    fn load(&self, key: &str) -> Result<Option<Tokens>>;
    fn save(&self, key: &str, tokens: &Tokens) -> Result<()>;
    /// Returns whether an entry existed.
    fn delete(&self, key: &str) -> Result<bool>;
}

/// Keychain entry name: one per local workspace and remote alias.
pub fn credential_key(workspace: uuid::Uuid, alias: &str) -> String {
    format!("mcp/{workspace}/{alias}")
}

/// macOS Keychain, Windows Credential Manager or the Linux kernel keyring.
pub struct Keychain;
const SERVICE: &str = "kn";
fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key).map_err(store_error)
}
fn store_error(e: keyring::Error) -> Error {
    Error::Io(std::io::Error::other(format!(
        "No se pudo usar el almacén seguro del sistema: {e}"
    )))
}
impl CredentialStore for Keychain {
    fn load(&self, key: &str) -> Result<Option<Tokens>> {
        match entry(key)?.get_password() {
            Ok(secret) => Ok(Some(serde_json::from_str(&secret)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }
    fn save(&self, key: &str, tokens: &Tokens) -> Result<()> {
        entry(key)?
            .set_password(&serde_json::to_string(tokens)?)
            .map_err(store_error)
    }
    fn delete(&self, key: &str) -> Result<bool> {
        match entry(key)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(store_error(e)),
        }
    }
}

/// In-process store for tests and embedding applications that keep their own vault.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, Tokens>>);
impl MemoryStore {
    fn map(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Tokens>>> {
        self.0
            .lock()
            .map_err(|_| Error::Io(std::io::Error::other("Almacén de credenciales inválido.")))
    }
}
impl CredentialStore for MemoryStore {
    fn load(&self, key: &str) -> Result<Option<Tokens>> {
        Ok(self.map()?.get(key).cloned())
    }
    fn save(&self, key: &str, tokens: &Tokens) -> Result<()> {
        self.map()?.insert(key.into(), tokens.clone());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<bool> {
        Ok(self.map()?.remove(key).is_some())
    }
}
