//! Reviewed mapping between kn's document operations and one server's MCP tools.
//! MCP defines tool discovery and invocation, not folder operations: every tool
//! name, argument and result field is declared here, never inferred from text.
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const OPERATIONS: [&str; 7] = [
    "identify",
    "list",
    "read",
    "update",
    "create",
    "create_folder",
    "delete",
];
const PLACEHOLDERS: [&str; 6] = [
    "folder_id",
    "cursor",
    "file_id",
    "name",
    "content",
    "expected_revision",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub profile_version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub result_source: ResultSource,
    pub content_encoding: ContentEncoding,
    pub operations: BTreeMap<String, Operation>,
    #[serde(default)]
    pub kinds: Kinds,
    #[serde(default)]
    pub errors: ErrorMapping,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResultSource {
    /// `structuredContent` of the tool result.
    Structured,
    /// The concatenated text content, parsed as JSON.
    TextJson,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentEncoding {
    Base64,
    /// Only UTF-8 documents can be read or written through this server.
    Utf8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub tool: String,
    /// SHA-256 of the tool's canonical input/output schemas; see [`schema_hash`].
    pub schema_sha256: String,
    /// JSON template; a string that is exactly `{placeholder}` is substituted.
    pub arguments: Value,
    /// Field name → JSON pointer into the result payload.
    #[serde(default)]
    pub result: BTreeMap<String, String>,
    /// create/create_folder: the server rejects a name that already exists.
    #[serde(default)]
    pub fails_if_exists: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kinds {
    #[serde(default)]
    pub file: Vec<String>,
    #[serde(default)]
    pub folder: Vec<String>,
    /// Documents without byte content kn can version, such as native Google docs.
    #[serde(default)]
    pub remote_only: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorMapping {
    /// JSON pointer to the error code inside an `isError` result payload.
    #[serde(default)]
    pub pointer: String,
    #[serde(default)]
    pub precondition_failed: Vec<String>,
    #[serde(default)]
    pub already_exists: Vec<String>,
    #[serde(default)]
    pub not_found: Vec<String>,
}

/// Required placeholders and result fields per operation.
fn contract(op: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match op {
        "identify" => (&["folder_id"], &["id", "kind"]),
        "list" => (
            &["folder_id"],
            &["items", "item_id", "item_name", "item_kind"],
        ),
        "read" => (&["file_id"], &["content", "revision"]),
        "update" => (&["file_id", "content"], &[]),
        "create" => (&["folder_id", "name", "content"], &[]),
        "create_folder" => (&["folder_id", "name"], &[]),
        "delete" => (&["file_id"], &[]),
        _ => (&[], &[]),
    }
}

impl Profile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let profile: Self = serde_json::from_slice(bytes)
            .map_err(|e| Error::Invalid(format!("Perfil MCP inválido: {e}")))?;
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<()> {
        let invalid = |m: String| Err(Error::Invalid(format!("Perfil MCP inválido: {m}")));
        if self.profile_version != 1 {
            return invalid("profile_version debe ser 1.".into());
        }
        for required in ["list", "read"] {
            if !self.operations.contains_key(required) {
                return invalid(format!("falta la operación {required}."));
            }
        }
        for (op, spec) in &self.operations {
            if !OPERATIONS.contains(&op.as_str()) {
                return invalid(format!("operación desconocida {op}."));
            }
            if spec.tool.is_empty() {
                return invalid(format!("{op} no nombra una herramienta."));
            }
            if spec.schema_sha256.len() != 64
                || !spec
                    .schema_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return invalid(format!("{op}: schema_sha256 debe ser SHA-256 hexadecimal."));
            }
            if !spec.arguments.is_object() {
                return invalid(format!("{op}: arguments debe ser un objeto."));
            }
            let mut used = vec![];
            placeholders(&spec.arguments, &mut used);
            for name in &used {
                if !PLACEHOLDERS.contains(&name.as_str()) {
                    return invalid(format!("{op}: marcador desconocido {{{name}}}."));
                }
            }
            let (args, fields) = contract(op);
            for arg in args {
                if !used.iter().any(|u| u == arg) {
                    return invalid(format!("{op}: falta el marcador {{{arg}}}."));
                }
            }
            for field in fields {
                if !spec.result.contains_key(*field) {
                    return invalid(format!("{op}: falta el campo de resultado {field}."));
                }
            }
            for pointer in spec.result.values() {
                if !pointer.is_empty() && !pointer.starts_with('/') {
                    return invalid(format!("{op}: {pointer} no es un JSON pointer."));
                }
            }
            if spec.result.contains_key("next_cursor") && !used.iter().any(|u| u == "cursor") {
                return invalid(format!("{op}: pagina sin el marcador {{cursor}}."));
            }
        }
        if !self.errors.pointer.is_empty() && !self.errors.pointer.starts_with('/') {
            return invalid("errors.pointer no es un JSON pointer.".into());
        }
        Ok(())
    }
    pub fn uses(&self, op: &str, placeholder: &str) -> bool {
        self.operations.get(op).is_some_and(|spec| {
            let mut used = vec![];
            placeholders(&spec.arguments, &mut used);
            used.iter().any(|u| u == placeholder)
        })
    }
    /// A write is only offered when the server can refuse to overwrite others' work.
    pub fn write_safety(&self, op: &str) -> std::result::Result<(), String> {
        match op {
            "update" | "delete" if !self.uses(op, "expected_revision") => Err(
                "La herramienta no aplica una condición de revisión; kn no escribe sin ella."
                    .into(),
            ),
            "create" | "create_folder"
                if !self.operations.get(op).is_some_and(|s| s.fails_if_exists) =>
            {
                Err("El perfil no declara que el servidor rechace nombres existentes.".into())
            }
            _ => Ok(()),
        }
    }
    /// Substitute `{placeholder}` strings. A missing optional value removes its key.
    pub fn render(&self, op: &str, values: &[(&str, Option<&str>)]) -> Result<Value> {
        let spec = self
            .operations
            .get(op)
            .ok_or_else(|| Error::Unsupported(format!("El perfil no define {op}.")))?;
        Ok(substitute(&spec.arguments, values).unwrap_or(Value::Null))
    }
}
fn placeholder(s: &str) -> Option<&str> {
    s.strip_prefix('{')?.strip_suffix('}')
}
fn placeholders(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            if let Some(name) = placeholder(s) {
                out.push(name.into());
            }
        }
        Value::Array(items) => items.iter().for_each(|i| placeholders(i, out)),
        Value::Object(map) => map.values().for_each(|i| placeholders(i, out)),
        _ => (),
    }
}
/// None means "omit this value" (an absent optional placeholder).
fn substitute(v: &Value, values: &[(&str, Option<&str>)]) -> Option<Value> {
    match v {
        Value::String(s) => match placeholder(s) {
            Some(name) => values
                .iter()
                .find(|(k, _)| *k == name)
                .and_then(|(_, v)| v.map(|v| Value::String(v.into()))),
            None => Some(v.clone()),
        },
        Value::Array(items) => Some(Value::Array(
            items.iter().filter_map(|i| substitute(i, values)).collect(),
        )),
        Value::Object(map) => Some(Value::Object(
            map.iter()
                .filter_map(|(k, i)| Some((k.clone(), substitute(i, values)?)))
                .collect(),
        )),
        _ => Some(v.clone()),
    }
}
impl Kinds {
    pub fn classify(&self, value: &str) -> super::driver::ItemKind {
        use super::driver::ItemKind;
        if self.folder.iter().any(|v| v == value) {
            ItemKind::Folder
        } else if self.remote_only.iter().any(|v| v == value) {
            ItemKind::RemoteOnly
        } else if self.file.is_empty() || self.file.iter().any(|v| v == value) {
            ItemKind::File
        } else {
            // Unknown kinds stay visible but unmanaged; kn never writes over them.
            ItemKind::RemoteOnly
        }
    }
}
impl ErrorMapping {
    pub fn classify(&self, payload: Option<&Value>) -> super::driver::Rejection {
        use super::driver::Rejection;
        let code = payload
            .and_then(|p| p.pointer(&self.pointer))
            .and_then(text);
        match code {
            Some(c) if self.precondition_failed.contains(&c) => Rejection::PreconditionFailed,
            Some(c) if self.already_exists.contains(&c) => Rejection::AlreadyExists,
            Some(c) if self.not_found.contains(&c) => Rejection::NotFound,
            _ => Rejection::Other,
        }
    }
}
/// Strings and numbers as text; anything else is absent.
pub fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Pin of a tool contract: SHA-256 over canonical JSON of its input and output schemas.
pub fn schema_hash(tool: &Value) -> String {
    let pinned = serde_json::json!({
        "inputSchema": tool.get("inputSchema").cloned().unwrap_or(Value::Null),
        "outputSchema": tool.get("outputSchema").cloned().unwrap_or(Value::Null),
    });
    let mut out = String::new();
    canonical(&pinned, &mut out);
    Sha256::digest(out.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Sorted keys and no whitespace, independent of serde_json's map ordering feature.
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn schema_hash_ignores_key_order_and_descriptions() {
        let a = json!({"name": "x", "description": "one", "inputSchema": {"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "number"}}}});
        let b = json!({"description": "two", "inputSchema": {"properties": {"b": {"type": "number"}, "a": {"type": "string"}}, "type": "object"}, "name": "x"});
        assert_eq!(schema_hash(&a), schema_hash(&b));
        let c =
            json!({"inputSchema": {"type": "object", "properties": {"a": {"type": "integer"}}}});
        assert_ne!(schema_hash(&a), schema_hash(&c));
    }
    #[test]
    fn render_substitutes_whole_placeholders_and_omits_absent_values() {
        let profile = Profile::parse(
            json!({
                "profile_version": 1, "name": "t", "result_source": "structured", "content_encoding": "base64",
                "operations": {
                    "list": {"tool": "ls", "schema_sha256": "0".repeat(64),
                        "arguments": {"folder": "{folder_id}", "page": "{cursor}", "literal": "{not-closed", "opts": {"deep": false}},
                        "result": {"items": "/items", "item_id": "/id", "item_name": "/name", "item_kind": "/type", "next_cursor": "/next"}},
                    "read": {"tool": "cat", "schema_sha256": "0".repeat(64), "arguments": {"id": "{file_id}"},
                        "result": {"content": "/c", "revision": "/r"}}
                }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            profile
                .render("list", &[("folder_id", Some("root")), ("cursor", None)])
                .unwrap(),
            json!({"folder": "root", "literal": "{not-closed", "opts": {"deep": false}})
        );
        assert!(profile.write_safety("update").is_err());
    }
    #[test]
    fn profiles_without_pagination_marker_or_unknown_placeholders_are_rejected() {
        let base = |args: Value| {
            json!({
                "profile_version": 1, "name": "t", "result_source": "structured", "content_encoding": "base64",
                "operations": {
                    "list": {"tool": "ls", "schema_sha256": "0".repeat(64), "arguments": args,
                        "result": {"items": "/items", "item_id": "/id", "item_name": "/name", "item_kind": "/type", "next_cursor": "/next"}},
                    "read": {"tool": "cat", "schema_sha256": "0".repeat(64), "arguments": {"id": "{file_id}"},
                        "result": {"content": "/c", "revision": "/r"}}
                }
            })
            .to_string()
        };
        assert!(Profile::parse(base(json!({"folder": "{folder_id}"})).as_bytes()).is_err());
        assert!(
            Profile::parse(
                base(json!({"folder": "{folder_id}", "c": "{cursor}", "x": "{token}"})).as_bytes()
            )
            .is_err()
        );
    }
}
