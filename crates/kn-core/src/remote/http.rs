//! HTTP boundary for remote servers. Redirects are never followed, so a bearer
//! token only reaches the endpoint the person configured.
use crate::error::{Error, Result};
use std::{io::Read, time::Duration};
use ureq::http::Uri;

const MAX_BODY: u64 = 512 * 1024 * 1024;

#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
}
pub struct Response {
    pub status: u16,
    headers: Vec<(String, String)>,
    body: Box<dyn Read>,
}
impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn content_type(&self) -> &str {
        self.header("content-type").unwrap_or("")
    }
    pub fn bytes(self) -> Result<Vec<u8>> {
        let mut out = vec![];
        self.body
            .take(MAX_BODY)
            .read_to_end(&mut out)
            .map_err(unavailable)?;
        Ok(out)
    }
    pub fn reader(self) -> Box<dyn Read> {
        Box::new(self.body.take(MAX_BODY))
    }
}
pub fn unavailable(e: impl std::fmt::Display) -> Error {
    Error::RemoteUnavailable(format!("Falló la conexión con el servidor remoto: {e}"))
}
impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}
impl Http {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent(concat!("kn/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
    pub fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        validate_url(url)?;
        let mut request = self.agent.post(url);
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        convert(request.send(body))
    }
    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        validate_url(url)?;
        let mut request = self.agent.get(url);
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        convert(request.call())
    }
}
fn convert(
    result: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Response> {
    let response = result.map_err(unavailable)?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    Ok(Response {
        status,
        headers,
        body: Box::new(response.into_body().into_reader()),
    })
}

/// Accept https, or plain http only on loopback for servers on this machine.
pub fn validate_url(url: &str) -> Result<Uri> {
    let invalid = || Error::Invalid(format!("URL inválida: {url}"));
    let uri: Uri = url.parse().map_err(|_| invalid())?;
    let authority = uri.authority().ok_or_else(invalid)?;
    if authority.as_str().contains('@') {
        return Err(Error::Invalid(
            "La URL no puede incluir credenciales.".into(),
        ));
    }
    match uri.scheme_str() {
        Some("https") => Ok(uri),
        Some("http") if is_loopback(authority.host()) => Ok(uri),
        _ => Err(Error::Invalid(
            "Solo se admiten URLs https, o http en loopback para servidores locales.".into(),
        )),
    }
}
pub fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

#[cfg(test)]
mod tests {
    use super::validate_url;
    #[test]
    fn only_https_or_loopback_http_without_userinfo() {
        assert!(validate_url("https://mcp.example.com/mcp").is_ok());
        assert!(validate_url("http://127.0.0.1:8080/mcp").is_ok());
        assert!(validate_url("http://localhost/mcp").is_ok());
        assert!(validate_url("http://mcp.example.com/mcp").is_err());
        assert!(validate_url("https://user:pass@mcp.example.com/").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("not a url").is_err());
    }
}
