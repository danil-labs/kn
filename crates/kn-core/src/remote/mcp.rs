//! Minimal MCP client over Streamable HTTP: initialize, tools/list and tools/call.
//! kn declares no client capabilities, so servers have no reason to send it
//! sampling or elicitation requests; any such message is ignored.
use super::http::{Http, Response, unavailable};
use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};

pub const PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

pub struct Client {
    http: Http,
    endpoint: String,
    token: Option<String>,
    session: Option<String>,
    protocol: String,
    next_id: u64,
    pub server: Value,
}
pub struct ToolResult {
    pub is_error: bool,
    pub structured: Option<Value>,
    pub text: String,
}

impl Client {
    /// Initialize a session; nothing is read from or written to documents.
    pub fn connect(http: Http, endpoint: &str, token: Option<String>) -> Result<Self> {
        let mut client = Self {
            http,
            endpoint: endpoint.into(),
            token,
            session: None,
            protocol: PROTOCOL_VERSION.into(),
            next_id: 1,
            server: Value::Null,
        };
        let result = client.request(
            "initialize",
            json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                "clientInfo": {"name": "kn", "version": env!("CARGO_PKG_VERSION")}}),
        )?;
        let version = result["protocolVersion"].as_str().unwrap_or_default();
        if !SUPPORTED.contains(&version) {
            return Err(Error::Remote(format!(
                "El servidor propone una versión MCP no admitida: {version:?}."
            )));
        }
        client.protocol = version.into();
        if result["capabilities"].get("tools").is_none() {
            return Err(Error::Remote(
                "El servidor no ofrece herramientas MCP.".into(),
            ));
        }
        client.server = result.get("serverInfo").cloned().unwrap_or(Value::Null);
        client.notify("notifications/initialized")?;
        Ok(client)
    }
    pub fn protocol(&self) -> &str {
        &self.protocol
    }
    pub fn list_tools(&mut self) -> Result<Vec<Value>> {
        let mut tools = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..1000 {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let result = self.request("tools/list", params)?;
            tools.extend(
                result["tools"]
                    .as_array()
                    .ok_or_else(|| malformed("tools/list"))?
                    .iter()
                    .cloned(),
            );
            match result.get("nextCursor").and_then(Value::as_str) {
                Some(c) if !c.is_empty() => cursor = Some(c.into()),
                _ => return Ok(tools),
            }
        }
        Err(Error::Remote("La lista de herramientas no termina.".into()))
    }
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<ToolResult> {
        let result = self.request("tools/call", json!({"name": name, "arguments": arguments}))?;
        let text = result["content"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i["type"] == "text")
                    .filter_map(|i| i["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();
        Ok(ToolResult {
            is_error: result["isError"] == true,
            structured: result
                .get("structuredContent")
                .filter(|v| !v.is_null())
                .cloned(),
            text,
        })
    }
    fn post(&self, message: &Value) -> Result<Response> {
        let body = serde_json::to_vec(message)?;
        let mut headers: Vec<(&str, String)> = vec![
            ("Content-Type", "application/json".into()),
            ("Accept", "application/json, text/event-stream".into()),
        ];
        if let Some(token) = &self.token {
            headers.push(("Authorization", format!("Bearer {token}")));
        }
        if let Some(session) = &self.session {
            headers.push(("Mcp-Session-Id", session.clone()));
        }
        if message["method"] != "initialize" {
            headers.push(("MCP-Protocol-Version", self.protocol.clone()));
        }
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let response = self.http.post(&self.endpoint, &headers, &body)?;
        match response.status {
            200..=299 => Ok(response),
            401 => Err(Error::AuthRequired(
                "El servidor MCP requiere autorización: ejecuta kn remote login o entrega KN_MCP_ACCESS_TOKEN."
                    .into(),
            )),
            403 => Err(Error::AuthRequired(
                "El servidor MCP rechazó los permisos de esta cuenta (403).".into(),
            )),
            404 if self.session.is_some() => Err(Error::RemoteUnavailable(
                "La sesión MCP expiró; vuelve a intentar.".into(),
            )),
            300..=399 => Err(Error::Remote(
                "El servidor respondió con una redirección; kn no la sigue para no reenviar credenciales."
                    .into(),
            )),
            status @ 500..=599 => Err(Error::RemoteUnavailable(format!(
                "El servidor MCP respondió HTTP {status}."
            ))),
            status => Err(Error::Remote(format!(
                "El servidor MCP respondió HTTP {status}."
            ))),
        }
    }
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let response =
            self.post(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        if method == "initialize"
            && let Some(session) = response.header("mcp-session-id")
        {
            if session.is_empty() || !session.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
                return Err(malformed("initialize"));
            }
            self.session = Some(session.into());
        }
        let message = if response.content_type().starts_with("text/event-stream") {
            read_sse(response, id)?
        } else {
            let message: Value =
                serde_json::from_slice(&response.bytes()?).map_err(|_| malformed(method))?;
            if message["id"] != id {
                return Err(malformed(method));
            }
            message
        };
        if let Some(error) = message.get("error") {
            let detail = error["message"].as_str().unwrap_or("sin detalle");
            // Protocol errors mean the request was not executed; any other error can
            // arrive after the server acted, so its outcome is unknown.
            return Err(match error["code"].as_i64() {
                Some(-32700 | -32600 | -32601 | -32602) => {
                    Error::Remote(format!("El servidor MCP rechazó {method}: {detail}"))
                }
                _ => Error::RemoteUnavailable(format!(
                    "El servidor MCP falló en {method} con resultado desconocido: {detail}"
                )),
            });
        }
        message
            .get("result")
            .cloned()
            .ok_or_else(|| malformed(method))
    }
    fn notify(&mut self, method: &str) -> Result<()> {
        // 202 Accepted without a body; the response carries nothing to read.
        self.post(&json!({"jsonrpc": "2.0", "method": method}))
            .map(drop)
    }
}
/// A response kn cannot interpret: treated like a lost response, never as success.
fn malformed(method: &str) -> Error {
    Error::RemoteUnavailable(format!(
        "El servidor MCP devolvió una respuesta ilegible para {method}."
    ))
}
/// Read server-sent events until the response to `id`; other messages are ignored.
fn read_sse(response: Response, id: u64) -> Result<Value> {
    let mut data = String::new();
    for line in BufReader::new(response.reader()).lines() {
        let line = line.map_err(unavailable)?;
        if line.is_empty() {
            if !data.is_empty() {
                if let Ok(message) = serde_json::from_str::<Value>(&data)
                    && message["id"] == id
                    && (message.get("result").is_some() || message.get("error").is_some())
                {
                    return Ok(message);
                }
                data.clear();
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    Err(Error::RemoteUnavailable(
        "El flujo del servidor MCP terminó sin respuesta.".into(),
    ))
}
