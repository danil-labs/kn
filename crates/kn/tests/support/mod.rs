#![allow(dead_code)]
//! Reference MCP server for tests: an in-memory folder tree served over
//! Streamable HTTP (MCP 2025-11-25), with knobs to inject failures.
use base64::Engine as _;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use tiny_http::{Header, Method, Request, Response, Server};

const PROTOCOL: &str = "2025-11-25";
const WRITES: [&str; 4] = ["update_file", "create_file", "create_folder", "delete_file"];
const FIRST_PAGE: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Folder,
    Native,
}
impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Folder => "folder",
            Kind::Native => "native",
        }
    }
}
struct Item {
    name: String,
    parent: Option<String>,
    kind: Kind,
    bytes: Vec<u8>,
    revision: String,
}
struct State {
    items: BTreeMap<String, Item>,
    next_id: u64,
    next_rev: u64,
    token: Option<String>,
    page_size: usize,
    sse: bool,
    sessions: HashSet<String>,
    calls: BTreeMap<String, usize>,
    writes: usize,
    lists: usize,
    // Absolute call numbers (list_children or write calls) that trigger a fault.
    fail_lists: Vec<usize>,
    lose_writes: Vec<usize>,
    mutations: Vec<(usize, String, Option<Vec<u8>>)>,
    altered: BTreeSet<String>,
}

pub struct FakeServer {
    state: Arc<Mutex<State>>,
    server: Arc<Server>,
    port: u16,
    thread: Option<JoinHandle<()>>,
}

impl FakeServer {
    pub fn start() -> Self {
        let mut state = State {
            items: BTreeMap::new(),
            next_id: 0,
            next_rev: 0,
            token: Some("test-token".into()),
            page_size: 2,
            sse: true,
            sessions: HashSet::new(),
            calls: BTreeMap::new(),
            writes: 0,
            lists: 0,
            fail_lists: vec![],
            lose_writes: vec![],
            mutations: vec![],
            altered: BTreeSet::new(),
        };
        let revision = state.rev();
        state.items.insert(
            "root".into(),
            Item {
                name: String::new(),
                parent: None,
                kind: Kind::Folder,
                bytes: vec![],
                revision,
            },
        );
        let state = Arc::new(Mutex::new(state));
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind fake MCP server"));
        let port = server.server_addr().to_ip().expect("ip listener").port();
        let thread = {
            let (state, server) = (state.clone(), server.clone());
            std::thread::spawn(move || {
                for mut request in server.incoming_requests() {
                    let reply = handle(&state, port, &mut request);
                    reply.send(request);
                }
            })
        };
        Self {
            state,
            server,
            port,
            thread: Some(thread),
        }
    }
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }
    pub fn set_token(&self, token: Option<&str>) {
        self.lock().token = token.map(Into::into);
    }
    pub fn set_page_size(&self, n: usize) {
        assert!(n > 0, "page size must be positive");
        self.lock().page_size = n;
    }
    pub fn set_sse(&self, on: bool) {
        self.lock().sse = on;
    }
    /// Serve a different schema for `tool`, as if the provider changed its contract.
    pub fn alter_schema(&self, tool: &str) {
        assert!(
            tools().iter().any(|t| t["name"] == tool),
            "unknown tool {tool}"
        );
        self.lock().altered.insert(tool.into());
    }
    pub fn put(&self, path: &str, bytes: &[u8]) {
        self.lock().put(path, Kind::File, bytes);
    }
    pub fn put_native(&self, path: &str) {
        self.lock().put(path, Kind::Native, b"");
    }
    pub fn mkdir(&self, path: &str) {
        self.lock().folder(&segments(path));
    }
    pub fn remove(&self, path: &str) {
        self.lock().remove(path);
    }
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        let st = self.lock();
        let item = &st.items[&st.resolve(path)?];
        (item.kind == Kind::File).then(|| item.bytes.clone())
    }
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        let st = self.lock();
        st.items
            .iter()
            .filter(|(_, i)| i.kind == Kind::File)
            .map(|(id, i)| (st.path_of(id), i.bytes.clone()))
            .collect()
    }
    pub fn revision(&self, path: &str) -> Option<String> {
        let st = self.lock();
        Some(st.items[&st.resolve(path)?].revision.clone())
    }
    pub fn write_calls(&self) -> usize {
        self.lock().writes
    }
    pub fn calls(&self, tool: &str) -> usize {
        self.lock().calls.get(tool).copied().unwrap_or(0)
    }
    pub fn fail_list_call(&self, n: usize) {
        let mut st = self.lock();
        let at = st.lists + n;
        st.fail_lists.push(at);
    }
    pub fn lose_write_response(&self, n: usize) {
        let mut st = self.lock();
        let at = st.writes + n;
        st.lose_writes.push(at);
    }
    pub fn mutate_before_write(&self, n: usize, path: &str, bytes: Option<&[u8]>) {
        let mut st = self.lock();
        let at = st.writes + n;
        st.mutations
            .push((at, path.into(), bytes.map(<[u8]>::to_vec)));
    }
}
impl Drop for FakeServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn profile_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/profiles/reference.json")
}

pub fn tools() -> Vec<Value> {
    let s = || json!({"type": "string"});
    let kind = || json!({"type": "string", "enum": ["file", "folder", "native"]});
    let obj = |props: Value, required: &[&str]| json!({"type": "object", "properties": props, "required": required});
    let tool = |name: &str, description: &str, input: Value, output: Value| json!({"name": name, "description": description, "inputSchema": input, "outputSchema": output});
    let entry = obj(
        json!({"id": s(), "name": s(), "type": kind(), "revision": s()}),
        &["id", "name", "type", "revision"],
    );
    vec![
        tool(
            "get_item",
            "Describe an item.",
            obj(json!({"item_id": s()}), &["item_id"]),
            obj(
                json!({"id": s(), "name": s(), "type": kind()}),
                &["id", "name", "type"],
            ),
        ),
        tool(
            "list_children",
            "List a folder, sorted by name.",
            obj(json!({"folder_id": s(), "cursor": s()}), &["folder_id"]),
            obj(
                json!({"items": {"type": "array", "items": entry}, "next_cursor": {"type": ["string", "null"]}}),
                &["items", "next_cursor"],
            ),
        ),
        tool(
            "read_file",
            "Read a file's bytes and revision.",
            obj(json!({"file_id": s()}), &["file_id"]),
            obj(
                json!({"content_base64": s(), "revision": s()}),
                &["content_base64", "revision"],
            ),
        ),
        tool(
            "update_file",
            "Replace a file's bytes if its revision still matches.",
            obj(
                json!({"file_id": s(), "content_base64": s(), "if_revision": s()}),
                &["file_id", "content_base64", "if_revision"],
            ),
            obj(json!({"id": s(), "revision": s()}), &["id", "revision"]),
        ),
        tool(
            "create_file",
            "Create a file; fails if the name exists.",
            obj(
                json!({"folder_id": s(), "name": s(), "content_base64": s()}),
                &["folder_id", "name", "content_base64"],
            ),
            obj(json!({"id": s(), "revision": s()}), &["id", "revision"]),
        ),
        tool(
            "create_folder",
            "Create a folder; fails if the name exists.",
            obj(
                json!({"folder_id": s(), "name": s()}),
                &["folder_id", "name"],
            ),
            obj(json!({"id": s()}), &["id"]),
        ),
        tool(
            "delete_file",
            "Delete a file if its revision still matches.",
            obj(
                json!({"file_id": s(), "if_revision": s()}),
                &["file_id", "if_revision"],
            ),
            obj(json!({"deleted": {"type": "boolean"}}), &["deleted"]),
        ),
    ]
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}
fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}
fn fail(code: &str, message: &str) -> Value {
    json!({"code": code, "message": message})
}
fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, Value> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| fail("invalid_arguments", &format!("{key} is required")))
}
fn decode(args: &Value) -> Result<Vec<u8>, Value> {
    base64::engine::general_purpose::STANDARD
        .decode(arg(args, "content_base64")?)
        .map_err(|_| fail("invalid_arguments", "content_base64 is not base64"))
}
fn not_found() -> Value {
    fail("not_found", "no such item")
}

impl State {
    fn rev(&mut self) -> String {
        self.next_rev += 1;
        format!("r{}", self.next_rev)
    }
    fn child(&self, folder: &str, name: &str) -> Option<String> {
        self.items
            .iter()
            .find(|(_, i)| i.parent.as_deref() == Some(folder) && i.name == name)
            .map(|(id, _)| id.clone())
    }
    fn resolve(&self, path: &str) -> Option<String> {
        segments(path)
            .into_iter()
            .try_fold("root".to_string(), |id, name| self.child(&id, name))
    }
    fn path_of(&self, id: &str) -> String {
        let mut names = vec![];
        let mut cur = &self.items[id];
        while let Some(parent) = &cur.parent {
            names.push(cur.name.as_str());
            cur = &self.items[parent];
        }
        names.reverse();
        names.join("/")
    }
    fn is_folder(&self, id: &str) -> bool {
        self.items.get(id).is_some_and(|i| i.kind == Kind::Folder)
    }
    fn insert(&mut self, parent: &str, name: &str, kind: Kind, bytes: Vec<u8>) -> String {
        self.next_id += 1;
        let id = format!("f{}", self.next_id);
        let revision = self.rev();
        self.items.insert(
            id.clone(),
            Item {
                name: name.into(),
                parent: Some(parent.into()),
                kind,
                bytes,
                revision,
            },
        );
        id
    }
    fn folder(&mut self, path: &[&str]) -> String {
        let mut id = "root".to_string();
        for name in path {
            id = match self.child(&id, name) {
                Some(child) => {
                    assert!(self.is_folder(&child), "{name} is not a folder");
                    child
                }
                None => self.insert(&id, name, Kind::Folder, vec![]),
            };
        }
        id
    }
    fn put(&mut self, path: &str, kind: Kind, bytes: &[u8]) {
        let segs = segments(path);
        let (name, dirs) = segs.split_last().expect("empty path");
        let parent = self.folder(dirs);
        match self.child(&parent, name) {
            Some(id) => {
                assert!(!self.is_folder(&id), "{path} is a folder");
                let revision = self.rev();
                let item = self.items.get_mut(&id).unwrap();
                (item.kind, item.bytes, item.revision) = (kind, bytes.to_vec(), revision);
            }
            None => {
                self.insert(&parent, name, kind, bytes.to_vec());
            }
        }
    }
    fn remove(&mut self, path: &str) {
        let id = self.resolve(path).expect("no such path");
        self.remove_tree(&id);
        self.rev();
    }
    fn remove_tree(&mut self, id: &str) {
        let children: Vec<String> = self
            .items
            .iter()
            .filter(|(_, i)| i.parent.as_deref() == Some(id))
            .map(|(c, _)| c.clone())
            .collect();
        for child in children {
            self.remove_tree(&child);
        }
        self.items.remove(id);
    }
    fn served_tools(&self) -> Vec<Value> {
        let mut tools = tools();
        for tool in &mut tools {
            if self
                .altered
                .contains(tool["name"].as_str().unwrap_or_default())
            {
                tool["inputSchema"]["properties"]["note"] = json!({"type": "string"});
            }
        }
        tools
    }
    fn call(&mut self, tool: &str, a: &Value) -> Result<Value, Value> {
        match tool {
            "get_item" => {
                let id = arg(a, "item_id")?;
                let item = self.items.get(id).ok_or_else(not_found)?;
                Ok(json!({"id": id, "name": item.name, "type": item.kind.as_str()}))
            }
            "list_children" => {
                let folder = arg(a, "folder_id")?;
                if !self.is_folder(folder) {
                    return Err(not_found());
                }
                let offset = match a.get("cursor") {
                    None | Some(Value::Null) => 0,
                    Some(c) => c
                        .as_str()
                        .and_then(|c| c.parse().ok())
                        .ok_or_else(|| fail("invalid_arguments", "bad cursor"))?,
                };
                let mut children: Vec<_> = self
                    .items
                    .iter()
                    .filter(|(_, i)| i.parent.as_deref() == Some(folder))
                    .collect();
                children.sort_by(|x, y| x.1.name.cmp(&y.1.name));
                let items: Vec<Value> = children
                    .iter()
                    .skip(offset)
                    .take(self.page_size)
                    .map(|(id, i)| json!({"id": id, "name": i.name, "type": i.kind.as_str(), "revision": i.revision}))
                    .collect();
                let end = offset + self.page_size;
                let next = (end < children.len()).then(|| end.to_string());
                Ok(json!({"items": items, "next_cursor": next}))
            }
            "read_file" => {
                let item = self.items.get(arg(a, "file_id")?).ok_or_else(not_found)?;
                match item.kind {
                    Kind::File => Ok(json!({
                        "content_base64": base64::engine::general_purpose::STANDARD.encode(&item.bytes),
                        "revision": item.revision,
                    })),
                    Kind::Native => Err(fail("not_downloadable", "native documents have no bytes")),
                    Kind::Folder => Err(fail("not_a_file", "item is a folder")),
                }
            }
            "update_file" => {
                let id = arg(a, "file_id")?;
                let item = self.items.get(id).ok_or_else(not_found)?;
                if item.kind != Kind::File {
                    return Err(fail("not_a_file", "only regular files can be updated"));
                }
                if item.revision != arg(a, "if_revision")? {
                    return Err(fail("precondition_failed", "revision changed"));
                }
                let bytes = decode(a)?;
                let revision = self.rev();
                let item = self.items.get_mut(id).unwrap();
                (item.bytes, item.revision) = (bytes, revision.clone());
                Ok(json!({"id": id, "revision": revision}))
            }
            "create_file" | "create_folder" => {
                let (folder, name) = (arg(a, "folder_id")?, arg(a, "name")?);
                if !self.is_folder(folder) {
                    return Err(not_found());
                }
                if name.is_empty() || name.contains('/') {
                    return Err(fail("invalid_arguments", "bad name"));
                }
                if self.child(folder, name).is_some() {
                    return Err(fail("already_exists", "name already used in folder"));
                }
                if tool == "create_folder" {
                    let id = self.insert(folder, name, Kind::Folder, vec![]);
                    return Ok(json!({"id": id}));
                }
                let id = self.insert(folder, name, Kind::File, decode(a)?);
                Ok(json!({"id": id, "revision": self.items[&id].revision}))
            }
            "delete_file" => {
                let id = arg(a, "file_id")?;
                let item = self.items.get(id).ok_or_else(not_found)?;
                if item.kind == Kind::Folder {
                    return Err(fail("not_a_file", "item is a folder"));
                }
                if item.revision != arg(a, "if_revision")? {
                    return Err(fail("precondition_failed", "revision changed"));
                }
                self.items.remove(id);
                self.rev();
                Ok(json!({"deleted": true}))
            }
            _ => unreachable!("unknown tools are rejected before dispatch"),
        }
    }
    /// Count the call and apply injected faults. Returns (fail, lose response).
    fn faults(&mut self, tool: &str) -> (bool, bool) {
        *self.calls.entry(tool.into()).or_default() += 1;
        if tool == "list_children" {
            self.lists += 1;
            return (take(&mut self.fail_lists, self.lists), false);
        }
        if !WRITES.contains(&tool) {
            return (false, false);
        }
        self.writes += 1;
        let n = self.writes;
        let (due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.mutations)
            .into_iter()
            .partition(|m| m.0 == n);
        self.mutations = rest;
        for (_, path, bytes) in due {
            match bytes {
                Some(bytes) => self.put(&path, Kind::File, &bytes),
                None if self.resolve(&path).is_some() => self.remove(&path),
                None => {}
            }
        }
        (false, take(&mut self.lose_writes, n))
    }
}
fn take(pending: &mut Vec<usize>, n: usize) -> bool {
    let before = pending.len();
    pending.retain(|&at| at != n);
    pending.len() != before
}

struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}
impl Reply {
    fn status(status: u16) -> Self {
        Self {
            status,
            headers: vec![],
            body: vec![],
        }
    }
    fn json(message: &Value) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type", "application/json".into())],
            body: message.to_string().into_bytes(),
        }
    }
    /// An unrelated log event first, then `messages`; the stream then ends.
    fn sse(messages: &[Value]) -> Self {
        let notice = json!({"jsonrpc": "2.0", "method": "notifications/message",
            "params": {"level": "info", "data": "working"}});
        let body = std::iter::once(&notice)
            .chain(messages)
            .map(|m| format!("data: {m}\n\n"))
            .collect::<String>();
        Self {
            status: 200,
            headers: vec![("Content-Type", "text/event-stream".into())],
            body: body.into_bytes(),
        }
    }
    fn send(self, request: Request) {
        let mut response = Response::from_data(self.body).with_status_code(self.status);
        for (key, value) in self.headers {
            response.add_header(Header::from_bytes(key, value).expect("valid header"));
        }
        let _ = request.respond(response);
    }
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn handle(state: &Mutex<State>, port: u16, request: &mut Request) -> Reply {
    if request.method() != &Method::Post || request.url() != "/mcp" {
        return Reply::status(404);
    }
    let headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .map(|h| {
            (
                h.field.as_str().as_str().to_ascii_lowercase(),
                h.value.as_str().to_owned(),
            )
        })
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let accept = header("accept").unwrap_or_default();
    if !accept.contains("application/json") || !accept.contains("text/event-stream") {
        return Reply::status(400);
    }
    let mut st = lock(state);
    if let Some(token) = &st.token
        && header("authorization") != Some(format!("Bearer {token}").as_str())
    {
        let mut reply = Reply::status(401);
        reply.headers.push((
            "WWW-Authenticate",
            format!(
                "Bearer resource_metadata=\"http://127.0.0.1:{port}/.well-known/oauth-protected-resource/mcp\""
            ),
        ));
        return reply;
    }
    let mut body = vec![];
    if request.as_reader().read_to_end(&mut body).is_err() {
        return Reply::status(400);
    }
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return Reply::status(400);
    };
    if !message.is_object() {
        return Reply::status(400);
    }
    let method = message["method"].as_str().unwrap_or_default();
    let id = message.get("id").cloned();
    if method == "initialize" {
        let Some(id) = id else {
            return Reply::status(400);
        };
        let session = session_id();
        st.sessions.insert(session.clone());
        let mut reply = Reply::json(&json!({"jsonrpc": "2.0", "id": id, "result": {
            "protocolVersion": PROTOCOL,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "kn-reference", "version": "1"},
        }}));
        reply.headers.push(("Mcp-Session-Id", session));
        return reply;
    }
    match header("mcp-session-id") {
        None => return Reply::status(400),
        Some(s) if !st.sessions.contains(s) => return Reply::status(404),
        Some(_) => {}
    }
    if header("mcp-protocol-version") != Some(PROTOCOL) {
        return Reply::status(400);
    }
    let Some(id) = id else {
        return Reply::status(202);
    };
    let result = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
    match method {
        "ping" => Reply::json(&result(json!({}))),
        "tools/list" => {
            let tools = st.served_tools();
            match message["params"].get("cursor").and_then(Value::as_str) {
                None => Reply::json(&result(
                    json!({"tools": tools[..FIRST_PAGE], "nextCursor": "p2"}),
                )),
                Some("p2") => Reply::json(&result(json!({"tools": tools[FIRST_PAGE..]}))),
                Some(_) => Reply::json(&rpc_error(&id, -32602, "Invalid cursor")),
            }
        }
        "tools/call" => {
            let tool = message["params"]["name"].as_str().unwrap_or_default();
            if !tools().iter().any(|t| t["name"] == tool) {
                return Reply::json(&rpc_error(&id, -32602, "Unknown tool"));
            }
            let args = message["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (failed, lost) = st.faults(tool);
            let response = if failed {
                rpc_error(&id, -32603, "Injected failure")
            } else {
                let (payload, is_error) = match st.call(tool, &args) {
                    Ok(payload) => (payload, false),
                    Err(payload) => (payload, true),
                };
                result(json!({
                    "content": [{"type": "text", "text": payload.to_string()}],
                    "structuredContent": payload,
                    "isError": is_error,
                }))
            };
            if lost {
                Reply::sse(&[])
            } else if st.sse {
                Reply::sse(&[response])
            } else {
                Reply::json(&response)
            }
        }
        _ => Reply::json(&rpc_error(&id, -32601, "Method not found")),
    }
}

fn session_id() -> String {
    use std::hash::{BuildHasher, RandomState};
    let seed = RandomState::new();
    format!("s-{:016x}{:016x}", seed.hash_one(1u8), seed.hash_one(2u8))
}
