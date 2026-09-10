//! Profile-driven document operations over an MCP client.
use super::{
    http::Http,
    mcp::{Client, ToolResult},
    profile::{ContentEncoding, OPERATIONS, Profile, ResultSource, schema_hash, text},
};
use crate::error::{Error, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    File,
    Folder,
    RemoteOnly,
}
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub kind: ItemKind,
    pub revision: Option<String>,
}
pub struct Page {
    pub items: Vec<Item>,
    pub next_cursor: Option<String>,
}
pub struct FileBytes {
    pub bytes: Vec<u8>,
    pub revision: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    PreconditionFailed,
    AlreadyExists,
    NotFound,
    Other,
}
pub enum WriteOutcome {
    Applied {
        id: Option<String>,
        revision: Option<String>,
    },
    /// The server answered and did not apply the change.
    Rejected { reason: Rejection, message: String },
    /// The request may have reached the server; only a new observation can tell.
    Unconfirmed(String),
}
#[derive(Clone, Debug, Serialize)]
pub struct Capability {
    pub operation: &'static str,
    pub tool: Option<String>,
    pub available: bool,
    pub reason: Option<String>,
    pub expected_schema_sha256: Option<String>,
    pub actual_schema_sha256: Option<String>,
}

pub struct Remote {
    client: Client,
    profile: Profile,
    capabilities: Vec<Capability>,
    allow_writes: bool,
}

impl Remote {
    /// Connect, list tools and compare each pinned schema. Writes are refused
    /// before contacting the server unless the transfer mode allows them.
    pub fn open(
        http: Http,
        endpoint: &str,
        token: Option<String>,
        profile: Profile,
        allow_writes: bool,
    ) -> Result<Self> {
        let mut client = Client::connect(http, endpoint, token)?;
        let tools = client.list_tools()?;
        let by_name: BTreeMap<&str, &Value> = tools
            .iter()
            .filter_map(|t| Some((t["name"].as_str()?, t)))
            .collect();
        let capabilities = OPERATIONS
            .iter()
            .map(|op| {
                let Some(spec) = profile.operations.get(*op) else {
                    return Capability {
                        operation: op,
                        tool: None,
                        available: false,
                        reason: Some("El perfil no define esta operación.".into()),
                        expected_schema_sha256: None,
                        actual_schema_sha256: None,
                    };
                };
                let actual = by_name.get(spec.tool.as_str()).map(|t| schema_hash(t));
                let reason = match &actual {
                    None => Some(format!("El servidor no ofrece la herramienta {}.", spec.tool)),
                    Some(hash) if *hash != spec.schema_sha256 => Some(
                        "El esquema de la herramienta cambió; actualiza y prueba el perfil.".into(),
                    ),
                    Some(_) => profile.write_safety(op).err(),
                };
                Capability {
                    operation: op,
                    tool: Some(spec.tool.clone()),
                    available: reason.is_none(),
                    reason,
                    expected_schema_sha256: Some(spec.schema_sha256.clone()),
                    actual_schema_sha256: actual,
                }
            })
            .collect();
        Ok(Self {
            client,
            profile,
            capabilities,
            allow_writes,
        })
    }
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }
    pub fn server(&self) -> &Value {
        &self.client.server
    }
    pub fn protocol(&self) -> &str {
        self.client.protocol()
    }
    pub fn content_encoding(&self) -> ContentEncoding {
        self.profile.content_encoding
    }
    pub fn require(&self, op: &str) -> Result<()> {
        let Some(c) = self.capabilities.iter().find(|c| c.operation == op) else {
            return Err(Error::Unsupported(format!("Operación remota desconocida: {op}.")));
        };
        if c.available {
            return Ok(());
        }
        let reason = c.reason.clone().unwrap_or_default();
        if c.actual_schema_sha256.is_some() && c.actual_schema_sha256 != c.expected_schema_sha256 {
            return Err(Error::ProfileMismatch(format!("{op}: {reason}")));
        }
        Err(Error::Unsupported(format!(
            "Operación remota no disponible ({op}): {reason}"
        )))
    }
    fn payload(&self, result: &ToolResult) -> Option<Value> {
        match self.profile.result_source {
            ResultSource::Structured => result.structured.clone(),
            ResultSource::TextJson => serde_json::from_str(&result.text).ok(),
        }
    }
    fn field(&self, op: &str, payload: &Value, field: &str) -> Option<String> {
        let pointer = self.profile.operations.get(op)?.result.get(field)?;
        text(payload.pointer(pointer)?)
    }
    fn read_call(&mut self, op: &str, values: &[(&str, Option<&str>)]) -> Result<Value> {
        self.require(op)?;
        let args = self.profile.render(op, values)?;
        let tool = self.profile.operations[op].tool.clone();
        let result = self.client.call_tool(&tool, args)?;
        if result.is_error {
            return Err(Error::Remote(format!(
                "La herramienta {tool} devolvió un error: {}",
                short(&result.text)
            )));
        }
        self.payload(&result).ok_or_else(|| {
            Error::Remote(format!("La herramienta {tool} no devolvió los datos esperados."))
        })
    }
    pub fn identify(&mut self, id: &str) -> Result<Item> {
        let payload = self.read_call("identify", &[("folder_id", Some(id))])?;
        let kind = self.field("identify", &payload, "kind").unwrap_or_default();
        Ok(Item {
            id: self
                .field("identify", &payload, "id")
                .ok_or_else(|| missing("identify", "id"))?,
            name: self.field("identify", &payload, "name").unwrap_or_default(),
            kind: self.profile.kinds.classify(&kind),
            revision: self.field("identify", &payload, "revision"),
        })
    }
    pub fn list(&mut self, folder_id: &str, cursor: Option<&str>) -> Result<Page> {
        let payload =
            self.read_call("list", &[("folder_id", Some(folder_id)), ("cursor", cursor)])?;
        let spec = &self.profile.operations["list"];
        let items = payload
            .pointer(&spec.result["items"])
            .and_then(Value::as_array)
            .ok_or_else(|| missing("list", "items"))?;
        let item = |v: &Value, f: &str| -> Option<String> {
            text(v.pointer(spec.result.get(&format!("item_{f}"))?)?)
        };
        let items = items
            .iter()
            .map(|v| {
                let kind = self
                    .profile
                    .kinds
                    .classify(&item(v, "kind").unwrap_or_default());
                let revision = item(v, "revision");
                if kind == ItemKind::File && revision.is_none() {
                    return Err(missing("list", "item_revision"));
                }
                Ok(Item {
                    id: item(v, "id").ok_or_else(|| missing("list", "item_id"))?,
                    name: item(v, "name").ok_or_else(|| missing("list", "item_name"))?,
                    kind,
                    revision,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = spec
            .result
            .get("next_cursor")
            .and_then(|p| payload.pointer(p))
            .and_then(text)
            .filter(|c| !c.is_empty());
        Ok(Page { items, next_cursor })
    }
    /// Bytes and the revision the server binds to exactly those bytes.
    pub fn read(&mut self, file_id: &str) -> Result<FileBytes> {
        let payload = self.read_call("read", &[("file_id", Some(file_id))])?;
        let content = self
            .field("read", &payload, "content")
            .ok_or_else(|| missing("read", "content"))?;
        let revision = self
            .field("read", &payload, "revision")
            .ok_or_else(|| missing("read", "revision"))?;
        let bytes = match self.profile.content_encoding {
            ContentEncoding::Base64 => base64::engine::general_purpose::STANDARD
                .decode(content)
                .map_err(|_| Error::Remote("El contenido remoto no es base64 válido.".into()))?,
            ContentEncoding::Utf8 => content.into_bytes(),
        };
        Ok(FileBytes { bytes, revision })
    }
    fn encode(&self, bytes: &[u8]) -> Result<String> {
        match self.profile.content_encoding {
            ContentEncoding::Base64 => Ok(base64::engine::general_purpose::STANDARD.encode(bytes)),
            ContentEncoding::Utf8 => String::from_utf8(bytes.to_vec()).map_err(|_| {
                Error::Unsupported("El perfil solo transmite documentos UTF-8.".into())
            }),
        }
    }
    pub fn update(
        &mut self,
        file_id: &str,
        bytes: &[u8],
        expected_revision: &str,
    ) -> Result<WriteOutcome> {
        let content = self.encode(bytes)?;
        self.write(
            "update",
            &[
                ("file_id", Some(file_id)),
                ("content", Some(&content)),
                ("expected_revision", Some(expected_revision)),
            ],
        )
    }
    pub fn create(&mut self, folder_id: &str, name: &str, bytes: &[u8]) -> Result<WriteOutcome> {
        let content = self.encode(bytes)?;
        self.write(
            "create",
            &[
                ("folder_id", Some(folder_id)),
                ("name", Some(name)),
                ("content", Some(&content)),
            ],
        )
    }
    pub fn create_folder(&mut self, folder_id: &str, name: &str) -> Result<WriteOutcome> {
        self.write(
            "create_folder",
            &[("folder_id", Some(folder_id)), ("name", Some(name))],
        )
    }
    pub fn delete(&mut self, file_id: &str, expected_revision: &str) -> Result<WriteOutcome> {
        self.write(
            "delete",
            &[
                ("file_id", Some(file_id)),
                ("expected_revision", Some(expected_revision)),
            ],
        )
    }
    fn write(&mut self, op: &str, values: &[(&str, Option<&str>)]) -> Result<WriteOutcome> {
        if !self.allow_writes {
            return Err(Error::ModeForbids(
                "El modo de transferencia no permite escrituras remotas mediante MCP.".into(),
            ));
        }
        self.require(op)?;
        let args = self.profile.render(op, values)?;
        let tool = self.profile.operations[op].tool.clone();
        Ok(match self.client.call_tool(&tool, args) {
            Err(Error::RemoteUnavailable(message)) => WriteOutcome::Unconfirmed(message),
            Err(e) => WriteOutcome::Rejected {
                reason: Rejection::Other,
                message: e.to_string(),
            },
            Ok(result) if result.is_error => WriteOutcome::Rejected {
                reason: self.profile.errors.classify(self.payload(&result).as_ref()),
                message: short(&result.text),
            },
            Ok(result) => {
                let payload = self.payload(&result).unwrap_or(Value::Null);
                WriteOutcome::Applied {
                    id: self.field(op, &payload, "id"),
                    revision: self.field(op, &payload, "revision"),
                }
            }
        })
    }
}
fn missing(op: &str, field: &str) -> Error {
    Error::Remote(format!(
        "La respuesta de {op} no trae {field}; revisa el perfil del servidor."
    ))
}
fn short(text: &str) -> String {
    let t: String = text.chars().take(300).collect();
    if t.is_empty() { "sin detalle".into() } else { t }
}
