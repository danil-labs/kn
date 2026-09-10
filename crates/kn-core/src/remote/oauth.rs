//! Authorization of kn as a public MCP client (MCP 2025-11-25 authorization):
//! RFC 9728 discovery, RFC 8414 metadata, RFC 7591 registration, PKCE S256 and
//! RFC 8707 resource indicators, with a loopback redirect. Token values, codes
//! and verifiers never appear in errors or logs.
use super::{
    credentials::Tokens,
    http::{Http, Response, validate_url},
    now,
};
use crate::error::{Error, Result};
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

const EXPIRED: &str = "La sesión expiró; ejecuta kn remote login.";
const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Authorize kn against a protected MCP server and return tokens bound to it.
pub fn login(
    http: &Http,
    endpoint: &str,
    open_browser: &dyn Fn(&str) -> Result<()>,
    timeout: Duration,
) -> Result<Tokens> {
    let endpoint_uri = validate_url(endpoint)?;
    let (metadata_hint, challenge_scope) = probe(http, endpoint)?;
    let prm = protected_resource(http, &endpoint_uri, metadata_hint.as_deref())?;
    let prm_resource = prm["resource"].as_str().ok_or_else(|| {
        Error::Remote("La metadata del recurso protegido no declara resource.".into())
    })?;
    if canonical(prm_resource) != canonical(endpoint) {
        return Err(Error::Remote(format!(
            "La metadata del recurso protegido corresponde a {prm_resource}, no a {endpoint}; kn no envía tokens a otro recurso."
        )));
    }
    let issuer = prm["authorization_servers"]
        .as_array()
        .and_then(|servers| servers.first())
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::Remote(
                "La metadata del recurso protegido no declara servidores de autorización.".into(),
            )
        })?;
    let issuer = server_url(issuer, "el servidor de autorización")?;
    let server = authorization_server(http, &issuer)?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let redirect_uri = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr()?.port()
    );
    let client_id = register(http, &server, &redirect_uri)?;

    let verifier = random_string(64)?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let state = random_string(32)?;
    let scope = challenge_scope.or_else(|| {
        let scopes: Vec<&str> = prm["scopes_supported"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        (!scopes.is_empty()).then(|| scopes.join(" "))
    });
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state.as_str()),
        ("resource", endpoint),
    ];
    if let Some(scope) = &scope {
        params.push(("scope", scope));
    }
    let separator = if server.authorization.contains('?') {
        '&'
    } else {
        '?'
    };
    open_browser(&format!(
        "{}{separator}{}",
        server.authorization,
        form(&params)
    ))?;

    let code = wait_callback(&listener, &state, timeout)?;
    let body = token_call(
        http,
        &server.token,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &client_id),
            ("code_verifier", &verifier),
            ("resource", endpoint),
        ],
        |error| {
            Error::AuthRequired(format!(
                "El servidor de autorización rechazó el código ({error}); vuelve a iniciar sesión."
            ))
        },
    )?;
    parse_tokens(&body, None, server.token, client_id, endpoint.into())
}

/// Exchange a refresh token for new tokens bound to the same resource.
pub fn refresh(http: &Http, tokens: &Tokens) -> Result<Tokens> {
    let refresh_token = tokens
        .refresh_token
        .as_deref()
        .ok_or_else(|| Error::AuthRequired(EXPIRED.into()))?;
    let body = token_call(
        http,
        &tokens.token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &tokens.client_id),
            ("resource", &tokens.resource),
        ],
        |_| Error::AuthRequired(EXPIRED.into()),
    )?;
    parse_tokens(
        &body,
        Some(refresh_token),
        tokens.token_endpoint.clone(),
        tokens.client_id.clone(),
        tokens.resource.clone(),
    )
}

/// A parameter of the Bearer challenge in a `WWW-Authenticate` value; other
/// schemes, token68 values and parameter names are compared case-insensitively.
pub fn challenge_param(header: &str, name: &str) -> Option<String> {
    let s = header.as_bytes();
    let mut i = 0;
    let mut bearer = false;
    let token_end = |mut i: usize| {
        while i < s.len() && !matches!(s[i], b' ' | b'\t' | b',' | b'=') {
            i += 1;
        }
        i
    };
    let skip_ws = |mut i: usize| {
        while i < s.len() && matches!(s[i], b' ' | b'\t') {
            i += 1;
        }
        i
    };
    while i < s.len() {
        if matches!(s[i], b' ' | b'\t' | b',') {
            i += 1;
            continue;
        }
        let end = token_end(i);
        if end == i {
            // A stray '=' (token68 padding): skip it.
            i += 1;
            continue;
        }
        let token = &header[i..end];
        let after = skip_ws(end);
        if after >= s.len() || s[after] != b'=' {
            bearer = token.eq_ignore_ascii_case("bearer");
            i = end;
            continue;
        }
        let mut j = skip_ws(after + 1);
        let value = if j < s.len() && s[j] == b'"' {
            let start = j + 1;
            let mut value = String::new();
            let mut chars = header[start..].char_indices();
            j = s.len();
            while let Some((k, c)) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next().map(|(_, c)| c)),
                    '"' => {
                        j = start + k + 1;
                        break;
                    }
                    c => value.push(c),
                }
            }
            value
        } else {
            let start = j;
            while j < s.len() && !matches!(s[j], b' ' | b'\t' | b',') {
                j += 1;
            }
            header[start..j].to_string()
        };
        if bearer && token.eq_ignore_ascii_case(name) {
            return Some(value);
        }
        i = j;
    }
    None
}

/// Unauthenticated initialize: learn the resource metadata URL and scope.
fn probe(http: &Http, endpoint: &str) -> Result<(Option<String>, Option<String>)> {
    let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": super::mcp::PROTOCOL_VERSION, "capabilities": {},
            "clientInfo": {"name": "kn", "version": env!("CARGO_PKG_VERSION")}}}))?;
    let response = http.post(
        endpoint,
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ],
        &body,
    )?;
    match response.status {
        200..=299 => Err(Error::Invalid(
            "El servidor no exige autorización; no hace falta iniciar sesión.".into(),
        )),
        401 => {
            let header = response.header("www-authenticate").unwrap_or("");
            Ok((
                challenge_param(header, "resource_metadata"),
                challenge_param(header, "scope").filter(|s| !s.trim().is_empty()),
            ))
        }
        status => Err(status_error("El servidor MCP", status)),
    }
}

/// RFC 9728 metadata, from the challenge or the well-known locations.
fn protected_resource(
    http: &Http,
    endpoint: &ureq::http::Uri,
    hint: Option<&str>,
) -> Result<Value> {
    if let Some(hint) = hint {
        let url = server_url(hint, "la metadata del recurso protegido")?;
        return get_json(http, &url)?.ok_or_else(|| {
            Error::Remote(format!(
                "No se pudo obtener la metadata del recurso protegido en {url}."
            ))
        });
    }
    let origin = origin(endpoint);
    let path = endpoint.path().trim_end_matches('/');
    let mut candidates = vec![];
    if !path.is_empty() {
        candidates.push(format!(
            "{origin}/.well-known/oauth-protected-resource{path}"
        ));
    }
    candidates.push(format!("{origin}/.well-known/oauth-protected-resource"));
    first_json(http, &candidates)?.ok_or_else(|| {
        Error::Remote("El servidor no publica la metadata del recurso protegido (RFC 9728).".into())
    })
}

struct Server {
    authorization: String,
    token: String,
    registration: Option<String>,
}

/// RFC 8414 / OpenID Connect discovery for the issuer, in the order MCP requires.
fn authorization_server(http: &Http, issuer: &str) -> Result<Server> {
    let uri = validate_url(issuer)?;
    let origin = origin(&uri);
    let path = uri.path().trim_end_matches('/');
    let candidates = if path.is_empty() {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server{path}"),
            format!("{origin}/.well-known/openid-configuration{path}"),
            format!("{origin}{path}/.well-known/openid-configuration"),
        ]
    };
    let metadata = first_json(http, &candidates)?.ok_or_else(|| {
        Error::Remote(format!(
            "No se encontró la metadata del servidor de autorización {issuer}."
        ))
    })?;
    if let Some(declared) = metadata["issuer"].as_str()
        && canonical(declared) != canonical(issuer)
    {
        return Err(Error::Remote(format!(
            "La metadata del servidor de autorización declara el emisor {declared}, no {issuer}."
        )));
    }
    let endpoint = |field: &str, what: &str| -> Result<String> {
        let url = metadata[field].as_str().ok_or_else(|| {
            Error::Remote(format!(
                "La metadata del servidor de autorización no declara {field}."
            ))
        })?;
        server_url(url, what)
    };
    let authorization = endpoint("authorization_endpoint", "la autorización")?;
    let token = endpoint("token_endpoint", "los tokens")?;
    let s256 = metadata["code_challenge_methods_supported"]
        .as_array()
        .is_some_and(|methods| methods.iter().any(|m| m == "S256"));
    if !s256 {
        return Err(Error::Unsupported(
            "El servidor de autorización no anuncia PKCE S256; kn no inicia sesión sin él.".into(),
        ));
    }
    let registration = match metadata.get("registration_endpoint") {
        Some(Value::String(url)) => Some(server_url(url, "el registro")?),
        _ => None,
    };
    Ok(Server {
        authorization,
        token,
        registration,
    })
}

/// RFC 7591 dynamic registration as a public client.
fn register(http: &Http, server: &Server, redirect_uri: &str) -> Result<String> {
    let url = server.registration.as_deref().ok_or_else(|| {
        Error::Unsupported(
            "El servidor exige registrar kn manualmente; kn no lo presenta como un inicio de sesión listo."
                .into(),
        )
    })?;
    let body = serde_json::to_vec(
        &json!({"client_name": "kn", "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
        "token_endpoint_auth_method": "none"}),
    )?;
    let response = http.post(
        url,
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ],
        &body,
    )?;
    match response.status {
        200 | 201 => {}
        status @ 400..=499 => {
            let error = oauth_error(response);
            return Err(Error::Remote(format!(
                "El servidor rechazó el registro de kn (HTTP {status}, {error})."
            )));
        }
        status => return Err(status_error("El registro de clientes", status)),
    }
    let body = json_object(response, "el registro de clientes")?;
    body["client_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Remote("El registro de clientes no entregó client_id.".into()))
}

/// POST a token request; `refused` builds the error for HTTP 400/401.
fn token_call(
    http: &Http,
    url: &str,
    params: &[(&str, &str)],
    refused: impl FnOnce(String) -> Error,
) -> Result<Value> {
    let response = http.post(
        url,
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Accept", "application/json"),
        ],
        form(params).as_bytes(),
    )?;
    match response.status {
        200 => json_object(response, "la solicitud de tokens"),
        400 | 401 => Err(refused(oauth_error(response))),
        status => Err(status_error("El servidor de tokens", status)),
    }
}

fn parse_tokens(
    body: &Value,
    previous_refresh: Option<&str>,
    token_endpoint: String,
    client_id: String,
    resource: String,
) -> Result<Tokens> {
    let access_token = body["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| Error::Remote("La respuesta de tokens no incluye access_token.".into()))?;
    if !body["token_type"]
        .as_str()
        .is_some_and(|t| t.eq_ignore_ascii_case("bearer"))
    {
        return Err(Error::Remote(
            "El servidor de autorización entregó un token que no es Bearer.".into(),
        ));
    }
    let expires_in = match &body["expires_in"] {
        Value::Null => None,
        value => Some(
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| {
                    Error::Remote("La respuesta de tokens trae expires_in inválido.".into())
                })?,
        ),
    };
    let refresh_token = body["refresh_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .or(previous_refresh)
        .map(str::to_string);
    Ok(Tokens {
        access_token: access_token.into(),
        refresh_token,
        expires_at: expires_in.map(|secs| now().saturating_add(secs)),
        token_endpoint,
        client_id,
        resource,
    })
}

/// Serve the loopback redirect until the matching callback arrives or time runs out.
/// Connections are polled without blocking, so an idle browser preconnect never stalls it.
fn wait_callback(listener: &TcpListener, state: &str, timeout: Duration) -> Result<String> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let mut pending: Vec<(TcpStream, Vec<u8>)> = vec![];
    loop {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    pending.push((stream, vec![]));
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::Interrupted | ErrorKind::ConnectionAborted
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let mut i = 0;
        while i < pending.len() {
            let (stream, buf) = &mut pending[i];
            let mut chunk = [0u8; 2048];
            match stream.read(&mut chunk) {
                Ok(0) => {
                    pending.swap_remove(i);
                }
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n")
                        || buf.windows(2).any(|w| w == b"\n\n")
                    {
                        let (stream, buf) = pending.swap_remove(i);
                        if let Some(outcome) = answer(stream, &buf, state) {
                            return outcome;
                        }
                    } else if buf.len() > 16 * 1024 {
                        pending.swap_remove(i);
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
                    i += 1
                }
                Err(_) => {
                    // A broken browser connection does not decide the login.
                    pending.swap_remove(i);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::AuthRequired(
                "No se recibió la autorización a tiempo.".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Answer one request; None means keep waiting for the real callback.
fn answer(stream: TcpStream, request: &[u8], state: &str) -> Option<Result<String>> {
    let line = String::from_utf8_lossy(request);
    let mut parts = line.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        reply(stream, "404 Not Found", "kn: no encontrado.");
        return None;
    }
    if method != "GET" {
        reply(stream, "405 Method Not Allowed", "kn: método no admitido.");
        return None;
    }
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
            .find(|(k, _)| decode(k).as_deref() == Some(name))
            .and_then(|(_, v)| decode(v))
    };
    if let Some(error) = param("error") {
        reply(
            stream,
            "200 OK",
            "kn: la autorización fue rechazada; puedes cerrar esta ventana.",
        );
        return Some(Err(Error::AuthRequired(format!(
            "La autorización fue rechazada: {}",
            printable(&error)
        ))));
    }
    if param("state").as_deref() != Some(state) {
        reply(
            stream,
            "400 Bad Request",
            "kn: la respuesta no corresponde a esta solicitud.",
        );
        return Some(Err(Error::AuthRequired(
            "La respuesta de autorización no corresponde a esta solicitud (state distinto); vuelve a intentar."
                .into(),
        )));
    }
    let Some(code) = param("code").filter(|c| !c.is_empty()) else {
        reply(
            stream,
            "400 Bad Request",
            "kn: falta el código de autorización.",
        );
        return Some(Err(Error::AuthRequired(
            "La respuesta de autorización no incluyó un código.".into(),
        )));
    };
    reply(
        stream,
        "200 OK",
        "kn: autorización completada; puedes cerrar esta ventana.",
    );
    Some(Ok(code))
}

fn reply(mut stream: TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><html lang=\"es\"><meta charset=\"utf-8\"><title>kn</title><p>{text}</p></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let written = stream
        .set_nonblocking(false)
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(2))))
        .and_then(|()| stream.write_all(response.as_bytes()))
        .and_then(|()| stream.flush());
    // The page is a courtesy to the browser; the outcome is already decided.
    drop(written);
}

/// Validate a URL learned from a server before kn uses it.
fn server_url(url: &str, what: &str) -> Result<String> {
    validate_url(url).map(|_| url.to_string()).map_err(|_| {
        Error::Remote(format!(
            "El servidor anunció una URL no admitida para {what} ({url}); se exige https, o http en loopback."
        ))
    })
}

/// GET a JSON object; Ok(None) when the server does not publish it there.
fn get_json(http: &Http, url: &str) -> Result<Option<Value>> {
    let response = http.get(url, &[("Accept", "application/json")])?;
    match response.status {
        200 => json_object(response, url).map(Some),
        status @ 500..=599 => Err(status_error(url, status)),
        _ => Ok(None),
    }
}
fn first_json(http: &Http, urls: &[String]) -> Result<Option<Value>> {
    for url in urls {
        if let Some(value) = get_json(http, url)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
fn json_object(response: Response, what: &str) -> Result<Value> {
    serde_json::from_slice::<Value>(&response.bytes()?)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| Error::Remote(format!("Respuesta JSON ilegible de {what}.")))
}
/// The OAuth `error` code of a refusal, reduced to printable ASCII.
fn oauth_error(response: Response) -> String {
    let body = response.bytes().unwrap_or_default();
    serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["error"].as_str().map(printable))
        .unwrap_or_else(|| "sin detalle".into())
}
fn status_error(what: &str, status: u16) -> Error {
    match status {
        300..=399 => Error::Remote(format!(
            "{what} respondió con una redirección; kn no la sigue para no reenviar credenciales."
        )),
        500..=599 => Error::RemoteUnavailable(format!("{what} respondió HTTP {status}.")),
        _ => Error::Remote(format!("{what} respondió HTTP {status}.")),
    }
}
fn printable(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(100)
        .collect()
}

fn origin(uri: &ureq::http::Uri) -> String {
    let scheme = uri.scheme_str().unwrap_or("https");
    let authority = uri.authority().map_or("", |a| a.as_str());
    format!("{scheme}://{authority}")
}
/// Lowercase scheme and host, default port removed, no trailing slash.
fn canonical(url: &str) -> String {
    let Ok(uri) = url.parse::<ureq::http::Uri>() else {
        return url.to_string();
    };
    let scheme = uri.scheme_str().unwrap_or("").to_ascii_lowercase();
    let host = uri.host().unwrap_or("").to_ascii_lowercase();
    let port = match (uri.port_u16(), scheme.as_str()) {
        (Some(443), "https") | (Some(80), "http") | (None, _) => String::new(),
        (Some(port), _) => format!(":{port}"),
    };
    let path = uri.path().trim_end_matches('/');
    let query = uri.query().map_or(String::new(), |q| format!("?{q}"));
    format!("{scheme}://{host}{port}{path}{query}")
}

/// RFC 7636 unreserved characters, without modulo bias.
fn random_string(len: usize) -> Result<String> {
    let limit = 256 - 256 % UNRESERVED.len();
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        getrandom::fill(&mut buf).map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "No se pudo generar un valor aleatorio: {e}"
            )))
        })?;
        for b in buf.iter().map(|b| usize::from(*b)).filter(|b| *b < limit) {
            if out.len() < len {
                out.push(char::from(UNRESERVED[b % UNRESERVED.len()]));
            }
        }
    }
    Ok(out)
}
fn form(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::{canonical, challenge_param, decode, encode, random_string};

    #[test]
    fn challenge_param_reads_bearer_params_among_several_challenges() {
        let header = r#"Basic realm="x", Bearer error="invalid_token", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource/mcp", scope="files.read files.write""#;
        assert_eq!(
            challenge_param(header, "resource_metadata").as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource/mcp")
        );
        assert_eq!(
            challenge_param(header, "scope").as_deref(),
            Some("files.read files.write")
        );
        assert_eq!(challenge_param(header, "realm"), None);
        assert_eq!(
            challenge_param("bearer Scope=files , realm = \"a \\\"b\\\"\"", "scope").as_deref(),
            Some("files")
        );
        assert_eq!(
            challenge_param("Bearer realm = \"a \\\"b\\\"\"", "REALM").as_deref(),
            Some("a \"b\"")
        );
        assert_eq!(
            challenge_param("Negotiate abc==, Bearer scope=\"s\"", "scope").as_deref(),
            Some("s")
        );
        assert_eq!(challenge_param("Basic scope=\"s\"", "scope"), None);
        assert_eq!(challenge_param("Bearer", "scope"), None);
        assert_eq!(challenge_param("", "scope"), None);
        assert_eq!(
            challenge_param("Bearer scope=\"unterminated", "scope").as_deref(),
            Some("unterminated")
        );
    }

    #[test]
    fn canonical_forms_ignore_case_default_port_and_trailing_slash() {
        assert_eq!(
            canonical("HTTPS://MCP.Example.com:443/mcp/"),
            "https://mcp.example.com/mcp"
        );
        assert_eq!(
            canonical("http://127.0.0.1:8080/mcp"),
            "http://127.0.0.1:8080/mcp"
        );
        assert_ne!(
            canonical("https://mcp.example.com/mcp"),
            canonical("https://mcp.example.com/other")
        );
        assert_ne!(
            canonical("https://mcp.example.com:8443/mcp"),
            canonical("https://mcp.example.com/mcp")
        );
    }

    #[test]
    fn percent_encoding_round_trips_and_verifiers_use_unreserved_characters() {
        let text = "a b&c=d/é~";
        assert_eq!(encode(text), "a%20b%26c%3Dd%2F%C3%A9~");
        assert_eq!(decode(&encode(text)).as_deref(), Some(text));
        assert_eq!(decode("a+b%2").as_deref(), None);
        let verifier = random_string(64).unwrap();
        assert_eq!(verifier.len(), 64);
        assert!(
            verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        );
        assert_ne!(verifier, random_string(64).unwrap());
    }
}
