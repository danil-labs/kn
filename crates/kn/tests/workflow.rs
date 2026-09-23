use serde_json::{Value, json};
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
        self.run_env(path, args, code, &[])
    }
    fn run_env(&self, path: &Path, args: &[&str], code: i32, env: &[(&str, &str)]) -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_kn"))
            .current_dir(path)
            .env("KN_HOME", &self.home)
            .envs(env.iter().copied())
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
fn session_path(value: &Value) -> PathBuf {
    PathBuf::from(value["data"]["path"].as_str().unwrap())
}
fn canonical(path: &Path) -> String {
    fs::canonicalize(path).unwrap().to_str().unwrap().to_owned()
}
fn toplevel(path: &Path) -> String {
    format!("{}\n", canonical(path))
}
impl Fixture {
    fn registry(&self) -> Value {
        match fs::read(self.home.join("roots.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap(),
            Err(_) => json!({"schema_version": 1, "roots": {}}),
        }
    }
    fn registered(&self, root: &Path) -> Value {
        self.registry()["roots"][canonical(root)].clone()
    }
    /// Deja `root` como la dejaba un kn anterior al registro: marcador y lock en
    /// `.kn`, y ninguna entrada en roots.json.
    fn make_legacy(&self, root: &Path, id: &Value) {
        fs::create_dir_all(root.join(".kn")).unwrap();
        let config = json!({"schema_version": 2, "workspace_id": id, "session": null});
        fs::write(root.join(".kn/config.json"), config.to_string()).unwrap();
        fs::write(root.join(".kn/kn.lock"), "").unwrap();
        let mut registry = self.registry();
        registry["roots"]
            .as_object_mut()
            .unwrap()
            .remove(&canonical(root));
        fs::write(self.home.join("roots.json"), registry.to_string()).unwrap();
    }
    fn legacy(&self) -> Value {
        let init = self.init();
        self.make_legacy(&self.main, &init["data"]["workspace_id"]);
        init
    }
}
#[test]
fn git_resolution_follows_kn_git_and_reports_missing_git() {
    let f = Fixture::new();
    fs::write(f.main.join("acta.md"), "uno\n").unwrap();
    let no_git = f._tmp.path().join("sin-git");
    fs::create_dir(&no_git).unwrap();
    let real_git = kn_core::git::executable().unwrap();
    let init = |kn_git: Option<&Path>, path: &std::ffi::OsStr| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_kn"));
        cmd.current_dir(&f.main)
            .env("KN_HOME", &f.home)
            .env("PATH", path)
            .env_remove("KN_GIT")
            .args(["init", "--json"]);
        if let Some(git) = kn_git {
            cmd.env("KN_GIT", git);
        }
        let out = cmd.output().unwrap();
        let body: Value = serde_json::from_slice(&out.stdout).unwrap();
        (out.status.code(), body)
    };

    let (code, missing) = init(None, no_git.as_os_str());
    assert_eq!(code, Some(1), "{missing}");
    assert_eq!(missing["errors"][0]["code"], "GIT_MISSING");
    assert!(
        missing["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("KN_GIT")
    );
    assert!(
        !f.main.join(".kn").exists(),
        "sin Git no se toca la carpeta"
    );
    assert!(!f.home.exists(), "sin Git no se crea KN_HOME");

    let wrong = f._tmp.path().join("no-existe").join("git");
    let full_path = std::env::var_os("PATH").unwrap();
    let (code, invalid) = init(Some(&wrong), &full_path);
    assert_eq!(code, Some(1), "{invalid}");
    assert_eq!(invalid["errors"][0]["code"], "GIT_MISSING");
    assert!(
        invalid["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains(&wrong.display().to_string())
    );
    assert!(!f.main.join(".kn").exists());

    let (code, done) = init(Some(real_git), no_git.as_os_str());
    assert_eq!(code, Some(0), "{done}");
    assert_eq!(done["status"], "ok");
    assert!(!f.main.join(".kn").exists());
    assert_eq!(f.registered(&f.main), done["data"]["workspace_id"]);
}
#[test]
fn cloud_only_documents_wait_without_blocking_or_being_deleted() {
    let f = Fixture::new();
    let remoto = "anexos/[v2] informe final.pdf";
    fs::write(f.main.join("acta.md"), "local\n").unwrap();
    fs::create_dir(f.main.join("anexos")).unwrap();
    fs::write(f.main.join(remoto), "contenido remoto\n").unwrap();
    let nube = [("KN_TEST_CLOUD_ONLY", remoto)];

    let init = f.run_env(&f.main, &["init"], 0, &nube);
    assert_eq!(init["data"]["cloud_only"][0], remoto);
    let status = f.run_env(&f.main, &["status"], 0, &nube);
    assert_eq!(status["data"]["clean"], true);
    assert_eq!(status["data"]["cloud_only"][0], remoto);

    let choque = session_path(&f.run_env(&f.main, &["session", "start", "choque"], 0, &nube));
    assert!(choque.join("acta.md").is_file());
    assert!(
        !choque.join(remoto).exists(),
        "lo que sigue en la nube no entra en la sesión"
    );
    fs::create_dir_all(choque.join("anexos")).unwrap();
    fs::write(choque.join(remoto), "del agente\n").unwrap();
    f.run(&choque, &["snapshot"], 0);
    f.run_env(&choque, &["session", "finish"], 2, &nube);
    assert_eq!(
        fs::read_to_string(f.main.join(remoto)).unwrap(),
        "contenido remoto\n"
    );

    let agente = session_path(&f.run_env(&f.main, &["session", "start", "agente"], 0, &nube));
    fs::write(agente.join("respuesta.md"), "del agente\n").unwrap();
    f.run(&agente, &["snapshot"], 0);
    f.run_env(&agente, &["session", "finish"], 0, &nube);
    assert_eq!(
        fs::read_to_string(f.main.join(remoto)).unwrap(),
        "contenido remoto\n"
    );
    assert_eq!(
        fs::read_to_string(f.main.join("respuesta.md")).unwrap(),
        "del agente\n"
    );

    let descargado = f.session("descargado");
    assert_eq!(
        fs::read_to_string(descargado.join(remoto)).unwrap(),
        "contenido remoto\n",
        "ya descargado, la siguiente observación lo versiona"
    );
}
const NUBE: &str = "KN_TEST_CLOUD_ONLY";
fn cloud_record(f: &Fixture, init: &Value) -> PathBuf {
    f.home
        .join("repos")
        .join(init["data"]["workspace_id"].as_str().unwrap())
        .join("cloud-pending.json")
}
fn version_files(f: &Fixture, init: &Value, version: &Value) -> Vec<String> {
    let out = Command::new(kn_core::git::executable().unwrap())
        .current_dir(f._tmp.path())
        .env(
            "GIT_DIR",
            f.home
                .join("repos")
                .join(init["data"]["workspace_id"].as_str().unwrap()),
        )
        .args([
            "show",
            "--name-only",
            "-z",
            "--format=",
            &version.as_str().unwrap()[2..],
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    kn_core::git::nul_paths(&out.stdout).unwrap()
}
#[test]
fn cloud_downloads_are_reported_without_writing() {
    let f = Fixture::new();
    let remoto = "anexos/[v2] informe final.pdf";
    fs::write(f.main.join("acta.md"), "local\n").unwrap();
    fs::create_dir(f.main.join("anexos")).unwrap();
    fs::write(f.main.join(remoto), "contenido remoto\n").unwrap();
    let init = f.run_env(&f.main, &["init"], 0, &[(NUBE, remoto)]);
    let record = cloud_record(&f, &init);
    let before = fs::read(&record).unwrap();
    let pending: Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(pending["cloud_only"], json!([remoto]));
    assert!(!record.starts_with(&f.main));

    let waiting = f.run_env(&f.main, &["status"], 0, &[(NUBE, remoto)]);
    assert_eq!(waiting["data"]["cloud_only"], json!([remoto]));
    assert_eq!(
        waiting["data"]["downloaded_since_last_observation"],
        json!([])
    );

    let status = f.run(&f.main, &["status"], 0);
    assert_eq!(
        status["data"]["downloaded_since_last_observation"],
        json!([remoto])
    );
    assert_eq!(status["data"]["cloud_only"], json!([]));
    assert_eq!(
        fs::read(&record).unwrap(),
        before,
        "status solo lee el registro"
    );
}
#[test]
fn cloud_downloads_have_their_own_version() {
    let f = Fixture::new();
    let (uno, dos) = ("uno.pdf", "anexos/dos.pdf");
    fs::write(f.main.join("acta.md"), "original\n").unwrap();
    fs::create_dir(f.main.join("anexos")).unwrap();
    fs::write(f.main.join(uno), "uno\n").unwrap();
    fs::write(f.main.join(dos), "dos\n").unwrap();
    let init = f.run_env(&f.main, &["init"], 0, &[(NUBE, "uno.pdf\nanexos/dos.pdf")]);
    let record = cloud_record(&f, &init);

    // Solo se descargó uno: una sola versión, cloud_download.
    let solo = f.run_env(&f.main, &["worktree", "add", "solo"], 0, &[(NUBE, dos)]);
    assert_eq!(
        solo["data"]["downloaded_since_last_observation"],
        json!([uno])
    );
    let s = session_path(&solo);
    let log = f.run(&s, &["log"], 0)["data"]["versions"].clone();
    assert_eq!(log.as_array().unwrap().len(), 2, "{log}");
    assert_eq!(log[0]["reason"], "cloud_download");
    assert_eq!(log[0]["message"], "Documentos descargados de la nube");
    assert_eq!(version_files(&f, &init, &log[0]["id"]), [uno]);
    assert_eq!(log[1]["reason"], "init");

    let recorded = fs::read(&record).unwrap();
    fs::write(s.join("borrador.md"), "del agente\n").unwrap();
    f.run(&s, &["commit"], 0);
    assert_eq!(
        fs::read(&record).unwrap(),
        recorded,
        "una sesión no escribe el registro"
    );

    // El otro se descargó y alguien editó el acta: dos versiones separadas.
    fs::write(f.main.join("acta.md"), "editada\n").unwrap();
    let mixta = f.run(&f.main, &["worktree", "add", "mixta"], 0);
    assert_eq!(
        mixta["data"]["downloaded_since_last_observation"],
        json!([dos])
    );
    let log = f.run(&session_path(&mixta), &["log"], 0)["data"]["versions"].clone();
    let reasons: Vec<_> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
        [
            "external_observation",
            "cloud_download",
            "cloud_download",
            "init"
        ]
    );
    assert_eq!(version_files(&f, &init, &log[0]["id"]), ["acta.md"]);
    assert_eq!(version_files(&f, &init, &log[1]["id"]), [dos]);
    let status = f.run(&f.main, &["status"], 0);
    assert_eq!(status["data"]["clean"], true);
    assert_eq!(
        status["data"]["downloaded_since_last_observation"],
        json!([])
    );
}
#[test]
fn cloud_fetch_reads_pending_documents_without_versions() {
    let f = Fixture::new();
    let remoto = "anexos/informe final.pdf";
    let contenido = "contenido remoto\n";
    fs::write(f.main.join("acta.md"), "local\n").unwrap();
    fs::create_dir(f.main.join("anexos")).unwrap();
    fs::write(f.main.join(remoto), contenido).unwrap();
    let nube = [(NUBE, remoto)];
    f.run_env(&f.main, &["init"], 0, &nube);
    let head = f.raw(&f.main, &["rev-parse", "HEAD"], 0);

    let budget =
        f.run_env(&f.main, &["cloud", "fetch", "--max-bytes", "0"], 0, &nube)["data"].clone();
    assert_eq!(budget["skipped_budget"], json!([remoto]));
    assert_eq!(budget["fetched"], json!([]));
    assert_eq!(budget["bytes_fetched"], 0);

    let done = f.run_env(
        &f.main,
        &["cloud", "fetch", "--timeout-secs", "5"],
        0,
        &nube,
    )["data"]
        .clone();
    assert_eq!(done["fetched"], json!([remoto]));
    assert_eq!(done["failed"], json!([]));
    assert_eq!(done["skipped_budget"], json!([]));
    assert_eq!(done["bytes_fetched"], contenido.len());
    assert_eq!(done["remaining"], json!([remoto]), "la simulación sigue");

    let s = session_path(&f.run_env(&f.main, &["worktree", "add", "agente"], 0, &nube));
    assert_eq!(
        f.run_env(&s, &["cloud", "fetch"], 0, &nube)["data"]["fetched"],
        json!([remoto]),
        "desde una sesión descarga en la principal"
    );
    let human = Command::new(env!("CARGO_BIN_EXE_kn"))
        .current_dir(&f.main)
        .env("KN_HOME", &f.home)
        .env(NUBE, remoto)
        .args(["cloud", "fetch"])
        .output()
        .unwrap();
    assert!(human.status.success());
    assert_eq!(String::from_utf8(human.stdout).unwrap().lines().count(), 1);

    f.run(&f.main, &["cloud", "fetch", "--timeout-secs", "0"], 3);
    assert_eq!(
        f.run(f._tmp.path(), &["cloud", "fetch"], 1)["errors"][0]["code"],
        "NOT_A_WORKSPACE"
    );
    assert_eq!(f.raw(&f.main, &["rev-parse", "HEAD"], 0), head);
}
#[cfg(unix)]
#[test]
fn a_failed_init_leaves_no_orphan_history() {
    let f = Fixture::new();
    std::os::unix::fs::symlink(f._tmp.path(), f.main.join("fuera")).unwrap();
    f.run(&f.main, &["init"], 1);
    let repos = f.home.join("repos");
    assert!(
        !repos.exists() || fs::read_dir(&repos).unwrap().next().is_none(),
        "un init fallido no deja historial en KN_HOME"
    );
    assert_eq!(f.registry()["roots"], json!({}), "ni lo registra");
    fs::remove_file(f.main.join("fuera")).unwrap();
    let init = f.init();
    assert_eq!(fs::read_dir(&repos).unwrap().count(), 1);
    assert_eq!(f.registered(&f.main), init["data"]["workspace_id"]);
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
    assert_eq!(init["data"]["already_exists"], false);
    let again = f.init();
    assert_eq!(again["data"]["workspace_id"], init["data"]["workspace_id"]);
    assert_eq!(again["data"]["already_exists"], true);
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
    // Copiar o mover solo lleva la identidad con un marcador anterior al registro.
    let init = f.legacy();
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
    assert_eq!(
        fs::read(copy.join(".kn/config.json")).unwrap(),
        fs::read(f.main.join(".kn/config.json")).unwrap(),
        "--fresh no reescribe el marcador"
    );
    let moved = f._tmp.path().join("moved");
    fs::rename(&f.main, &moved).unwrap();
    f.run(&moved, &["status"], 0);
    let reused = f.run(&moved, &["init"], 0);
    assert_eq!(reused["data"]["workspace_id"], init["data"]["workspace_id"]);
    assert_eq!(reused["data"]["already_exists"], true);
    assert_eq!(f.registered(&moved), init["data"]["workspace_id"]);
    // Una copia hecha antes de registrar la principal movida sigue siendo una copia.
    let late = f._tmp.path().join("late-copy");
    fs::create_dir_all(late.join(".kn")).unwrap();
    fs::copy(moved.join(".kn/config.json"), late.join(".kn/config.json")).unwrap();
    assert_eq!(
        f.run(&late, &["status"], 1)["errors"][0]["code"],
        "WORKSPACE_COPIED"
    );
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
fn finish_records_pending_edits_before_integrating() {
    let f = Fixture::new();
    f.init();
    let s = f.session("agent");
    fs::write(s.join("borrador.md"), "sin versión todavía\n").unwrap();
    let out = f.run(&s, &["session", "finish"], 0);
    assert_eq!(out["data"]["recorded_document_count"], 1);
    assert_eq!(
        fs::read_to_string(f.main.join("borrador.md")).unwrap(),
        "sin versión todavía\n"
    );
    assert_eq!(
        f.run(&s, &["history", "--limit", "1"], 0)["data"]["versions"][0]["reason"],
        "pre_finish_snapshot"
    );
    assert_eq!(f.run(&s, &["status"], 0)["data"]["clean"], true);
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
                    // Windows may expose delayed directory timestamps from read_dir.
                    // File bytes, file mtimes and the complete entry set remain exact.
                    (
                        bytes,
                        if meta.is_dir() {
                            std::time::SystemTime::UNIX_EPOCH
                        } else {
                            meta.modified().unwrap()
                        },
                    ),
                );
                if meta.is_dir() {
                    walk(root, &e.path(), out);
                }
            }
        }
        walk(root, root, &mut out);
        out
    }
    for legacy in [true, false] {
        let f = Fixture::new();
        if legacy {
            f.legacy();
        } else {
            f.init();
        }
        let moved = f._tmp.path().join("moved");
        fs::rename(&f.main, &moved).unwrap();
        fs::write(moved.join("manual.txt"), "not yet observed").unwrap();
        let before = tree(f._tmp.path());
        let out = f.run(&moved, &["inspect"], 0);
        if legacy {
            assert_eq!(out["data"]["primary_root"], canonical(&moved));
        } else {
            // El registro nombra la ruta anterior; en la nueva no hay identidad.
            assert_eq!(out["data"]["managed"], false);
        }
        assert_eq!(tree(f._tmp.path()), before);
    }
}

#[test]
fn inspect_does_not_hide_damaged_or_copied_workspaces() {
    let registered = Fixture::new();
    registered.init();
    fs::write(registered.home.join("roots.json"), "broken").unwrap();
    assert_eq!(
        registered.run(&registered.main, &["inspect"], 1)["errors"][0]["code"],
        "INVALID_STATE"
    );
    let f = Fixture::new();
    f.legacy();
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
    // Con marcador anterior, --fresh no lo reescribe: el registro tiene que ganarle.
    for legacy in [false, true] {
        let f = Fixture::new();
        if legacy {
            f.legacy();
        } else {
            f.init();
        }
        let old = f.session("old");
        fs::write(old.join("draft.txt"), "keep in old session").unwrap();
        f.run(&old, &["commit"], 0);
        let fresh = f.run(&f.main, &["init", "--fresh"], 0);
        assert_eq!(f.registered(&f.main), fresh["data"]["workspace_id"]);
        assert_eq!(f.main.join(".kn/config.json").exists(), legacy);
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
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn init_writes_nothing_into_the_documents_folder() {
    let f = Fixture::new();
    fs::write(f.main.join("acta.md"), "uno\n").unwrap();
    fs::create_dir(f.main.join("anexos")).unwrap();
    fs::write(f.main.join("anexos/nota.md"), "dos\n").unwrap();
    let before = names(&f.main);
    let init = f.init();
    let id = init["data"]["workspace_id"].clone();
    assert_eq!(init["data"]["already_exists"], false);
    assert_eq!(names(&f.main), before, "ni .kn ni lock en la carpeta");
    assert_eq!(f.registry()["schema_version"], 1);
    assert_eq!(f.registered(&f.main), id);
    let again = f.init();
    assert_eq!(again["data"]["workspace_id"], id);
    assert_eq!(again["data"]["already_exists"], true);

    let sub = f.main.join("anexos");
    assert_eq!(f.run(&sub, &["status"], 0)["data"]["workspace_id"], id);
    assert_eq!(
        f.raw(&sub, &["rev-parse", "--is-inside-work-tree"], 0),
        b"true\n"
    );
    assert_eq!(
        f.raw(&sub, &["rev-parse", "--show-toplevel"], 0),
        toplevel(&f.main).as_bytes()
    );
    let s = session_path(&f.run(&sub, &["worktree", "add", "agente"], 0));
    fs::write(s.join("respuesta.md"), "del agente\n").unwrap();
    f.run(&s, &["commit", "-m", "Respuesta"], 0);
    f.run(&s, &["worktree", "update"], 0);
    f.run(&s, &["worktree", "finish"], 0);
    assert_eq!(
        fs::read_to_string(f.main.join("respuesta.md")).unwrap(),
        "del agente\n"
    );
    assert!(f.raw(&sub, &["status", "--porcelain", "-z"], 0).is_empty());
    assert_eq!(
        f.run(&sub, &["worktree", "list"], 0)["data"]["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!f.main.join(".kn").exists());

    // Mover una carpeta registrada la deja sin identidad; init allí empieza otra.
    let moved = f._tmp.path().join("moved");
    fs::rename(&f.main, &moved).unwrap();
    assert_eq!(
        f.run(&moved, &["status"], 1)["errors"][0]["code"],
        "NOT_A_WORKSPACE"
    );
    let other = f.run(&moved, &["init"], 0);
    assert_ne!(other["data"]["workspace_id"], id);
    assert_eq!(other["data"]["already_exists"], false);
}

#[test]
fn nested_primaries_keep_independent_histories() {
    let f = Fixture::new();
    let child = f.main.join("equipo");
    fs::create_dir(&child).unwrap();
    fs::write(f.main.join("plan.md"), "padre\n").unwrap();
    fs::write(child.join("acta.md"), "hijo\n").unwrap();
    let parent = f.init();
    let hijo = f.run(&child, &["init"], 0);
    assert_ne!(hijo["data"]["workspace_id"], parent["data"]["workspace_id"]);
    assert_eq!(hijo["data"]["already_exists"], false);
    assert!(!child.join(".kn").exists());

    fs::create_dir(child.join("sub")).unwrap();
    assert_eq!(
        f.raw(&child.join("sub"), &["rev-parse", "--show-toplevel"], 0),
        toplevel(&child).as_bytes()
    );
    assert_eq!(
        f.raw(&f.main, &["rev-parse", "--show-toplevel"], 0),
        toplevel(&f.main).as_bytes()
    );
    assert_eq!(
        f.run(&child, &["status"], 0)["data"]["workspace_id"],
        hijo["data"]["workspace_id"]
    );

    let s = session_path(&f.run(&child, &["worktree", "add", "equipo"], 0));
    assert!(s.join("acta.md").is_file());
    assert!(!s.join("plan.md").exists(), "la sesión es del hijo");
    fs::write(s.join("respuesta.md"), "del agente\n").unwrap();
    f.run(&s, &["worktree", "finish"], 0);
    assert_eq!(
        fs::read_to_string(child.join("respuesta.md")).unwrap(),
        "del agente\n"
    );

    // Para la principal de arriba, lo que guardó el hijo es un cambio externo.
    let status = f.run(&f.main, &["status"], 0);
    assert_eq!(
        status["data"]["local_changes"][0]["path"],
        "equipo/respuesta.md"
    );
    let p = session_path(&f.run(&f.main, &["worktree", "add", "padre"], 0));
    let log = f.run(&p, &["log"], 0)["data"]["versions"].clone();
    assert_eq!(log[0]["reason"], "external_observation");
    assert_eq!(
        version_files(&f, &parent, &log[0]["id"]),
        ["equipo/respuesta.md"]
    );
    assert_eq!(
        fs::read_to_string(p.join("equipo/respuesta.md")).unwrap(),
        "del agente\n"
    );
}

#[test]
fn init_inside_a_session_is_refused() {
    let f = Fixture::new();
    f.init();
    let s = f.session("agente");
    fs::create_dir(s.join("sub")).unwrap();
    for dir in [s.clone(), s.join("sub")] {
        for args in [vec!["init"], vec!["init", "--fresh"]] {
            let out = f.run(&dir, &args, 3);
            assert_eq!(out["errors"][0]["code"], "INVALID_INPUT");
            assert!(
                out["errors"][0]["message"]
                    .as_str()
                    .unwrap()
                    .contains("sesión"),
                "{out}"
            );
        }
    }
    assert_eq!(fs::read_dir(f.home.join("repos")).unwrap().count(), 1);
    assert_eq!(f.registry()["roots"].as_object().unwrap().len(), 1);
}

#[test]
fn legacy_markers_keep_working_and_migrate_removes_them() {
    let f = Fixture::new();
    fs::write(f.main.join("acta.md"), "uno\n").unwrap();
    let init = f.legacy();
    let id = init["data"]["workspace_id"].clone();
    let registry = fs::read(f.home.join("roots.json")).unwrap();

    // Las lecturas no registran.
    assert_eq!(f.run(&f.main, &["status"], 0)["data"]["workspace_id"], id);
    assert_eq!(
        f.raw(&f.main, &["rev-parse", "--show-toplevel"], 0),
        toplevel(&f.main).as_bytes()
    );
    assert!(
        f.raw(&f.main, &["status", "--porcelain", "-z"], 0)
            .is_empty()
    );
    assert_eq!(f.run(&f.main, &["inspect"], 0)["data"]["workspace_id"], id);
    assert_eq!(fs::read(f.home.join("roots.json")).unwrap(), registry);

    let migrated = f.run(&f.main, &["migrate"], 0)["data"].clone();
    let control = fs::canonicalize(&f.main).unwrap().join(".kn");
    assert_eq!(migrated["registered"], true);
    assert_eq!(migrated["workspace_id"], id);
    assert_eq!(
        migrated["removed"],
        json!([
            control.join("config.json"),
            control.join("kn.lock"),
            control
        ])
    );
    assert!(!f.main.join(".kn").exists());
    assert_eq!(f.registered(&f.main), id);

    let s = f.session("agente");
    fs::write(s.join("respuesta.md"), "del agente\n").unwrap();
    f.run(&s, &["commit", "-m", "Respuesta"], 0);
    f.run(&s, &["worktree", "finish"], 0);
    assert_eq!(f.run(&f.main, &["status"], 0)["data"]["workspace_id"], id);
    assert!(
        f.run(&s, &["log"], 0)["data"]["versions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["reason"] == "init"),
        "el historial es el mismo"
    );

    let again = f.run(&f.main, &["migrate"], 0)["data"].clone();
    assert_eq!(again["registered"], false);
    assert_eq!(again["removed"], json!([]));
    assert_eq!(again["workspace_id"], id);
    assert_eq!(
        f.run(&s, &["migrate"], 3)["errors"][0]["code"],
        "INVALID_INPUT"
    );

    // Una escritura registra la carpeta y no toca su marcador.
    let g = Fixture::new();
    let init = g.legacy();
    let marker = g.main.join(".kn/config.json");
    let marker_bytes_g = fs::read(&marker).unwrap();
    g.session("agente");
    assert_eq!(g.registered(&g.main), init["data"]["workspace_id"]);
    assert_eq!(fs::read(&marker).unwrap(), marker_bytes_g);
    let migrated = g.run(&g.main, &["migrate"], 0)["data"].clone();
    assert_eq!(migrated["registered"], false);
    assert_eq!(migrated["removed"].as_array().unwrap().len(), 3);

    // Con algo ajeno en .kn no se quita nada.
    let h = Fixture::new();
    h.legacy();
    let marker_bytes = fs::read(h.main.join(".kn/config.json")).unwrap();
    fs::write(h.main.join(".kn/notas.txt"), "de la persona").unwrap();
    let refused = h.run(&h.main, &["migrate"], 2);
    assert_eq!(refused["errors"][0]["code"], "CONFLICT");
    assert!(
        refused["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("notas.txt")
    );
    assert_eq!(
        fs::read(h.main.join(".kn/config.json")).unwrap(),
        marker_bytes
    );
    assert!(h.main.join(".kn/kn.lock").is_file());
    assert_eq!(h.registered(&h.main), Value::Null);
}

#[test]
fn a_synced_legacy_marker_does_not_block_another_machine() {
    let f = Fixture::new();
    fs::write(f.main.join("acta.md"), "uno\n").unwrap();
    let owner = f.legacy();
    let marker = f.main.join(".kn/config.json");
    let marker_bytes = fs::read(&marker).unwrap();
    let other_home = f._tmp.path().join("otra-maquina");
    let other = [("KN_HOME", other_home.to_str().unwrap())];

    let missing = f.run_env(&f.main, &["status"], 3, &other);
    assert!(
        missing["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("kn init"),
        "{missing}"
    );
    let mine = f.run_env(&f.main, &["init"], 0, &other);
    assert_ne!(mine["data"]["workspace_id"], owner["data"]["workspace_id"]);
    assert_eq!(mine["data"]["already_exists"], false);
    assert_eq!(fs::read(&marker).unwrap(), marker_bytes);
    assert_eq!(
        f.run_env(&f.main, &["status"], 0, &other)["data"]["workspace_id"],
        mine["data"]["workspace_id"]
    );
    let fresh = f.run_env(&f.main, &["init", "--fresh"], 0, &other);
    assert_ne!(fresh["data"]["workspace_id"], mine["data"]["workspace_id"]);
    assert_eq!(fs::read(&marker).unwrap(), marker_bytes);
    assert_eq!(
        f.run_env(&f.main, &["migrate"], 2, &other)["errors"][0]["code"],
        "CONFLICT"
    );
    assert_eq!(fs::read(&marker).unwrap(), marker_bytes);

    // La dueña sigue con su historial, y al migrar la otra máquina no lo nota.
    assert_eq!(
        f.run(&f.main, &["status"], 0)["data"]["workspace_id"],
        owner["data"]["workspace_id"]
    );
    f.session("duena");
    f.run(&f.main, &["migrate"], 0);
    assert!(!f.main.join(".kn").exists());
    assert_eq!(
        f.run_env(&f.main, &["status"], 0, &other)["data"]["workspace_id"],
        fresh["data"]["workspace_id"]
    );
}

#[test]
fn orphan_histories_are_ignored() {
    let f = Fixture::new();
    fs::write(f.main.join("acta.md"), "uno\n").unwrap();
    let orphan = |id: &str| {
        let dir = f.home.join("repos").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let location = json!({"root": canonical(&f.main)});
        fs::write(dir.join("location.json"), location.to_string()).unwrap();
    };
    let first = "11111111-1111-4111-8111-111111111111";
    orphan(first);
    assert_eq!(
        f.run(&f.main, &["status"], 1)["errors"][0]["code"],
        "NOT_A_WORKSPACE"
    );
    let init = f.init();
    let id = init["data"]["workspace_id"].clone();
    assert_ne!(id, first);
    assert_eq!(init["data"]["already_exists"], false);
    orphan("22222222-2222-4222-8222-222222222222");
    assert_eq!(f.run(&f.main, &["status"], 0)["data"]["workspace_id"], id);
    assert_eq!(f.init()["data"]["workspace_id"], id);
    let fresh = f.run(&f.main, &["init", "--fresh"], 0)["data"]["workspace_id"].clone();
    assert_ne!(fresh, id);
    assert_eq!(
        f.run(&f.main, &["status"], 0)["data"]["workspace_id"],
        fresh
    );
    assert!(
        f.home
            .join("repos")
            .join(id.as_str().unwrap())
            .join("HEAD")
            .is_file(),
        "--fresh conserva el historial anterior"
    );
}

#[test]
fn cloud_fetch_with_a_zero_budget_reads_nothing() {
    let f = Fixture::new();
    let (vacio, lleno) = ("vacio.pdf", "anexos/informe.pdf");
    fs::write(f.main.join(vacio), "").unwrap();
    fs::create_dir(f.main.join("anexos")).unwrap();
    fs::write(f.main.join(lleno), "contenido\n").unwrap();
    let nube = [(NUBE, "vacio.pdf\nanexos/informe.pdf")];
    f.run_env(&f.main, &["init"], 0, &nube);

    let none =
        f.run_env(&f.main, &["cloud", "fetch", "--max-bytes", "0"], 0, &nube)["data"].clone();
    assert_eq!(none["fetched"], json!([]));
    assert_eq!(none["failed"], json!([]));
    assert_eq!(none["skipped_budget"], json!([vacio, lleno]));
    assert_eq!(none["bytes_fetched"], 0);

    let one = f.run_env(&f.main, &["cloud", "fetch", "--max-bytes", "1"], 0, &nube)["data"].clone();
    assert_eq!(one["fetched"], json!([vacio]));
    assert_eq!(one["skipped_budget"], json!([lleno]));
}

/// Mientras vive, leer el documento falla, como una descarga que el proveedor no completa.
struct Unreadable {
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    _handle: fs::File,
}
impl Unreadable {
    fn new(path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
            Self {
                path: path.to_path_buf(),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let handle = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .unwrap();
            Self { _handle: handle }
        }
    }
}
impl Drop for Unreadable {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
}
impl Fixture {
    /// `cloud fetch --progress`: el envelope de stdout y cada línea de stderr.
    fn fetch_progress(&self, args: &[&str], nube: &str) -> (Value, Vec<Value>) {
        let out = Command::new(env!("CARGO_BIN_EXE_kn"))
            .current_dir(&self.main)
            .env("KN_HOME", &self.home)
            .env(NUBE, nube)
            .args(["cloud", "fetch", "--progress", "--json"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let envelope = serde_json::from_slice(&out.stdout).expect("un solo envelope en stdout");
        let lines = String::from_utf8(out.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        (envelope, lines)
    }
}

#[test]
fn cloud_fetch_all_reports_progress_and_remembers_failures() {
    let f = Fixture::new();
    fs::create_dir(f.main.join("lote")).unwrap();
    let mut nube = vec![];
    let mut total = 0;
    for i in 0..40 {
        let rel = format!("lote/doc{i:02}.pdf");
        fs::write(f.main.join(&rel), "x".repeat(i + 1)).unwrap();
        total += i + 1;
        nube.push(rel);
    }
    let roto_contenido = "sin descargar\n";
    fs::write(f.main.join("roto.pdf"), roto_contenido).unwrap();
    nube.push("roto.pdf".into());
    let lista = nube.join("\n");
    let env = [(NUBE, lista.as_str())];
    let init = f.run_env(&f.main, &["init"], 0, &env);
    let record = cloud_record(&f, &init);
    // Un registro del schema 1, sin `failed`, se sigue leyendo.
    fs::write(
        &record,
        json!({"schema_version": 1, "cloud_only": nube}).to_string(),
    )
    .unwrap();
    let status = f.run_env(&f.main, &["status"], 0, &env)["data"].clone();
    assert_eq!(status["cloud_only"].as_array().unwrap().len(), 41);
    assert_eq!(status["cloud_only_bytes"], total + roto_contenido.len());
    assert_eq!(status["cloud_failed"], json!([]));

    let roto = Unreadable::new(&f.main.join("roto.pdf"));
    let (out, lines) = f.fetch_progress(&["--all", "--timeout-secs", "5"], &lista);
    let data = &out["data"];
    assert_eq!(data["fetched"].as_array().unwrap().len(), 40);
    assert_eq!(data["failed"].as_array().unwrap().len(), 1);
    assert_eq!(data["failed"][0]["path"], "roto.pdf");
    assert_eq!(data["bytes_fetched"], total);
    assert_eq!(lines.len(), 41, "una línea por documento");
    let mut seen: Vec<_> = lines.iter().map(|l| l["path"].to_string()).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 41, "ninguno se intenta dos veces entre tandas");
    for line in &lines {
        for key in [
            "path",
            "outcome",
            "bytes",
            "fetched_count",
            "failed_count",
            "remaining_count",
            "bytes_fetched",
            "bytes_remaining",
        ] {
            assert!(line.get(key).is_some(), "{key}: {line}");
        }
    }
    let last = lines.last().unwrap();
    assert_eq!(last["fetched_count"], 40);
    assert_eq!(last["failed_count"], 1);
    assert_eq!(last["remaining_count"], 0);
    assert_eq!(last["bytes_remaining"], 0);
    assert_eq!(last["bytes_fetched"], total);
    let fallo = lines.iter().find(|l| l["path"] == "roto.pdf").unwrap();
    assert_eq!(fallo["outcome"], "failed");
    assert_eq!(fallo["bytes"], 0);

    let remembered: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    assert_eq!(remembered["schema_version"], 2);
    assert_eq!(remembered["failed"][0]["path"], "roto.pdf");
    assert!(
        remembered["failed"][0]["reason"]
            .as_str()
            .is_some_and(|r| !r.is_empty())
    );
    assert!(
        remembered["failed"][0]["at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    assert_eq!(
        f.run_env(&f.main, &["status"], 0, &env)["data"]["cloud_failed"],
        json!(["roto.pdf"])
    );

    let again = f.run_env(&f.main, &["cloud", "fetch"], 0, &env)["data"].clone();
    assert_eq!(
        again["failed"],
        json!([]),
        "sin --retry-failed no se reintenta"
    );
    assert!(
        !again["fetched"]
            .as_array()
            .unwrap()
            .contains(&json!("roto.pdf"))
    );
    assert!(
        again["remaining"]
            .as_array()
            .unwrap()
            .contains(&json!("roto.pdf"))
    );

    drop(roto);
    let retried =
        f.run_env(&f.main, &["cloud", "fetch", "--retry-failed"], 0, &env)["data"].clone();
    assert!(
        retried["fetched"]
            .as_array()
            .unwrap()
            .contains(&json!("roto.pdf"))
    );
    assert_eq!(
        f.run_env(&f.main, &["status"], 0, &env)["data"]["cloud_failed"],
        json!([])
    );

    // Un fallo se olvida cuando el documento desaparece.
    let roto = Unreadable::new(&f.main.join("roto.pdf"));
    f.run_env(&f.main, &["cloud", "fetch", "--all"], 0, &env);
    assert_eq!(
        f.run_env(&f.main, &["status"], 0, &env)["data"]["cloud_failed"],
        json!(["roto.pdf"])
    );
    drop(roto);
    fs::remove_file(f.main.join("roto.pdf")).unwrap();
    assert_eq!(
        f.run_env(&f.main, &["status"], 0, &env)["data"]["cloud_failed"],
        json!([])
    );
    f.run_env(&f.main, &["cloud", "fetch"], 0, &env);
    let settled: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    assert_eq!(settled["failed"], json!([]));
}

#[test]
fn cloud_fetch_all_caps_the_total_budget_across_batches() {
    let f = Fixture::new();
    fs::create_dir(f.main.join("lote")).unwrap();
    let nube: Vec<String> = (0..40).map(|i| format!("lote/doc{i:02}.pdf")).collect();
    for rel in &nube {
        fs::write(f.main.join(rel), "x").unwrap();
    }
    let lista = nube.join("\n");
    let env = [(NUBE, lista.as_str())];
    f.run_env(&f.main, &["init"], 0, &env);
    let capped = f.run_env(
        &f.main,
        &["cloud", "fetch", "--all", "--max-bytes", "35"],
        0,
        &env,
    )["data"]
        .clone();
    assert_eq!(capped["fetched"].as_array().unwrap().len(), 35);
    assert_eq!(capped["skipped_budget"].as_array().unwrap().len(), 5);
    assert_eq!(capped["bytes_fetched"], 35);

    // Sin --all también hay progreso, y el presupuesto corta la pasada.
    let (out, lines) = f.fetch_progress(&["--max-bytes", "3"], &lista);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["fetched_count"], 1);
    assert_eq!(lines[2]["remaining_count"], 37);
    assert_eq!(out["data"]["skipped_budget"].as_array().unwrap().len(), 37);
}

#[test]
fn a_second_cloud_fetch_on_the_same_history_is_refused() {
    use fs2::FileExt;
    let f = Fixture::new();
    fs::write(f.main.join("informe.pdf"), "remoto\n").unwrap();
    let env = [(NUBE, "informe.pdf")];
    let init = f.run_env(&f.main, &["init"], 0, &env);
    let lock_path = f
        .home
        .join("repos")
        .join(init["data"]["workspace_id"].as_str().unwrap())
        .join("cloud-fetch.lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    lock.lock_exclusive().unwrap();
    let busy = f.run_env(&f.main, &["cloud", "fetch"], 1, &env);
    assert_eq!(busy["errors"][0]["code"], "CLOUD_FETCH_BUSY");
    assert_eq!(busy["errors"][0]["retryable"], true);
    let s = session_path(&f.run_env(&f.main, &["worktree", "add", "agente"], 0, &env));
    assert_eq!(
        f.run_env(&s, &["cloud", "fetch"], 1, &env)["errors"][0]["code"],
        "CLOUD_FETCH_BUSY",
        "desde una sesión es el mismo historial"
    );
    drop(lock);
    assert_eq!(
        f.run_env(&f.main, &["cloud", "fetch"], 0, &env)["data"]["fetched"],
        json!(["informe.pdf"])
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
/// Simula que el proveedor no entrega el documento: sin permiso de lectura, Git
/// falla o lo reporta modificado si intenta leerlo. Cambia también el ctime, como
/// al liberarlo. En Windows solo queda la simulación por nombre.
fn unreadable(path: &Path, locked: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if locked { 0o000 } else { 0o644 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = (path, locked);
}
fn porcelain(f: &Fixture, path: &Path, env: &[(&str, &str)]) -> Vec<u8> {
    let out = Command::new(env!("CARGO_BIN_EXE_kn"))
        .current_dir(path)
        .env("KN_HOME", &f.home)
        .envs(env.iter().copied())
        .args(["status", "--porcelain", "-z"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
#[test]
fn versioned_documents_freed_by_the_cloud_are_not_read() {
    let f = Fixture::new();
    let liberado = "anexos/informe final.md";
    let contenido = "versionado completo\n";
    fs::write(f.main.join("acta.md"), "uno\n").unwrap();
    fs::create_dir(f.main.join("anexos")).unwrap();
    fs::write(f.main.join(liberado), contenido).unwrap();
    let init = f.init();

    // El proveedor libera un documento que ya está en HEAD.
    let nube = [(NUBE, liberado)];
    let doc = f.main.join(liberado);
    unreadable(&doc, true);
    let status = f.run_env(&f.main, &["status"], 0, &nube)["data"].clone();
    assert_eq!(status["clean"], true, "{status}");
    assert_eq!(status["local_changes"], json!([]));
    assert_eq!(status["cloud_only"], json!([liberado]));
    assert_eq!(status["cloud_only_bytes"], contenido.len());
    assert_eq!(status["primary_cloud_only"], Value::Null);
    assert!(porcelain(&f, &f.main, &nube).is_empty());

    // Observar, crear la sesión e integrar no lo leen; la sesión lo tiene completo.
    fs::write(f.main.join("acta.md"), "dos\n").unwrap();
    let added = f.run_env(&f.main, &["worktree", "add", "agente"], 0, &nube);
    assert_eq!(added["data"]["cloud_only"], json!([liberado]));
    let s = session_path(&added);
    assert_eq!(fs::read_to_string(s.join(liberado)).unwrap(), contenido);
    assert_eq!(fs::read_to_string(s.join("acta.md")).unwrap(), "dos\n");
    let log = f.run(&s, &["log"], 0)["data"]["versions"].clone();
    assert_eq!(log[0]["reason"], "external_observation");
    assert_eq!(version_files(&f, &init, &log[0]["id"]), ["acta.md"]);

    // La simulación va por nombre y alcanza también a la sesión; solo importa la principal.
    let session = f.run_env(&s, &["status"], 0, &nube)["data"].clone();
    assert_eq!(session["primary_cloud_only"], json!([liberado]));
    assert_eq!(session["primary_cloud_only_bytes"], contenido.len());

    fs::write(s.join("respuesta.md"), "del agente\n").unwrap();
    f.run(&s, &["commit"], 0);
    f.run_env(&s, &["worktree", "finish"], 0, &nube);
    assert_eq!(
        fs::read_to_string(f.main.join("respuesta.md")).unwrap(),
        "del agente\n"
    );
    let otra = session_path(&f.run_env(&f.main, &["worktree", "add", "otra"], 0, &nube));
    f.run_env(&f.main, &["init"], 0, &nube);
    fs::write(f.main.join("acta.md"), "tres\n").unwrap();
    f.run_env(&otra, &["worktree", "update"], 0, &nube);
    assert_eq!(fs::read_to_string(otra.join("acta.md")).unwrap(), "tres\n");

    // Pisarlo sin leerlo arriesgaría una edición en la nube: la integración espera.
    fs::write(otra.join(liberado), "del agente\n").unwrap();
    f.run(&otra, &["commit"], 0);
    let blocked = f.run_env(&otra, &["worktree", "finish"], 2, &nube);
    assert!(
        blocked["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains(liberado),
        "{blocked}"
    );

    // Descargado igual al versionado: no es un cambio.
    unreadable(&doc, false);
    let status = f.run(&f.main, &["status"], 0)["data"].clone();
    assert_eq!(status["clean"], true, "{status}");
    assert_eq!(status["cloud_only"], json!([]));
    assert_eq!(fs::read_to_string(&doc).unwrap(), contenido);

    // Editado en la nube mientras seguía liberado: al descargarse, cambio normal.
    fs::write(&doc, "editado en la nube\n").unwrap();
    unreadable(&doc, true);
    assert_eq!(
        f.run_env(&f.main, &["status"], 0, &nube)["data"]["clean"],
        true
    );
    unreadable(&doc, false);
    let status = f.run(&f.main, &["status"], 0)["data"].clone();
    assert_eq!(
        status["local_changes"],
        json!([{"path": liberado, "kind": "modified", "status": " M"}])
    );

    // cloud fetch también lo descarga.
    let fetched = f.run_env(&f.main, &["cloud", "fetch", "--all"], 0, &nube)["data"].clone();
    assert_eq!(fetched["fetched"], json!([liberado]));
}
