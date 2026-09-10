//! Authorization of kn as an MCP client (MCP 2025-11-25 authorization).
//! Placeholder: implemented in the OAuth task.
use super::{credentials::Tokens, http::Http};
use crate::error::{Error, Result};
use std::time::Duration;

/// Authorize kn against a protected MCP server and return tokens bound to it.
pub fn login(
    _http: &Http,
    _endpoint: &str,
    _open_browser: &dyn Fn(&str) -> Result<()>,
    _timeout: Duration,
) -> Result<Tokens> {
    Err(Error::Unsupported(
        "La autorización OAuth todavía no está implementada.".into(),
    ))
}

/// Exchange a refresh token for new tokens bound to the same resource.
pub fn refresh(_http: &Http, _tokens: &Tokens) -> Result<Tokens> {
    Err(Error::Unsupported(
        "La renovación OAuth todavía no está implementada.".into(),
    ))
}
