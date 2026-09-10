//! MCP remotes end to end: the real CLI against the reference server. These tests
//! certify kn's rules, not any provider: mocks alone never certify Drive or Graph.
mod support;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::{FakeServer, profile_path};

struct Env {
    _tmp: tempfile::TempDir,
    main: PathBuf,
    home: PathBuf,
    server: FakeServer,
}
impl Env {
    fn new() -> Self {
        Self::at(&["documents"])
    }
    fn at(parts: &[&str]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let main = parts
            .iter()
            .fold(tmp.path().to_path_buf(), |p, part| p.join(part));
        fs::create_dir_all(&main).unwrap();
        Self {
            home: tmp.path().join("engine"),
            main,
            _tmp: tmp,
            server: FakeServer::start(),
        }
    }
    fn run_token(&self, dir: &Path, args: &[&str], token: &str, code: i32) -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_kn"))
            .current_dir(dir)
            .env("KN_HOME", &self.home)
            .env("KN_MCP_ACCESS_TOKEN", token)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(code),
            "{args:?}: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn run(&self, dir: &Path, args: &[&str], code: i32) -> Value {
        self.run_token(dir, args, "test-token", code)
    }
    fn kn(&self, args: &[&str], code: i32) -> Value {
        self.run(&self.main, args, code)
    }
    fn add_remote(&self) {
        let profile = profile_path();
        let endpoint = self.server.endpoint();
        self.kn(
            &[
                "remote",
                "add",
                "drive",
                &endpoint,
                "--profile",
                profile.to_str().unwrap(),
                "--root",
                "root",
            ],
            0,
        );
    }
    fn connect(&self, mode: &str) {
        self.kn(&["init"], 0);
        self.add_remote();
        let mut args = vec!["mode", "set", mode, "--remote", "drive"];
        if mode == "mcp" {
            args.push("--primary-outside-sync");
        }
        self.kn(&args, 0);
    }
    fn session(&self, name: &str) -> PathBuf {
        PathBuf::from(
            self.kn(&["worktree", "add", name], 0)["data"]["path"]
                .as_str()
                .unwrap(),
        )
    }
    /// Reconcile the remote into main through a session, as a person would.
    fn adopt(&self) {
        let s = self.session("adopt");
        self.run(&s, &["pull"], 0);
        self.run(&s, &["worktree", "finish"], 0);
    }
    fn edit(&self, name: &str, change: impl FnOnce(&Path)) {
        let s = self.session(name);
        change(&s);
        self.run(&s, &["commit", "-m", name], 0);
        self.run(&s, &["worktree", "finish"], 0);
    }
    fn remote_state(&self) -> Value {
        self.kn(&["status"], 0)["data"]["remote"].clone()
    }
}
fn code(v: &Value) -> &str {
    v["errors"][0]["code"].as_str().unwrap_or("")
}
fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn files(entries: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(p, b)| ((*p).to_owned(), b.to_vec()))
        .collect()
}

#[test]
fn cloud_commands_require_an_explicit_transfer_mode() {
    let e = Env::new();
    e.kn(&["init"], 0);
    for (args, expected) in [
        (vec!["push"], "MODE_FORBIDS_OPERATION"),
        (vec!["fetch"], "MODE_FORBIDS_OPERATION"),
        (vec!["status", "--refresh"], "MODE_FORBIDS_OPERATION"),
        (vec!["pull"], "SESSION_REQUIRED"),
        (vec!["diff", "--remote"], "REMOTE_NOT_CONFIGURED"),
        (vec!["connect", "drive"], "INVALID_INPUT"),
    ] {
        assert_eq!(code(&e.kn(&args, 3)), expected, "{args:?}");
    }
    let status = e.kn(&["status"], 0)["data"].clone();
    assert_eq!(status["remote"]["state"], "not_configured");
    assert_eq!(status["capabilities"]["push"], false);
    assert_eq!(status["capabilities"]["certified_providers"], json!([]));
    // In desktop_sync a desktop client transfers: kn makes no MCP call at all.
    e.add_remote();
    e.kn(&["mode", "set", "desktop_sync"], 0);
    assert_eq!(code(&e.kn(&["fetch"], 3)), "MODE_FORBIDS_OPERATION");
    assert_eq!(code(&e.kn(&["push"], 3)), "MODE_FORBIDS_OPERATION");
    assert_eq!(e.server.calls("list_children") + e.server.write_calls(), 0);
}

#[test]
fn observed_desktop_mode_integrates_locally_and_never_writes_through_mcp() {
    let e = Env::new();
    fs::write(e.main.join("local.txt"), "local\n").unwrap();
    e.server.put("remote.txt", b"remote\n");
    e.connect("desktop_sync_observed");
    assert_eq!(e.kn(&["fetch"], 0)["data"]["state"], "unknown");
    assert!(!e.main.join("remote.txt").exists());
    let lists = e.server.calls("list_children");
    assert_eq!(code(&e.kn(&["push"], 3)), "MODE_FORBIDS_OPERATION");
    assert_eq!(
        e.server.calls("list_children"),
        lists,
        "push must be refused before contacting the server"
    );
    e.adopt();
    assert_eq!(read(&e.main.join("remote.txt")), "remote\n");
    let remote = e.remote_state();
    assert_eq!(remote["state"], "local_ahead");
    assert_eq!(remote["publisher"], "desktop_client");
    assert_eq!(remote["to_publish"], json!(["local.txt"]));
    // The desktop client uploads on its own; only a new observation confirms it.
    e.server.put("local.txt", b"local\n");
    assert_eq!(e.remote_state()["state"], "local_ahead");
    let refreshed = e.kn(&["status", "--refresh"], 0)["data"]["remote"].clone();
    assert_eq!(refreshed["state"], "synced");
    assert_eq!(refreshed["freshness"], "refreshed");
    assert_eq!(e.server.write_calls(), 0);
}

#[test]
fn fetch_imports_the_complete_remote_without_touching_main() {
    let e = Env::new();
    e.server.set_page_size(1);
    e.server.put("r.txt", b"remote\n");
    e.server.put("docs/nested/deep.md", b"# deep\n");
    e.server.put_native("Informe trimestral");
    e.server.put(".DS_Store", b"junk");
    e.connect("mcp");
    let before = e.kn(&["log"], 0)["data"]["versions"].clone();
    let data = e.kn(&["fetch"], 0)["data"].clone();
    assert_eq!(data["documents"], 2);
    let unmanaged: Vec<(&str, &str)> = data["unmanaged"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| (u["path"].as_str().unwrap(), u["reason"].as_str().unwrap()))
        .collect();
    assert!(unmanaged.contains(&("Informe trimestral", "remote_only")));
    assert!(unmanaged.contains(&(".DS_Store", "ignored_locally")));
    assert!(!e.main.join("r.txt").exists());
    assert_eq!(e.kn(&["log"], 0)["data"]["versions"], before);
    let diff = e.kn(&["diff", "--remote"], 0)["data"].clone();
    let changes: Vec<(&str, &str)> = diff["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["path"].as_str().unwrap(), c["kind"].as_str().unwrap()))
        .collect();
    assert_eq!(
        changes,
        [("docs/nested/deep.md", "deleted"), ("r.txt", "deleted")]
    );
    assert_eq!(e.remote_state()["base_version_id"], Value::Null);
    assert_eq!(code(&e.kn(&["diff", "--base"], 2)), "CONFLICT");
}

#[test]
fn first_reconciliation_conflicts_instead_of_choosing_a_winner() {
    let e = Env::new();
    fs::write(e.main.join("shared.txt"), "local\n").unwrap();
    fs::write(e.main.join("only-local.txt"), "l\n").unwrap();
    e.server.put("shared.txt", b"remote\n");
    e.server.put("only-remote.txt", b"r\n");
    e.connect("mcp");
    assert_eq!(code(&e.kn(&["push"], 2)), "CONFLICT");
    assert_eq!(e.server.write_calls(), 0);
    let s = e.session("reconcile");
    assert_eq!(code(&e.run(&s, &["pull"], 2)), "CONFLICT");
    assert_eq!(read(&e.main.join("shared.txt")), "local\n");
    assert!(!e.main.join("only-remote.txt").exists());
    assert_eq!(
        e.run(&s, &["status"], 0)["data"]["conflicts"],
        json!(["shared.txt"])
    );
    fs::write(s.join("shared.txt"), "merged\n").unwrap();
    git(&s, &["add", "--", "shared.txt"]);
    e.run(&s, &["commit", "-m", "Resolver"], 0);
    e.run(&s, &["worktree", "finish"], 0);
    let remote = e.remote_state();
    assert_eq!(remote["state"], "local_ahead");
    assert_eq!(
        remote["to_publish"],
        json!(["only-local.txt", "shared.txt"])
    );
    e.kn(&["push"], 0);
    assert_eq!(
        e.server.files(),
        files(&[
            ("only-local.txt", b"l\n"),
            ("only-remote.txt", b"r\n"),
            ("shared.txt", b"merged\n")
        ])
    );
    assert_eq!(e.remote_state()["state"], "synced");
}

#[test]
fn push_publishes_a_fixed_commit_with_explicit_deletions_and_verifies_it() {
    let e = Env::new();
    e.server.put("keep.txt", b"v1\n");
    e.server.put("old.txt", b"old\n");
    e.connect("mcp");
    e.adopt();
    assert_eq!(e.remote_state()["state"], "synced");
    let binary: &[u8] = &[0, 159, 146, 150, 255];
    e.edit("changes", |s| {
        fs::write(s.join("keep.txt"), "v2\n").unwrap();
        fs::remove_file(s.join("old.txt")).unwrap();
        fs::create_dir_all(s.join("docs/deep")).unwrap();
        fs::write(s.join("docs/deep/new.md"), "# new\n").unwrap();
        fs::write(s.join("new.bin"), binary).unwrap();
    });
    assert_eq!(
        e.remote_state()["to_publish"],
        json!(["docs/deep/new.md", "keep.txt", "new.bin", "old.txt"])
    );
    let plan = e.kn(&["push", "--dry-run"], 0)["data"].clone();
    let ops: Vec<(&str, &str)> = plan["plan"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["kind"].as_str().unwrap(), o["path"].as_str().unwrap()))
        .collect();
    assert_eq!(
        ops,
        [
            ("create_folder", "docs"),
            ("create_folder", "docs/deep"),
            ("create", "docs/deep/new.md"),
            ("update", "keep.txt"),
            ("create", "new.bin"),
            ("delete", "old.txt"),
        ]
    );
    assert_eq!(plan["requires_allow_deletes"], true);
    assert_eq!(code(&e.kn(&["push"], 3)), "INVALID_INPUT");
    assert_eq!(e.server.write_calls(), 0);
    let pushed = e.kn(&["push", "--allow-deletes"], 0)["data"].clone();
    assert_eq!(pushed["published"], 6);
    assert_eq!(pushed["state"], "synced");
    assert_eq!(
        e.server.files(),
        files(&[
            ("docs/deep/new.md", b"# new\n"),
            ("keep.txt", b"v2\n"),
            ("new.bin", binary)
        ])
    );
    let remote = e.remote_state();
    assert_eq!(remote["state"], "synced");
    assert_eq!(remote["base_version_id"], remote["local_version_id"]);
    assert_eq!(e.kn(&["push"], 0)["data"]["published"], 0);
    assert_eq!(e.server.write_calls(), 6);
}

#[test]
fn a_remote_edit_between_observation_and_write_is_never_overwritten() {
    let e = Env::new();
    e.server.put("a.txt", b"base\n");
    e.connect("mcp");
    e.adopt();
    e.edit("local", |s| fs::write(s.join("a.txt"), "local\n").unwrap());
    e.server
        .mutate_before_write(1, "a.txt", Some(b"concurrent\n"));
    assert_eq!(code(&e.kn(&["push"], 2)), "CONFLICT");
    assert_eq!(e.server.get("a.txt").unwrap(), b"concurrent\n");
    // An unverified publication blocks a mode change until an observation closes it.
    let switch = ["mode", "set", "desktop_sync_observed", "--remote", "drive"];
    assert_eq!(code(&e.kn(&switch, 2)), "CONFLICT");
    let remote = e.kn(&["status", "--refresh"], 0)["data"]["remote"].clone();
    assert_eq!(remote["state"], "conflicted");
    assert_eq!(remote["conflicts"], json!(["a.txt"]));
    assert_eq!(remote["open_publications"], 0);
    let s = e.session("resolve");
    assert_eq!(code(&e.run(&s, &["pull"], 2)), "CONFLICT");
    assert_eq!(read(&e.main.join("a.txt")), "local\n");
    assert_eq!(e.server.get("a.txt").unwrap(), b"concurrent\n");
    e.kn(&switch, 0);
}

#[test]
fn a_lost_write_response_is_verified_by_observation_and_not_repeated() {
    let e = Env::new();
    e.server.put("a.txt", b"base\n");
    e.connect("mcp");
    e.adopt();
    e.edit("local", |s| {
        fs::write(s.join("a.txt"), "edited\n").unwrap();
        fs::write(s.join("new.txt"), "new\n").unwrap();
    });
    e.server.lose_write_response(1);
    assert_eq!(code(&e.kn(&["push"], 1)), "REMOTE_UNAVAILABLE");
    assert_eq!(e.server.get("a.txt").unwrap(), b"edited\n");
    assert!(e.server.get("new.txt").is_none());
    e.kn(&["push"], 0);
    assert_eq!(e.server.get("new.txt").unwrap(), b"new\n");
    assert_eq!(e.server.calls("update_file"), 1);
    assert_eq!(e.server.calls("create_file"), 1);
    assert_eq!(e.remote_state()["state"], "synced");
}

#[test]
fn an_incomplete_observation_is_reported_and_never_infers_deletions() {
    let e = Env::new();
    for name in ["a.txt", "b.txt", "c.txt"] {
        e.server.put(name, b"x\n");
    }
    e.connect("mcp");
    e.adopt();
    e.server.fail_list_call(2);
    assert_eq!(code(&e.kn(&["fetch"], 1)), "REMOTE_INCOMPLETE");
    let remote = e.remote_state();
    assert_eq!(remote["state"], "synced");
    assert_eq!(remote["freshness"], "stored");
    assert_eq!(remote["last_attempt"]["complete"], false);
    assert!(e.main.join("c.txt").exists());
    e.edit("local", |s| fs::write(s.join("a.txt"), "y\n").unwrap());
    e.server.fail_list_call(1);
    assert_eq!(code(&e.kn(&["push"], 1)), "REMOTE_INCOMPLETE");
    e.server.put("A.TXT", b"case\n");
    let failed = e.kn(&["fetch"], 1);
    assert_eq!(code(&failed), "REMOTE_INCOMPLETE");
    assert!(
        failed["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("mayúsculas")
    );
    assert_eq!(e.server.write_calls(), 0);
}

#[test]
fn a_wrong_access_token_requires_authorization() {
    let e = Env::new();
    e.connect("desktop_sync_observed");
    assert_eq!(
        code(&e.run_token(&e.main, &["fetch"], "wrong", 1)),
        "AUTH_REQUIRED"
    );
}

#[test]
fn mode_transitions_are_explicit_and_contact_no_server() {
    let e = Env::new();
    e.kn(&["init"], 0);
    e.add_remote();
    let profile = profile_path();
    let profile = profile.to_str().unwrap();
    let add = |alias: &str, url: &str| {
        e.kn(
            &[
                "remote",
                "add",
                alias,
                url,
                "--profile",
                profile,
                "--root",
                "root",
            ],
            3,
        )
    };
    assert_eq!(code(&add("drive", &e.server.endpoint())), "INVALID_INPUT");
    assert_eq!(code(&add("web", "http://example.com/mcp")), "INVALID_INPUT");
    for args in [
        vec!["mode", "set", "mcp", "--remote", "drive"],
        vec![
            "mode",
            "set",
            "mcp",
            "--remote",
            "nope",
            "--primary-outside-sync",
        ],
        vec!["mode", "set", "desktop_sync", "--remote", "drive"],
        vec!["mode", "set", "drive"],
    ] {
        assert_eq!(code(&e.kn(&args, 3)), "INVALID_INPUT", "{args:?}");
    }
    let set = e.kn(
        &["mode", "set", "desktop_sync_observed", "--remote", "drive"],
        0,
    );
    assert_eq!(set["data"]["publisher"], "desktop_client");
    let inspect = e.kn(&["inspect"], 0)["data"].clone();
    assert_eq!(inspect["origin"]["connection"], "configured");
    assert_eq!(inspect["capabilities"]["pull"], true);
    assert_eq!(inspect["capabilities"]["push"], false);
    assert_eq!(
        code(&e.kn(&["remote", "remove", "drive"], 3)),
        "INVALID_INPUT"
    );
    e.kn(&["mode", "set", "local"], 0);
    e.kn(&["remote", "remove", "drive"], 0);
    assert!(
        e.kn(&["remote", "list"], 0)["data"]["remotes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        e.server.calls("list_children") + e.server.calls("get_item"),
        0
    );

    // A primary inside a known sync-client folder cannot become the MCP publisher.
    let synced = Env::at(&["Google Drive", "documentos"]);
    synced.kn(&["init"], 0);
    synced.add_remote();
    assert_eq!(
        code(&synced.kn(
            &[
                "mode",
                "set",
                "mcp",
                "--remote",
                "drive",
                "--primary-outside-sync"
            ],
            3
        )),
        "MODE_FORBIDS_OPERATION"
    );
}

#[test]
fn verify_reports_capabilities_and_schema_drift_blocks_publication() {
    let e = Env::new();
    e.server.put("a.txt", b"one\n");
    e.connect("mcp");
    let verified = e.kn(&["remote", "verify", "drive"], 0)["data"].clone();
    assert_eq!(verified["observation_ready"], true);
    assert_eq!(verified["publication_ready"], true);
    assert_eq!(verified["certified"], false);
    assert_eq!(verified["protocol_version"], "2025-11-25");
    assert!(
        verified["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["available"] == true)
    );
    e.adopt();
    e.edit("local", |s| fs::write(s.join("a.txt"), "two\n").unwrap());
    e.server.alter_schema("update_file");
    let drifted = e.kn(&["remote", "verify", "drive"], 0)["data"].clone();
    assert_eq!(drifted["publication_ready"], false);
    assert_eq!(code(&e.kn(&["push"], 3)), "PROFILE_MISMATCH");
    assert_eq!(e.server.write_calls(), 0);
    assert_eq!(e.server.get("a.txt").unwrap(), b"one\n");
}
