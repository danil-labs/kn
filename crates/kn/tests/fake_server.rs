//! The real kn_core MCP client against the reference server in `support`.
mod support;
use kn_core::error::{Error, Result};
use kn_core::remote::{
    driver::{Item, ItemKind, Rejection, Remote, WriteOutcome},
    http::Http,
    profile::{Profile, schema_hash},
};
use support::{FakeServer, profile_path, tools};

const TOKEN: &str = "test-token";

fn profile() -> Profile {
    Profile::parse(&std::fs::read(profile_path()).expect("read profile")).expect("valid profile")
}
fn try_open(server: &FakeServer, token: Option<&str>, allow_writes: bool) -> Result<Remote> {
    Remote::open(
        Http::new(),
        &server.endpoint(),
        token.map(Into::into),
        profile(),
        allow_writes,
    )
}
fn open(server: &FakeServer) -> Remote {
    try_open(server, Some(TOKEN), true).expect("open remote")
}
fn list_all(remote: &mut Remote, folder: &str) -> Vec<Item> {
    let (mut items, mut cursor) = (vec![], None);
    loop {
        let page = remote.list(folder, cursor.as_deref()).expect("list");
        items.extend(page.items);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return items,
        }
    }
}
fn find(remote: &mut Remote, name: &str) -> Item {
    list_all(remote, "root")
        .into_iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("{name} not listed"))
}
fn applied(outcome: WriteOutcome) -> (Option<String>, Option<String>) {
    match outcome {
        WriteOutcome::Applied { id, revision } => (id, revision),
        WriteOutcome::Rejected { reason, message } => panic!("rejected {reason:?}: {message}"),
        WriteOutcome::Unconfirmed(message) => panic!("unconfirmed: {message}"),
    }
}
fn rejected(outcome: WriteOutcome) -> Rejection {
    match outcome {
        WriteOutcome::Rejected { reason, .. } => reason,
        WriteOutcome::Applied { .. } => panic!("applied, expected a rejection"),
        WriteOutcome::Unconfirmed(message) => panic!("unconfirmed: {message}"),
    }
}
fn code<T>(result: Result<T>) -> &'static str {
    result.err().expect("expected an error").code()
}

#[test]
fn profile_pins_match_served_tools_through_pagination_and_sse() {
    let server = FakeServer::start();
    let remote = open(&server);
    assert_eq!(remote.protocol(), "2025-11-25");
    assert_eq!(remote.server()["name"], "kn-reference");
    let caps = remote.capabilities();
    assert_eq!(caps.len(), 7);
    for cap in caps {
        assert!(cap.available, "{}: {:?}", cap.operation, cap.reason);
    }
    let profile = profile();
    for tool in tools() {
        assert!(
            profile
                .operations
                .values()
                .any(|op| op.tool == tool["name"] && op.schema_sha256 == schema_hash(&tool)),
            "{} is not pinned",
            tool["name"]
        );
    }
}

#[test]
fn lists_reads_and_writes_with_revision_checks() {
    let server = FakeServer::start();
    for name in ["a.md", "b.md", "c.md", "d.md"] {
        server.put(name, name.as_bytes());
    }
    server.mkdir("e");
    let mut remote = open(&server);
    let root = remote.identify("root").unwrap();
    assert_eq!(root.kind, ItemKind::Folder);

    let first = remote.list("root", None).unwrap();
    assert_eq!(first.items.len(), 2);
    assert!(first.next_cursor.is_some());
    let items = list_all(&mut remote, "root");
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["a.md", "b.md", "c.md", "d.md", "e"]);
    assert_eq!(items[4].kind, ItemKind::Folder);
    assert_eq!(server.calls("list_children"), 4);

    let a = find(&mut remote, "a.md");
    let read = remote.read(&a.id).unwrap();
    assert_eq!(read.bytes, b"a.md");
    assert_eq!(Some(&read.revision), server.revision("a.md").as_ref());
    assert_eq!(a.revision.as_ref(), Some(&read.revision));

    let (_, revision) = applied(remote.update(&a.id, b"new", &read.revision).unwrap());
    assert_eq!(server.get("a.md").unwrap(), b"new");
    assert_eq!(revision, server.revision("a.md"));
    assert_ne!(revision.as_ref(), Some(&read.revision));
    let stale = remote.update(&a.id, b"lost", &read.revision).unwrap();
    assert_eq!(rejected(stale), Rejection::PreconditionFailed);
    assert_eq!(server.get("a.md").unwrap(), b"new");

    let exists = remote.create("root", "b.md", b"dup").unwrap();
    assert_eq!(rejected(exists), Rejection::AlreadyExists);
    assert_eq!(server.get("b.md").unwrap(), b"b.md");
    let (id, revision) = applied(remote.create("root", "f.md", b"fresh").unwrap());
    assert!(id.is_some());
    assert_eq!(server.get("f.md").unwrap(), b"fresh");
    assert_eq!(revision, server.revision("f.md"));
    let exists = remote.create_folder("root", "e").unwrap();
    assert_eq!(rejected(exists), Rejection::AlreadyExists);
    let (folder, _) = applied(remote.create_folder("root", "g").unwrap());
    applied(remote.create(&folder.unwrap(), "h.md", b"deep").unwrap());
    assert_eq!(server.get("g/h.md").unwrap(), b"deep");

    let b = find(&mut remote, "b.md");
    let old = b.revision.clone().unwrap();
    server.put("b.md", b"edited");
    assert_eq!(
        rejected(remote.delete(&b.id, &old).unwrap()),
        Rejection::PreconditionFailed
    );
    let current = server.revision("b.md").unwrap();
    applied(remote.delete(&b.id, &current).unwrap());
    assert_eq!(server.get("b.md"), None);
    assert_eq!(
        rejected(remote.delete(&b.id, &current).unwrap()),
        Rejection::NotFound
    );
    assert_eq!(
        server.files().into_keys().collect::<Vec<_>>(),
        ["a.md", "c.md", "d.md", "f.md", "g/h.md"]
    );
    assert_eq!(server.write_calls(), 10);
    assert_eq!(server.calls("delete_file"), 3);
}

#[test]
fn plain_json_responses_work_too() {
    let server = FakeServer::start();
    server.set_sse(false);
    server.put("x.txt", b"\0binary\xff");
    let mut remote = open(&server);
    let x = find(&mut remote, "x.txt");
    let read = remote.read(&x.id).unwrap();
    assert_eq!(read.bytes, b"\0binary\xff");
    applied(remote.update(&x.id, b"y", &read.revision).unwrap());
    assert_eq!(server.get("x.txt").unwrap(), b"y");
}

#[test]
fn lost_write_response_is_unconfirmed_although_applied() {
    let server = FakeServer::start();
    server.put("a.md", b"old");
    let mut remote = open(&server);
    let a = find(&mut remote, "a.md");
    server.lose_write_response(1);
    let outcome = remote
        .update(&a.id, b"new", a.revision.as_deref().unwrap())
        .unwrap();
    assert!(matches!(outcome, WriteOutcome::Unconfirmed(_)));
    assert_eq!(server.get("a.md").unwrap(), b"new");
    assert_eq!(server.write_calls(), 1);
}

#[test]
fn concurrent_edit_before_write_is_preserved() {
    let server = FakeServer::start();
    server.put("a.md", b"old");
    let mut remote = open(&server);
    let a = find(&mut remote, "a.md");
    let read = remote.read(&a.id).unwrap();
    server.mutate_before_write(1, "a.md", Some(b"theirs"));
    let outcome = remote.update(&a.id, b"ours", &read.revision).unwrap();
    assert_eq!(rejected(outcome), Rejection::PreconditionFailed);
    assert_eq!(server.get("a.md").unwrap(), b"theirs");
}

#[test]
fn missing_or_wrong_token_requires_auth() {
    let server = FakeServer::start();
    assert_eq!(
        code(try_open(&server, Some("wrong"), true)),
        "AUTH_REQUIRED"
    );
    assert_eq!(code(try_open(&server, None, true)), "AUTH_REQUIRED");
    server.set_token(None);
    assert!(try_open(&server, None, true).is_ok());
}

#[test]
fn read_only_mode_never_sends_writes() {
    let server = FakeServer::start();
    server.put("a.md", b"old");
    let mut remote = try_open(&server, Some(TOKEN), false).unwrap();
    let a = find(&mut remote, "a.md");
    let revision = a.revision.clone().unwrap();
    let forbidden = "MODE_FORBIDS_OPERATION";
    assert_eq!(code(remote.update(&a.id, b"x", &revision)), forbidden);
    assert_eq!(code(remote.create("root", "n.md", b"x")), forbidden);
    assert_eq!(code(remote.create_folder("root", "n")), forbidden);
    assert_eq!(code(remote.delete(&a.id, &revision)), forbidden);
    assert_eq!(server.write_calls(), 0);
    assert_eq!(server.get("a.md").unwrap(), b"old");
}

#[test]
fn native_items_are_remote_only() {
    let server = FakeServer::start();
    server.put_native("Doc");
    server.put("a.md", b"a");
    let mut remote = open(&server);
    let doc = find(&mut remote, "Doc");
    assert_eq!(doc.kind, ItemKind::RemoteOnly);
    assert_eq!(code(remote.read(&doc.id)), "REMOTE_ERROR");
    assert_eq!(server.files().into_keys().collect::<Vec<_>>(), ["a.md"]);
}

#[test]
fn changed_schema_disables_only_that_capability() {
    let server = FakeServer::start();
    server.put("a.md", b"old");
    server.alter_schema("update_file");
    let mut remote = open(&server);
    for cap in remote.capabilities() {
        assert_eq!(
            cap.available,
            cap.operation != "update",
            "{}",
            cap.operation
        );
    }
    let err = remote.require("update").expect_err("mismatch");
    assert!(matches!(err, Error::ProfileMismatch(_)));
    assert_eq!(err.code(), "PROFILE_MISMATCH");
    let a = find(&mut remote, "a.md");
    let revision = a.revision.clone().unwrap();
    assert_eq!(
        code(remote.update(&a.id, b"x", &revision)),
        "PROFILE_MISMATCH"
    );
    assert_eq!(server.write_calls(), 0);
    assert!(remote.require("read").is_ok());
}

#[test]
fn failed_list_call_is_an_error_not_an_empty_page() {
    let server = FakeServer::start();
    for name in ["a", "b", "c"] {
        server.put(name, b"x");
    }
    let mut remote = open(&server);
    server.fail_list_call(2);
    let first = remote.list("root", None).unwrap();
    let cursor = first.next_cursor.expect("second page");
    // An internal JSON-RPC error has an unknown outcome, never an empty page.
    assert_eq!(
        code(remote.list("root", Some(&cursor))),
        "REMOTE_UNAVAILABLE"
    );
    assert_eq!(remote.list("root", Some(&cursor)).unwrap().items.len(), 1);
}

#[test]
fn transport_rules_are_enforced() {
    let server = FakeServer::start();
    let url = server.endpoint();
    let http = Http::new();
    let auth = format!("Bearer {TOKEN}");
    let accept = "application/json, text/event-stream";
    let body =
        |method: &str| format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{{}}}}"#);
    let post =
        |headers: &[(&str, &str)], body: &str| http.post(&url, headers, body.as_bytes()).unwrap();

    assert_eq!(http.get(&url, &[("Accept", accept)]).unwrap().status, 404);
    let other = url.replace("/mcp", "/other");
    assert_eq!(http.post(&other, &[], b"{}").unwrap().status, 404);
    let init = body("initialize");
    assert_eq!(post(&[("Authorization", &auth)], &init).status, 400);
    let denied = post(&[("Accept", accept)], &init);
    assert_eq!(denied.status, 401);
    let port = url.split(':').nth(2).unwrap().trim_end_matches("/mcp");
    assert_eq!(
        denied.header("www-authenticate").unwrap(),
        format!(
            "Bearer resource_metadata=\"http://127.0.0.1:{port}/.well-known/oauth-protected-resource/mcp\""
        )
    );

    let ok = post(&[("Accept", accept), ("Authorization", &auth)], &init);
    assert_eq!(ok.status, 200);
    assert!(ok.content_type().starts_with("application/json"));
    let session = ok.header("mcp-session-id").unwrap().to_owned();
    assert!(!session.is_empty() && session.bytes().all(|b| (0x21..=0x7e).contains(&b)));

    let base = [("Accept", accept), ("Authorization", auth.as_str())];
    fn with<'a>(
        base: &[(&'a str, &'a str)],
        extra: &[(&'a str, &'a str)],
    ) -> Vec<(&'a str, &'a str)> {
        [base, extra].concat()
    }
    let list = body("tools/list");
    let version = ("MCP-Protocol-Version", "2025-11-25");
    assert_eq!(post(&with(&base, &[version]), &list).status, 400);
    assert_eq!(
        post(&with(&base, &[("Mcp-Session-Id", "nope"), version]), &list).status,
        404
    );
    let session = ("Mcp-Session-Id", session.as_str());
    assert_eq!(post(&with(&base, &[session]), &list).status, 400);
    assert_eq!(
        post(
            &with(&base, &[session, ("MCP-Protocol-Version", "2025-06-18")]),
            &list
        )
        .status,
        400
    );
    let notify = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let accepted = post(&with(&base, &[session, version]), notify);
    assert_eq!(accepted.status, 202);
    assert!(accepted.bytes().unwrap().is_empty());

    let page: serde_json::Value = serde_json::from_slice(
        &post(&with(&base, &[session, version]), &list)
            .bytes()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page["id"], 1);
    assert_eq!(page["result"]["tools"].as_array().unwrap().len(), 4);
    assert_eq!(page["result"]["nextCursor"], "p2");
    let unknown: serde_json::Value = serde_json::from_slice(
        &post(&with(&base, &[session, version]), &body("resources/list"))
            .bytes()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);

    let call = r#"{"jsonrpc":"2.0","id":"c1","method":"tools/call","params":{"name":"get_item","arguments":{"item_id":"missing"}}}"#;
    let reply = post(&with(&base, &[session, version]), call);
    assert!(reply.content_type().starts_with("text/event-stream"));
    let text = String::from_utf8(reply.bytes().unwrap()).unwrap();
    let events: Vec<serde_json::Value> = text
        .split("\n\n")
        .filter(|e| !e.is_empty())
        .map(|e| serde_json::from_str(e.strip_prefix("data: ").unwrap()).unwrap())
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["method"], "notifications/message");
    assert_eq!(events[1]["id"], "c1");
    let result = &events[1]["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "not_found");
    let text: serde_json::Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
}
