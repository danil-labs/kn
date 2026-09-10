//! OAuth login of kn against a fake protected MCP server and authorization server.
use base64::Engine as _;
use kn_core::{
    error::{Error, Result},
    remote::{credentials::Tokens, http::Http, now, oauth},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
use tiny_http::{Header, Method, Request, Response, Server};

const CLIENT_ID: &str = "kn-client-1";
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq)]
enum Authorize {
    Approve,
    Deny,
    TamperState,
}
#[derive(Clone)]
struct Config {
    open: bool,
    s256: bool,
    registration: bool,
    rotate: bool,
    bare_challenge: bool,
    metadata_url: Option<String>,
    prm_resource: Option<String>,
    authorize: Authorize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            open: false,
            s256: true,
            registration: true,
            rotate: false,
            bare_challenge: false,
            metadata_url: None,
            prm_resource: None,
            authorize: Authorize::Approve,
        }
    }
}
struct Grant {
    code: String,
    challenge: String,
    redirect_uri: String,
    client_id: String,
    resource: String,
}
#[derive(Default)]
struct Seen {
    registration: Option<Value>,
    authorize: HashMap<String, String>,
    token_forms: Vec<HashMap<String, String>>,
    grant: Option<Grant>,
    refresh: Option<String>,
}

struct Fake {
    base: String,
    server: Arc<Server>,
    seen: Arc<Mutex<Seen>>,
}
impl Fake {
    fn start(config: Config) -> Self {
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (srv, b, s) = (server.clone(), base.clone(), seen.clone());
        thread::spawn(move || {
            for request in srv.incoming_requests() {
                handle(request, &b, &config, &s);
            }
        });
        Self { base, server, seen }
    }
    fn endpoint(&self) -> String {
        format!("{}/mcp", self.base)
    }
    fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen.lock().unwrap()
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

enum Reply {
    Json(u16, Value),
    Challenge(String),
    Redirect(String),
    Status(u16),
}
fn handle(mut request: Request, base: &str, config: &Config, seen: &Mutex<Seen>) {
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body).unwrap();
    let form_type = request.headers().iter().any(|h| {
        h.field.equiv("Content-Type") && h.value.as_str() == "application/x-www-form-urlencoded"
    });
    let mut seen = seen.lock().unwrap();
    let endpoint = format!("{base}/mcp");
    let reply = match (request.method(), path) {
        (Method::Post, "/mcp") if config.open => Reply::Json(
            200,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": "2025-11-25"}}),
        ),
        (Method::Post, "/mcp") => {
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap()["method"],
                "initialize"
            );
            let metadata = config
                .metadata_url
                .clone()
                .unwrap_or(format!("{base}/.well-known/oauth-protected-resource/mcp"));
            Reply::Challenge(if config.bare_challenge {
                "Bearer".into()
            } else {
                format!(
                    r#"Basic realm="fake", Bearer resource_metadata="{metadata}", scope="files""#
                )
            })
        }
        (Method::Get, "/.well-known/oauth-protected-resource/mcp") => Reply::Json(
            200,
            json!({"resource": config.prm_resource.clone().unwrap_or(endpoint),
                "authorization_servers": [base], "scopes_supported": ["files", "profile"]}),
        ),
        (Method::Get, "/.well-known/oauth-authorization-server") => {
            let mut metadata = json!({"issuer": base,
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": if config.s256 { json!(["plain", "S256"]) } else { json!(["plain"]) }});
            if config.registration {
                metadata["registration_endpoint"] = json!(format!("{base}/register"));
            }
            Reply::Json(200, metadata)
        }
        (Method::Post, "/register") => {
            let registration: Value = serde_json::from_str(&body).unwrap();
            seen.registration = Some(registration);
            Reply::Json(
                201,
                json!({"client_id": CLIENT_ID, "token_endpoint_auth_method": "none"}),
            )
        }
        (Method::Get, "/authorize") => {
            let q = params(query);
            let registered = seen.registration.as_ref().unwrap()["redirect_uris"][0].clone();
            let redirect_uri = q["redirect_uri"].clone();
            if q["client_id"] != CLIENT_ID
                || registered != redirect_uri.as_str()
                || q["response_type"] != "code"
                || q["code_challenge_method"] != "S256"
            {
                Reply::Status(400)
            } else {
                let state = match config.authorize {
                    Authorize::TamperState => "tampered".to_string(),
                    _ => q["state"].clone(),
                };
                seen.grant = Some(Grant {
                    code: "code-1".into(),
                    challenge: q["code_challenge"].clone(),
                    redirect_uri: redirect_uri.clone(),
                    client_id: q["client_id"].clone(),
                    resource: q["resource"].clone(),
                });
                seen.authorize = q;
                Reply::Redirect(match config.authorize {
                    Authorize::Deny => format!("{redirect_uri}?error=access_denied&state={state}"),
                    _ => format!("{redirect_uri}?code=code-1&state={state}"),
                })
            }
        }
        (Method::Post, "/token") => {
            assert!(form_type, "la solicitud de tokens debe ser form-urlencoded");
            let f = params(&body);
            seen.token_forms.push(f.clone());
            match f["grant_type"].as_str() {
                "authorization_code" => match seen.grant.take() {
                    Some(g)
                        if f["code"] == g.code
                            && pkce(&f["code_verifier"]) == g.challenge
                            && f["redirect_uri"] == g.redirect_uri
                            && f["client_id"] == g.client_id
                            && f["resource"] == g.resource =>
                    {
                        seen.refresh = Some("rt-1".into());
                        Reply::Json(
                            200,
                            json!({"access_token": "at-1", "token_type": "bearer",
                                "expires_in": 3600, "refresh_token": "rt-1"}),
                        )
                    }
                    _ => Reply::Json(400, json!({"error": "invalid_grant"})),
                },
                "refresh_token"
                    if seen.refresh.as_deref() == Some(f["refresh_token"].as_str())
                        && f["client_id"] == CLIENT_ID
                        && f["resource"] == endpoint =>
                {
                    let mut tokens =
                        json!({"access_token": "at-2", "token_type": "Bearer", "expires_in": 60});
                    if config.rotate {
                        tokens["refresh_token"] = json!("rt-2");
                        seen.refresh = Some("rt-2".into());
                    }
                    Reply::Json(200, tokens)
                }
                _ => Reply::Json(400, json!({"error": "invalid_grant"})),
            }
        }
        _ => Reply::Status(404),
    };
    drop(seen);
    let header = |k: &str, v: &str| Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap();
    let result = match reply {
        Reply::Json(status, value) => request.respond(
            Response::from_string(value.to_string())
                .with_status_code(status)
                .with_header(header("Content-Type", "application/json")),
        ),
        Reply::Challenge(value) => request.respond(
            Response::from_string("")
                .with_status_code(401)
                .with_header(header("WWW-Authenticate", &value)),
        ),
        Reply::Redirect(location) => request.respond(
            Response::from_string("")
                .with_status_code(302)
                .with_header(header("Location", &location)),
        ),
        Reply::Status(status) => request.respond(Response::empty(status)),
    };
    result.unwrap();
}

fn pkce(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn params(s: &str) -> HashMap<String, String> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

/// Browser pages seen by the fake browser: (status, body).
type Visits = mpsc::Receiver<Vec<(u16, String)>>;

/// Acts as the browser in another thread: `login` only listens after this returns.
fn browser() -> (impl Fn(&str) -> Result<()>, Visits) {
    let (tx, rx) = mpsc::channel();
    let open = move |url: &str| {
        let (url, tx) = (url.to_string(), tx.clone());
        thread::spawn(move || {
            let http = Http::new();
            let authorize = http.get(&url, &[]).unwrap();
            assert_eq!(
                authorize.status, 302,
                "el servidor rechazó la solicitud de autorización"
            );
            let location = authorize.header("location").unwrap().to_string();
            let origin = location.split("/callback").next().unwrap().to_string();
            let mut visits = vec![];
            for target in [format!("{origin}/favicon.ico"), location] {
                let response = http.get(&target, &[]).unwrap();
                let status = response.status;
                visits.push((
                    status,
                    String::from_utf8(response.bytes().unwrap()).unwrap(),
                ));
            }
            tx.send(visits).unwrap();
        });
        Ok(())
    };
    (open, rx)
}
fn never_opened(_: &str) -> Result<()> {
    panic!("no debe abrirse el navegador");
}
fn fails<T>(result: Result<T>) -> Error {
    match result {
        Ok(_) => panic!("se esperaba un error"),
        Err(e) => e,
    }
}

#[test]
fn login_verifies_pkce_sends_resource_and_forwards_scope() {
    let fake = Fake::start(Config::default());
    let (open, visits) = browser();
    let tokens = oauth::login(&Http::new(), &fake.endpoint(), &open, TIMEOUT).unwrap();
    assert_eq!(tokens.access_token, "at-1");
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt-1"));
    assert_eq!(tokens.client_id, CLIENT_ID);
    assert_eq!(tokens.token_endpoint, format!("{}/token", fake.base));
    assert_eq!(tokens.resource, fake.endpoint());
    let expires_at = tokens.expires_at.unwrap();
    assert!((now() + 3590..=now() + 3600).contains(&expires_at));

    let visits = visits.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(
        visits[0].0, 404,
        "otras rutas responden 404 y la espera sigue"
    );
    assert_eq!(visits[1].0, 200);
    assert!(
        visits[1]
            .1
            .contains("kn: autorización completada; puedes cerrar esta ventana.")
    );

    let seen = fake.seen();
    let registration = seen.registration.as_ref().unwrap();
    assert_eq!(registration["client_name"], "kn");
    assert_eq!(registration["token_endpoint_auth_method"], "none");
    assert_eq!(
        registration["grant_types"],
        json!(["authorization_code", "refresh_token"])
    );
    let redirect = registration["redirect_uris"][0].as_str().unwrap();
    assert!(redirect.starts_with("http://127.0.0.1:") && redirect.ends_with("/callback"));
    assert_eq!(seen.authorize["scope"], "files");
    assert_eq!(seen.authorize["resource"], fake.endpoint());
    assert_eq!(seen.authorize["state"].len(), 32);
    let form = &seen.token_forms[0];
    assert_eq!(form["resource"], fake.endpoint());
    let verifier = &form["code_verifier"];
    assert_eq!(verifier.len(), 64);
    assert!(
        verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
    );
}

#[test]
fn login_discovers_well_known_metadata_and_supported_scopes_without_challenge_params() {
    let fake = Fake::start(Config {
        bare_challenge: true,
        ..Config::default()
    });
    let (open, _visits) = browser();
    let tokens = oauth::login(&Http::new(), &fake.endpoint(), &open, TIMEOUT).unwrap();
    assert_eq!(tokens.access_token, "at-1");
    assert_eq!(fake.seen().authorize["scope"], "files profile");
}

#[test]
fn server_without_authorization_needs_no_login() {
    let fake = Fake::start(Config {
        open: true,
        ..Config::default()
    });
    let err = fails(oauth::login(
        &Http::new(),
        &fake.endpoint(),
        &never_opened,
        TIMEOUT,
    ));
    assert_eq!(err.code(), "INVALID_INPUT");
}

#[test]
fn authorization_server_without_s256_is_refused() {
    let fake = Fake::start(Config {
        s256: false,
        ..Config::default()
    });
    let err = fails(oauth::login(
        &Http::new(),
        &fake.endpoint(),
        &never_opened,
        TIMEOUT,
    ));
    assert_eq!(err.code(), "UNSUPPORTED_CAPABILITY");
    assert!(err.to_string().contains("S256"));
}

#[test]
fn server_without_registration_endpoint_is_unsupported() {
    let fake = Fake::start(Config {
        registration: false,
        ..Config::default()
    });
    let err = fails(oauth::login(
        &Http::new(),
        &fake.endpoint(),
        &never_opened,
        TIMEOUT,
    ));
    assert_eq!(err.code(), "UNSUPPORTED_CAPABILITY");
    assert!(err.to_string().contains("registrar kn manualmente"));
}

#[test]
fn tampered_state_requires_login_and_never_exchanges_the_code() {
    let fake = Fake::start(Config {
        authorize: Authorize::TamperState,
        ..Config::default()
    });
    let (open, visits) = browser();
    let err = fails(oauth::login(&Http::new(), &fake.endpoint(), &open, TIMEOUT));
    assert_eq!(err.code(), "AUTH_REQUIRED");
    assert_eq!(visits.recv_timeout(TIMEOUT).unwrap()[1].0, 400);
    assert!(fake.seen().token_forms.is_empty());
}

#[test]
fn denied_authorization_requires_login() {
    let fake = Fake::start(Config {
        authorize: Authorize::Deny,
        ..Config::default()
    });
    let (open, visits) = browser();
    let err = fails(oauth::login(&Http::new(), &fake.endpoint(), &open, TIMEOUT));
    assert_eq!(err.code(), "AUTH_REQUIRED");
    assert!(err.to_string().contains("access_denied"));
    assert_eq!(visits.recv_timeout(TIMEOUT).unwrap()[1].0, 200);
    assert!(fake.seen().token_forms.is_empty());
}

#[test]
fn missing_callback_times_out() {
    let fake = Fake::start(Config::default());
    let ignore = |_: &str| Ok(());
    let err = fails(oauth::login(
        &Http::new(),
        &fake.endpoint(),
        &ignore,
        Duration::from_millis(300),
    ));
    assert_eq!(err.code(), "AUTH_REQUIRED");
    assert!(err.to_string().contains("a tiempo"));
}

#[test]
fn metadata_for_another_resource_is_rejected() {
    let fake = Fake::start(Config {
        prm_resource: Some("https://other.example.com/mcp".into()),
        ..Config::default()
    });
    let err = fails(oauth::login(
        &Http::new(),
        &fake.endpoint(),
        &never_opened,
        TIMEOUT,
    ));
    assert_eq!(err.code(), "REMOTE_ERROR");
    assert!(err.to_string().contains("https://other.example.com/mcp"));
}

#[test]
fn plain_http_metadata_url_off_loopback_is_rejected() {
    let fake = Fake::start(Config {
        metadata_url: Some(
            "http://metadata.example.com/.well-known/oauth-protected-resource".into(),
        ),
        ..Config::default()
    });
    let err = fails(oauth::login(
        &Http::new(),
        &fake.endpoint(),
        &never_opened,
        TIMEOUT,
    ));
    assert_eq!(err.code(), "REMOTE_ERROR");
    assert!(err.to_string().contains("https"));
}

#[test]
fn refresh_keeps_the_refresh_token_when_not_rotated() {
    let fake = Fake::start(Config::default());
    let (open, _visits) = browser();
    let http = Http::new();
    let tokens = oauth::login(&http, &fake.endpoint(), &open, TIMEOUT).unwrap();
    let renewed = oauth::refresh(&http, &tokens).unwrap();
    assert_eq!(renewed.access_token, "at-2");
    assert_eq!(renewed.refresh_token.as_deref(), Some("rt-1"));
    assert_eq!(renewed.resource, fake.endpoint());
    assert!(renewed.expires_at.unwrap() <= now() + 60);
    let seen = fake.seen();
    let form = seen.token_forms.last().unwrap();
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["resource"], fake.endpoint());
}

#[test]
fn refresh_adopts_a_rotated_refresh_token() {
    let fake = Fake::start(Config {
        rotate: true,
        ..Config::default()
    });
    let (open, _visits) = browser();
    let http = Http::new();
    let tokens = oauth::login(&http, &fake.endpoint(), &open, TIMEOUT).unwrap();
    let renewed = oauth::refresh(&http, &tokens).unwrap();
    assert_eq!(renewed.refresh_token.as_deref(), Some("rt-2"));
    let err = fails(oauth::refresh(&http, &tokens));
    assert_eq!(err.code(), "AUTH_REQUIRED", "el token anterior ya no sirve");
    assert!(!err.to_string().contains("rt-1"));
}

#[test]
fn refresh_without_refresh_token_requires_login() {
    let tokens = Tokens {
        access_token: "at".into(),
        refresh_token: None,
        expires_at: Some(0),
        token_endpoint: "https://auth.invalid/token".into(),
        client_id: CLIENT_ID.into(),
        resource: "https://mcp.invalid/mcp".into(),
    };
    let err = fails(oauth::refresh(&Http::new(), &tokens));
    assert_eq!(err.code(), "AUTH_REQUIRED");
    assert!(err.to_string().contains("kn remote login"));
}
