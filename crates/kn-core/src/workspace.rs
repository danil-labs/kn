use crate::{
    error::{Error, Result},
    git::{Engine, Git},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub workspace_id: uuid::Uuid,
    pub session: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct Location {
    pub root: PathBuf,
}

pub struct Workspace {
    pub config: Config,
    pub git: Git,
    pub home: PathBuf,
    _lock: File,
}
pub fn home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("KN_HOME") {
        return Ok(PathBuf::from(path));
    }
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .map(|p| PathBuf::from(p).join(".kn"))
        .ok_or_else(|| Error::Invalid("Define KN_HOME para guardar el historial.".into()))
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid("Ruta sin directorio.".into()))?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value)?;
    tmp.write_all(b"\n")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| Error::Io(e.error))?;
    Ok(())
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn lock(dir: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("kn.lock"))?;
    acquire_lock(file, false)
}
fn acquire_lock(file: File, shared: bool) -> Result<File> {
    let start = Instant::now();
    loop {
        match if shared {
            FileExt::try_lock_shared(&file)
        } else {
            file.try_lock_exclusive()
        } {
            Ok(()) => return Ok(file),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                if start.elapsed() >= Duration::from_secs(2) {
                    return Err(Error::Busy);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e.into()),
        }
    }
}
/// Find a kn root without treating an independent nested Git repository as documents.
pub fn discover(start: &Path) -> Result<PathBuf> {
    let start = fs::canonicalize(start)?;
    if !start.is_dir() {
        return Err(Error::Invalid("La consulta requiere una carpeta.".into()));
    }
    for root in start.ancestors() {
        match fs::symlink_metadata(root.join(".kn/config.json")) {
            Ok(meta) => {
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return Err(Error::Unsafe(
                        "El registro kn debe ser un archivo regular.".into(),
                    ));
                }
                return Ok(root.to_path_buf());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        match fs::symlink_metadata(root.join(".git")) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::NotWorkspace)
}
impl Workspace {
    pub fn open(start: &Path) -> Result<Self> {
        Self::open_inner(start, false)
    }
    /// Uses an existing shared lock; never creates files or claims a moved root.
    pub fn open_read_only(start: &Path) -> Result<Self> {
        Self::open_inner(start, true)
    }
    fn open_inner(start: &Path, read_only: bool) -> Result<Self> {
        let root = discover(start)?;
        if fs::symlink_metadata(root.join(".kn"))?
            .file_type()
            .is_symlink()
        {
            return Err(Error::Unsafe(
                ".kn no puede ser un enlace simbólico.".into(),
            ));
        }
        let config: Config = read_json(&root.join(".kn/config.json"))?;
        if config.schema_version != 2 {
            return Err(Error::Invalid(
                "Formato anterior; conserva el historial y usa kn init --fresh.".into(),
            ));
        }
        let home = fs::canonicalize(home()?)?;
        let common = home.join("repos").join(config.workspace_id.to_string());
        if !common.join("HEAD").is_file() {
            return Err(Error::Invalid(
                "No se encontró el historial externo; recupera KN_HOME de tu respaldo.".into(),
            ));
        }
        let guard = if read_only {
            acquire_lock(File::open(common.join("kn.lock"))?, true)?
        } else {
            lock(&common)?
        };
        let location: Location = read_json(&common.join("location.json"))?;
        let dir = if let Some(name) = &config.session {
            validate_name(name)?;
            let expected = home
                .join("sessions")
                .join(config.workspace_id.to_string())
                .join(name);
            if fs::canonicalize(expected)? != root {
                return Err(Error::Copied);
            }
            // Git creates and owns this gitfile; only accept the registered metadata path.
            let pointer = fs::read_to_string(root.join(".git"))?;
            let dir = PathBuf::from(
                pointer
                    .trim()
                    .strip_prefix("gitdir: ")
                    .ok_or_else(|| Error::Unsafe("Sesión sin registro Git válido.".into()))?,
            );
            let dir = fs::canonicalize(dir)?;
            if !dir.starts_with(common.join("worktrees")) {
                return Err(Error::Unsafe(
                    "Registro de sesión fuera del historial.".into(),
                ));
            }
            dir
        } else {
            if location.root != root && location.root.exists() {
                return Err(Error::Copied);
            }
            // Read commands do not update location; mutations claim a moved root explicitly.
            common.clone()
        };
        Ok(Self {
            config,
            git: Git { dir, root, common },
            home,
            _lock: guard,
        })
    }
    pub fn claim(&self) -> Result<()> {
        if self.config.session.is_none() {
            atomic_json(
                &self.git.common.join("location.json"),
                &Location {
                    root: self.git.root.clone(),
                },
            )?;
        }
        Ok(())
    }
    pub fn require_session(&self) -> Result<()> {
        if self.config.session.is_none() {
            return Err(Error::SessionRequired);
        }
        Ok(())
    }
    pub fn primary(&self) -> Result<Git> {
        let loc: Location = read_json(&self.git.common.join("location.json"))?;
        if !loc.root.is_dir() {
            return Err(Error::Invalid(
                "Abre la carpeta principal y ejecuta kn init después de moverla.".into(),
            ));
        }
        let control = loc.root.join(".kn");
        let config_path = control.join("config.json");
        if fs::symlink_metadata(&control)?.file_type().is_symlink()
            || !fs::symlink_metadata(&config_path)?.file_type().is_file()
        {
            return Err(Error::Unsafe(
                "El registro de la principal debe ser local y regular.".into(),
            ));
        }
        let config: Config = read_json(&config_path)?;
        if config.schema_version != 2
            || config.workspace_id != self.config.workspace_id
            || config.session.is_some()
        {
            return Err(Error::IdentityChanged);
        }
        Ok(Git {
            dir: self.git.common.clone(),
            root: loc.root,
            common: self.git.common.clone(),
        })
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(Error::Invalid(
            "Nombre de sesión: 1–64 letras ASCII, números, guiones o guiones bajos.".into(),
        ));
    }
    Ok(())
}
pub fn initialize(root: &Path, fresh: bool) -> Result<Workspace> {
    let root = fs::canonicalize(root)?;
    // Sin Git, init dejaría `.kn/` en la carpeta de documentos antes de fallar.
    crate::git::executable()?;
    if root
        .join(".kn")
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(Error::Unsafe(
            ".kn no puede ser un enlace simbólico.".into(),
        ));
    }
    if root.join(".kn/config.json").exists() && !fresh {
        let ws = Workspace::open(&root)?;
        ws.claim()?;
        return Ok(ws);
    }
    if root
        .ancestors()
        .skip(1)
        .any(|p| p.join(".kn/config.json").exists())
    {
        return Err(Error::Invalid(
            "No se puede inicializar dentro de otra carpeta kn.".into(),
        ));
    }
    if fresh && root.join(".kn/config.json").exists() {
        let raw: serde_json::Value = read_json(&root.join(".kn/config.json"))?;
        if raw.get("session").is_some_and(|v| !v.is_null()) {
            return Err(Error::Invalid("No uses --fresh sobre una sesión.".into()));
        }
    }
    let home_path = home()?;
    fs::create_dir_all(&home_path)?;
    let home_path = fs::canonicalize(home_path)?;
    if home_path.starts_with(&root) {
        return Err(Error::Invalid(
            "KN_HOME debe estar fuera de la carpeta de documentos.".into(),
        ));
    }
    fs::create_dir_all(root.join(".kn"))?;
    // Serialize first-time initialization independently from the repository identity.
    let _init_guard = lock(&root.join(".kn"))?;
    if !fresh && root.join(".kn/config.json").exists() {
        return Workspace::open(&root);
    }
    let id = uuid::Uuid::new_v4();
    let common = home_path.join("repos").join(id.to_string());
    fs::create_dir_all(&common)?;
    // Sin esto cada reintento de un init fallido deja otro historial huérfano en KN_HOME.
    if let Err(e) = record_initial(&root, &common, id) {
        let _ = fs::remove_dir_all(&common);
        return Err(e);
    }
    Workspace::open(&root)
}

fn record_initial(root: &Path, common: &Path, id: uuid::Uuid) -> Result<()> {
    let git = Git {
        dir: common.to_path_buf(),
        common: common.to_path_buf(),
        root: root.to_path_buf(),
    };
    git.run(&["init", "--initial-branch=main", "--template="])?;
    fs::create_dir_all(common.join("info"))?;
    fs::write(
        common.join("info/exclude"),
        ".kn/\n.git/\n.DS_Store\nThumbs.db\nDesktop.ini\n~$*\n*.tmp\n.~lock.*#\n",
    )?;
    // Override attributes so arbitrary document attributes cannot run filters or normalize bytes.
    fs::write(
        common.join("info/attributes"),
        "* -text -filter -ident -working-tree-encoding !diff !merge\n",
    )?;
    atomic_json(
        &common.join("location.json"),
        &Location {
            root: root.to_path_buf(),
        },
    )?;
    let config = Config {
        schema_version: 2,
        workspace_id: id,
        session: None,
    };
    crate::ops::check_safe(&git)?;
    let hidden = crate::cloud::Hidden::of(root)?;
    git.run_with(&hidden.git_config()?, &["add", "-A", "--", "."])?;
    git.run(&[
        "commit",
        "--allow-empty",
        "-m",
        "Versión inicial",
        "-m",
        "Kn-Reason: init",
    ])?;
    crate::cloud::remember(&git, &hidden.paths)?;
    // Empty initial commit anchors worktrees; it is never exposed as a document version.
    atomic_json(&root.join(".kn/config.json"), &config)
}
