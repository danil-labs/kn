use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Fixture {
    _tmp: tempfile::TempDir,
    main: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("documents");
        let home = tmp.path().join("engine");
        fs::create_dir(&main).unwrap();
        Self {
            _tmp: tmp,
            main,
            home,
        }
    }
    fn run(&self, path: &Path, args: &[&str], code: i32) -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_kn"))
            .current_dir(path)
            .env("KN_HOME", &self.home)
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
            "JSON diagnostics must not duplicate errors"
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn init(&self) -> Value {
        self.run(&self.main, &["init"], 0)
    }
    fn session(&self, name: &str) -> PathBuf {
        PathBuf::from(
            self.run(&self.main, &["session", "start", name], 0)["data"]["path"]
                .as_str()
                .unwrap(),
        )
    }
}
#[test]
fn documents_sessions_restore_and_publish_locally() {
    let f = Fixture::new();
    fs::write(f.main.join("políticas de reembolso.md"), "original\n").unwrap();
    fs::write(f.main.join(".gitignore"), "private.txt\n").unwrap();
    fs::create_dir(f.main.join(".git")).unwrap();
    fs::write(f.main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let init = f.init();
    let first = init["data"]["initial_version_id"].as_str().unwrap();
    assert_eq!(
        f.init()["data"]["workspace_id"],
        init["data"]["workspace_id"]
    );
    f.run(&f.main, &["snapshot"], 3);
    let session = f.session("policy");
    let gitfile = fs::read(session.join(".git")).unwrap();
    fs::write(session.join("private.txt"), "keep").unwrap();
    fs::write(session.join("políticas de reembolso.md"), "new\n").unwrap();
    fs::write(session.join("extra.txt"), "extra").unwrap();
    assert_eq!(
        f.run(&session, &["status"], 0)["data"]["local_changes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    f.run(&session, &["diff", "--patch"], 0);
    assert_eq!(
        f.run(
            &session,
            &["snapshot", "-m", "restore is just message text"],
            0
        )["data"]["created"],
        true
    );
    assert_eq!(f.run(&session, &["snapshot"], 0)["data"]["created"], false);
    let h = f.run(&session, &["history", "--limit", "1"], 0);
    assert_eq!(h["data"]["versions"][0]["reason"], "manual_snapshot");
    assert_eq!(h["data"]["next_offset"], 1);
    fs::write(session.join("unsaved.txt"), "recoverable").unwrap();
    f.run(&session, &["restore", first], 0);
    assert_eq!(
        fs::read_to_string(session.join("políticas de reembolso.md")).unwrap(),
        "original\n"
    );
    assert_eq!(
        fs::read_to_string(session.join("private.txt")).unwrap(),
        "keep"
    );
    assert_eq!(fs::read(session.join(".git")).unwrap(), gitfile);
    assert!(!session.join("extra.txt").exists());
    fs::write(session.join("políticas de reembolso.md"), "approved\n").unwrap();
    f.run(&session, &["snapshot"], 0);
    assert_eq!(
        fs::read_to_string(f.main.join("políticas de reembolso.md")).unwrap(),
        "original\n"
    );
    f.run(&session, &["session", "finish"], 0);
    assert_eq!(
        fs::read_to_string(f.main.join("políticas de reembolso.md")).unwrap(),
        "approved\n"
    );
    assert_eq!(
        fs::read_to_string(f.main.join(".git/HEAD")).unwrap(),
        "ref: refs/heads/main\n"
    );
}
#[test]
fn concurrent_sessions_require_update_and_preserve_main_on_conflict() {
    let f = Fixture::new();
    fs::write(f.main.join("a.txt"), "base\n").unwrap();
    f.init();
    let a = f.session("a");
    let b = f.session("b");
    fs::write(a.join("a.txt"), "a\n").unwrap();
    f.run(&a, &["snapshot"], 0);
    fs::write(b.join("a.txt"), "b\n").unwrap();
    f.run(&b, &["snapshot"], 0);
    f.run(&a, &["session", "finish"], 0);
    f.run(&b, &["session", "finish"], 2);
    f.run(&b, &["session", "update"], 2);
    assert_eq!(fs::read_to_string(f.main.join("a.txt")).unwrap(), "a\n");
    f.run(&b, &["session", "finish"], 2);
}
#[test]
fn special_names_ignores_empty_folders_and_read_only_queries() {
    let f = Fixture::new();
    f.init();
    let s = f.session("names");
    let mut names = vec!["políticas de reembolso.md"];
    if !cfg!(windows) {
        names.extend([
            " leading trailing ",
            "arrow -> text",
            "tab\tname",
            "line\nname",
        ]);
    }
    for name in &names {
        fs::write(s.join(name), "document").unwrap();
    }
    for name in [".DS_Store", "Thumbs.db", "~$file.docx"] {
        fs::write(s.join(name), "junk").unwrap();
    }
    fs::create_dir(s.join("empty")).unwrap();
    fs::create_dir_all(s.join("nested/.kn")).unwrap();
    fs::write(s.join("nested/.kn/secret"), "ignored").unwrap();
    let before = fs::read(s.join(".kn/config.json")).unwrap();
    let result = f.run(&s, &["status"], 0);
    let paths: Vec<_> = result["data"]["local_changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["path"].as_str().unwrap())
        .collect();
    for name in &names {
        assert!(paths.contains(name));
    }
    assert_eq!(paths.len(), names.len());
    f.run(&s, &["diff"], 0);
    assert!(!s.join("empty/.knkeep").exists());
    assert_eq!(fs::read(s.join(".kn/config.json")).unwrap(), before);
    f.run(&s, &["snapshot"], 0);
    assert_eq!(
        f.run(&s, &["history"], 0)["data"]["versions"][0]["changed_document_count"],
        names.len()
    );
}
#[test]
fn copies_moves_locks_and_invalid_input() {
    use fs2::FileExt;
    let f = Fixture::new();
    let init = f.init();
    let copy = f._tmp.path().join("copy");
    fs::create_dir_all(copy.join(".kn")).unwrap();
    fs::copy(f.main.join(".kn/config.json"), copy.join(".kn/config.json")).unwrap();
    assert_eq!(
        f.run(&copy, &["status"], 1)["errors"][0]["code"],
        "WORKSPACE_COPIED"
    );
    assert_ne!(
        f.run(&copy, &["init", "--fresh"], 0)["data"]["workspace_id"],
        init["data"]["workspace_id"]
    );
    let moved = f._tmp.path().join("moved");
    fs::rename(&f.main, &moved).unwrap();
    f.run(&moved, &["status"], 0);
    f.run(&moved, &["init"], 0);
    let s = PathBuf::from(
        f.run(&moved, &["session", "start", "new"], 0)["data"]["path"]
            .as_str()
            .unwrap(),
    );
    for id in ["HEAD", "HEAD~1", "--helpful", "v_../../etc.", "v_abcdef"] {
        f.run(&s, &["restore", id], 3);
    }
    f.run(&s, &["restore", "--help"], 0);
    f.run(&s, &["snapshot", "--typo"], 3);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            f.home
                .join("repos")
                .join(init["data"]["workspace_id"].as_str().unwrap())
                .join("kn.lock"),
        )
        .unwrap();
    lock.lock_exclusive().unwrap();
    assert_eq!(
        f.run(&s, &["status"], 1)["errors"][0]["code"],
        "WORKSPACE_BUSY"
    );
}
#[test]
fn global_git_configuration_is_isolated() {
    let f = Fixture::new();
    let config = f._tmp.path().join("hostile.gitconfig");
    fs::write(&config, "[commit]\n gpgsign = true\n[core]\n autocrlf = true\n hooksPath = /not-a-real-hook-directory\n").unwrap();
    fs::write(f.main.join("bytes.txt"), b"one\r\ntwo\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kn"))
        .current_dir(&f.main)
        .env("KN_HOME", &f.home)
        .env("GIT_CONFIG_GLOBAL", config)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "true")
        .args(["init", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let s = f.session("bytes");
    assert_eq!(fs::read(s.join("bytes.txt")).unwrap(), b"one\r\ntwo\n");
}
#[test]
fn cloud_capabilities_remain_honest() {
    let f = Fixture::new();
    for args in [
        vec!["connect", "drive"],
        vec!["push"],
        vec!["pull"],
        vec!["status", "--refresh"],
        vec!["diff", "--base"],
    ] {
        assert_eq!(f.run(&f.main, &args, 3)["status"], "unsupported");
    }
}
#[cfg(unix)]
#[test]
fn unsafe_symlink_is_reported_but_cannot_be_committed() {
    let f = Fixture::new();
    f.init();
    let s = f.session("links");
    std::os::unix::fs::symlink(f._tmp.path(), s.join("outside")).unwrap();
    assert_eq!(
        f.run(&s, &["status"], 0)["data"]["unsafe_paths"][0],
        "outside"
    );
    f.run(&s, &["snapshot"], 1);
}

#[test]
fn restore_does_not_overwrite_an_ignored_obstruction() {
    let f = Fixture::new();
    fs::write(f.main.join("protected.txt"), "old tracked document").unwrap();
    let initial = f.init();
    let id = initial["data"]["initial_version_id"].as_str().unwrap();
    let s = f.session("obstruction");
    fs::remove_file(s.join("protected.txt")).unwrap();
    fs::write(s.join(".gitignore"), "protected.txt\n").unwrap();
    f.run(&s, &["snapshot"], 0);
    fs::write(s.join("protected.txt"), "private unversioned document").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kn"))
        .current_dir(&s)
        .env("KN_HOME", &f.home)
        .args(["restore", id, "--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "restore must reject an ignored obstruction"
    );
    assert_eq!(
        fs::read_to_string(s.join("protected.txt")).unwrap(),
        "private unversioned document"
    );
}

#[test]
fn independent_session_changes_merge_via_git() {
    let f = Fixture::new();
    f.init();
    let a = f.session("a");
    let b = f.session("b");
    fs::write(a.join("a.txt"), "a").unwrap();
    f.run(&a, &["snapshot"], 0);
    fs::write(b.join("b.txt"), "b").unwrap();
    f.run(&b, &["snapshot"], 0);
    f.run(&a, &["session", "finish"], 0);
    f.run(&b, &["session", "update"], 0);
    f.run(&b, &["session", "finish"], 0);
    assert_eq!(fs::read_to_string(f.main.join("a.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(f.main.join("b.txt")).unwrap(), "b");
}

#[test]
fn manual_documents_become_the_next_agent_sessions_baseline() {
    let f = Fixture::new();
    f.init();
    fs::write(f.main.join("manual.txt"), "uploaded by a person\n").unwrap();
    let status = f.run(&f.main, &["status"], 0);
    assert_eq!(status["data"]["local_changes"][0]["path"], "manual.txt");
    assert!(
        f.run(&f.main, &["history"], 0)["data"]["versions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let s = f.session("agent");
    assert_eq!(
        fs::read_to_string(s.join("manual.txt")).unwrap(),
        "uploaded by a person\n"
    );
    assert_eq!(
        fs::read_to_string(f.main.join("manual.txt")).unwrap(),
        "uploaded by a person\n"
    );
    let h = f.run(&s, &["history"], 0);
    assert_eq!(h["data"]["versions"][0]["reason"], "external_observation");
    let first = h["data"]["versions"][0]["id"].clone();
    f.session("another");
    assert_eq!(
        f.run(&f.main, &["history"], 0)["data"]["versions"][0]["id"],
        first
    );
}

#[test]
fn manual_changes_during_agent_work_are_preserved_and_integrated() {
    let f = Fixture::new();
    fs::write(f.main.join("manual.txt"), "original\n").unwrap();
    f.init();
    let s = f.session("agent");
    fs::write(s.join("agent.txt"), "agent draft\n").unwrap();
    f.run(&s, &["snapshot"], 0);
    fs::write(f.main.join("manual.txt"), "edited outside kn\n").unwrap();
    fs::write(f.main.join("new.txt"), "new manual document\n").unwrap();
    f.run(&s, &["session", "finish"], 2);
    assert!(!f.main.join("agent.txt").exists());
    f.run(&s, &["session", "update"], 0);
    f.run(&s, &["session", "finish"], 0);
    assert_eq!(
        fs::read_to_string(f.main.join("manual.txt")).unwrap(),
        "edited outside kn\n"
    );
    assert_eq!(
        fs::read_to_string(f.main.join("new.txt")).unwrap(),
        "new manual document\n"
    );
    assert_eq!(
        fs::read_to_string(f.main.join("agent.txt")).unwrap(),
        "agent draft\n"
    );
}

#[test]
fn manual_edits_conflict_only_inside_the_agent_session() {
    let f = Fixture::new();
    fs::write(f.main.join("shared.txt"), "original\n").unwrap();
    f.init();
    let s = f.session("agent");
    fs::write(s.join("shared.txt"), "agent edit\n").unwrap();
    f.run(&s, &["snapshot"], 0);
    fs::write(f.main.join("shared.txt"), "manual edit\n").unwrap();
    f.run(&s, &["session", "update"], 2);
    assert_eq!(
        fs::read_to_string(f.main.join("shared.txt")).unwrap(),
        "manual edit\n"
    );
    assert!(
        !f.run(&s, &["status"], 0)["data"]["conflicts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn inspect_unmanaged_folders_does_not_initialize_or_classify_code() {
    let f = Fixture::new();
    for git in [false, true] {
        if git {
            fs::create_dir(f.main.join(".git")).unwrap();
        }
        let out = f.run(&f.main, &["inspect"], 0);
        assert_eq!(out["data"]["managed"], false);
        assert_eq!(out["data"]["kind"], Value::Null);
        assert_eq!(out["data"]["origin"], Value::Null);
        assert_eq!(out["data"]["capabilities"]["sessions"], false);
        assert!(!f.main.join(".kn").exists());
        assert!(!f.home.exists());
    }
}

#[test]
fn inspect_identifies_sessions_from_subdirectories_and_explicit_paths() {
    let f = Fixture::new();
    let init = f.init();
    let s = f.session("agent");
    fs::create_dir(s.join("subfolder")).unwrap();
    let primary = f.run(&f.main, &["inspect"], 0);
    let session = f.run(
        &f.main,
        &["inspect", "--path", s.join("subfolder").to_str().unwrap()],
        0,
    );
    for info in [&primary["data"], &session["data"]] {
        assert_eq!(info["managed"], true);
        assert_eq!(info["kind"], "documents");
        assert_eq!(info["workspace_id"], init["data"]["workspace_id"]);
        assert_eq!(
            info["primary_root"],
            fs::canonicalize(&f.main).unwrap().to_str().unwrap()
        );
        assert_eq!(info["origin"]["kind"], "unknown");
        assert_eq!(info["origin"]["sharing"], "unknown");
        assert_eq!(info["origin"]["connection"], "not_connected");
        assert_eq!(info["capabilities"]["push"], false);
        assert_eq!(info["version_id"], init["data"]["initial_version_id"]);
        assert!(info["commit_id"].as_str().unwrap().len() >= 40);
    }
    assert_eq!(primary["data"]["role"], "primary");
    assert_eq!(session["data"]["role"], "session");
    assert_eq!(session["data"]["session"], "agent");
    assert_eq!(
        session["data"]["root"],
        fs::canonicalize(&s).unwrap().to_str().unwrap()
    );
    assert!(s.join(".git").is_file());
    // A separate repository within the document folder is not a kn session.
    fs::create_dir_all(f.main.join("code/.git")).unwrap();
    assert_eq!(
        f.run(&f.main.join("code"), &["inspect"], 0)["data"]["managed"],
        false
    );
}

#[test]
fn inspect_is_read_only_even_after_manual_changes_and_a_move() {
    fn tree(root: &Path) -> std::collections::BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
        let mut out = std::collections::BTreeMap::new();
        fn walk(
            root: &Path,
            p: &Path,
            out: &mut std::collections::BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>,
        ) {
            for e in fs::read_dir(p).unwrap() {
                let e = e.unwrap();
                let meta = e.metadata().unwrap();
                let bytes = if meta.is_dir() {
                    vec![]
                } else {
                    fs::read(e.path()).unwrap()
                };
                out.insert(
                    e.path().strip_prefix(root).unwrap().to_path_buf(),
                    (bytes, meta.modified().unwrap()),
                );
                if meta.is_dir() {
                    walk(root, &e.path(), out);
                }
            }
        }
        walk(root, root, &mut out);
        out
    }
    let f = Fixture::new();
    f.init();
    let moved = f._tmp.path().join("moved");
    fs::rename(&f.main, &moved).unwrap();
    fs::write(moved.join("manual.txt"), "not yet observed").unwrap();
    let before = tree(f._tmp.path());
    let out = f.run(&moved, &["inspect"], 0);
    assert_eq!(
        out["data"]["primary_root"],
        fs::canonicalize(&moved).unwrap().to_str().unwrap()
    );
    assert_eq!(tree(f._tmp.path()), before);
}

#[test]
fn inspect_does_not_hide_damaged_or_copied_workspaces() {
    let f = Fixture::new();
    f.init();
    let copy = f._tmp.path().join("copy");
    fs::create_dir_all(copy.join(".kn")).unwrap();
    fs::copy(f.main.join(".kn/config.json"), copy.join(".kn/config.json")).unwrap();
    assert_eq!(
        f.run(&copy, &["inspect"], 1)["errors"][0]["code"],
        "WORKSPACE_COPIED"
    );
    fs::write(f.main.join(".kn/config.json"), "broken").unwrap();
    assert_eq!(
        f.run(&f.main, &["inspect"], 1)["errors"][0]["code"],
        "INVALID_STATE"
    );
    assert_eq!(
        f.run(&f.main, &["inspect", "--path", "absent"], 1)["errors"][0]["code"],
        "IO_ERROR"
    );
    fs::write(f.main.join("file.txt"), "document").unwrap();
    assert_eq!(
        f.run(&f.main, &["inspect", "--path", "file.txt"], 3)["errors"][0]["code"],
        "INVALID_INPUT"
    );
}

#[test]
fn inspect_waits_for_writers_without_creating_a_missing_lock() {
    use fs2::FileExt;
    let f = Fixture::new();
    let init = f.init();
    let lockpath = f
        .home
        .join("repos")
        .join(init["data"]["workspace_id"].as_str().unwrap())
        .join("kn.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lockpath)
        .unwrap();
    lock.lock_exclusive().unwrap();
    assert_eq!(
        f.run(&f.main, &["inspect"], 1)["errors"][0]["code"],
        "WORKSPACE_BUSY"
    );
    drop(lock);
    fs::remove_file(&lockpath).unwrap();
    assert_eq!(
        f.run(&f.main, &["inspect"], 1)["errors"][0]["code"],
        "IO_ERROR"
    );
    assert!(!lockpath.exists());
}

impl Fixture {
    fn raw(&self, path: &Path, args: &[&str], code: i32) -> Vec<u8> {
        let out = Command::new(env!("CARGO_BIN_EXE_kn"))
            .current_dir(path)
            .env("KN_HOME", &self.home)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(code),
            "{args:?}: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if code == 0 {
            assert!(out.stderr.is_empty());
        }
        out.stdout
    }
}

#[test]
fn git_named_queries_and_aliases_support_service_integration() {
    let f = Fixture::new();
    assert!(
        f.raw(&f.main, &["rev-parse", "--is-inside-work-tree"], 1)
            .is_empty()
    );
    f.init();
    let s = PathBuf::from(
        f.run(&f.main, &["worktree", "add", "agent"], 0)["data"]["path"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir(s.join("sub")).unwrap();
    assert_eq!(
        f.raw(&s, &["rev-parse", "--is-inside-work-tree"], 0),
        b"true\n"
    );
    let root = f.raw(
        &f.main,
        &[
            "-C",
            s.join("sub").to_str().unwrap(),
            "rev-parse",
            "--show-toplevel",
        ],
        0,
    );
    assert_eq!(
        String::from_utf8(root).unwrap().trim(),
        fs::canonicalize(&s).unwrap().to_str().unwrap()
    );
    let common = f.raw(&s, &["rev-parse", "--git-common-dir"], 0);
    let own = f.raw(&s, &["rev-parse", "--git-dir"], 0);
    assert_ne!(common, own);
    assert_eq!(f.raw(&f.main, &["rev-parse", "--git-dir"], 0), common);
    fs::write(s.join("políticas.md"), "one\n").unwrap();
    assert_eq!(
        f.raw(&s, &["status", "--porcelain", "-z"], 0),
        "?? políticas.md\0".as_bytes()
    );
    f.run(&s, &["commit", "-m", "New document"], 0);
    assert!(f.raw(&s, &["status", "--porcelain"], 0).is_empty());
    assert_eq!(
        f.run(&s, &["log"], 0)["data"]["versions"],
        f.run(&s, &["history"], 0)["data"]["versions"]
    );
    let head = String::from_utf8(f.raw(&s, &["rev-parse", "HEAD"], 0)).unwrap();
    assert_eq!(
        &head.trim()[..12],
        &f.run(&s, &["log"], 0)["data"]["versions"][0]["id"]
            .as_str()
            .unwrap()[2..]
    );
    let listed = f.raw(&f.main, &["worktree", "list", "--porcelain", "-z"], 0);
    assert_eq!(
        listed,
        f.raw(&s, &["worktree", "list", "--porcelain", "-z"], 0)
    );
    assert!(
        listed
            .windows(b"branch refs/heads/main\0".len())
            .any(|w| w == b"branch refs/heads/main\0")
    );
    assert!(
        listed
            .windows(b"branch refs/heads/sessions/agent\0".len())
            .any(|w| w == b"branch refs/heads/sessions/agent\0")
    );
    assert!(
        listed
            .windows(
                format!(
                    "worktree {}\0",
                    fs::canonicalize(&f.main).unwrap().display()
                )
                .len()
            )
            .any(|w| w
                == format!(
                    "worktree {}\0",
                    fs::canonicalize(&f.main).unwrap().display()
                )
                .as_bytes())
    );
    // Never silently put JSON around a byte protocol.
    f.run(&s, &["status", "--porcelain"], 3);
    f.run(&s, &["worktree", "list", "--porcelain"], 3);
    f.run(&s, &["rev-parse", "HEAD"], 3);
    assert!(
        f.raw(&s, &["rev-parse", "--show-toplevel", "HEAD"], 3)
            .is_empty()
    );
}

#[test]
fn old_sessions_cannot_publish_to_a_reinitialized_primary() {
    let f = Fixture::new();
    f.init();
    let old = f.session("old");
    fs::write(old.join("draft.txt"), "keep in old session").unwrap();
    f.run(&old, &["commit"], 0);
    let fresh = f.run(&f.main, &["init", "--fresh"], 0);
    for args in [
        vec!["worktree", "add", "new"],
        vec!["worktree", "update"],
        vec!["worktree", "finish"],
    ] {
        assert_eq!(
            f.run(&old, &args, 1)["errors"][0]["code"],
            "WORKSPACE_IDENTITY_CHANGED"
        );
    }
    assert!(!f.main.join("draft.txt").exists());
    assert_eq!(
        fs::read_to_string(old.join("draft.txt")).unwrap(),
        "keep in old session"
    );
    assert_eq!(
        f.run(&f.main, &["inspect"], 0)["data"]["workspace_id"],
        fresh["data"]["workspace_id"]
    );
}

#[cfg(unix)]
#[test]
fn absolute_symlinks_cannot_escape_a_new_sessions_isolation() {
    let f = Fixture::new();
    fs::write(f.main.join("original.txt"), "primary").unwrap();
    std::os::unix::fs::symlink(f.main.join("original.txt"), f.main.join("alias.txt")).unwrap();
    assert_eq!(
        f.run(&f.main, &["init"], 1)["errors"][0]["code"],
        "UNSAFE_PATH"
    );
    fs::remove_file(f.main.join("alias.txt")).unwrap();
    std::os::unix::fs::symlink("original.txt", f.main.join("alias.txt")).unwrap();
    f.init();
    let s = f.session("relative");
    fs::write(s.join("alias.txt"), "session only").unwrap();
    assert_eq!(
        fs::read_to_string(f.main.join("original.txt")).unwrap(),
        "primary"
    );
    assert_eq!(
        fs::read_to_string(s.join("original.txt")).unwrap(),
        "session only"
    );
}
