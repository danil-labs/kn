use crate::{
    error::{Error, Result},
    git::{Engine, Git},
    registry::{self, Registry},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
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

/// De dónde sale la identidad de la carpeta abierta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Principal registrada en `KN_HOME/roots.json`.
    Registered,
    /// Principal con el marcador `.kn/config.json` anterior al registro.
    Legacy,
    /// Sesión en `KN_HOME/sessions`, con su propio marcador.
    Session,
}

/// Lo que identifica a una carpeta encontrada al subir desde el directorio de inicio.
pub enum Identity {
    Registered(uuid::Uuid),
    Legacy(Config),
    Session(Config),
}
pub struct Found {
    pub root: PathBuf,
    pub identity: Identity,
}

pub struct Workspace {
    pub config: Config,
    pub source: Source,
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
    lock_file(&dir.join("kn.lock"))
}
fn lock_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    acquire_lock(file, false)
}
pub(crate) fn acquire_lock(file: File, shared: bool) -> Result<File> {
    let start = Instant::now();
    loop {
        match if shared {
            FileExt::try_lock_shared(&file)
        } else {
            file.try_lock_exclusive()
        } {
            Ok(()) => return Ok(file),
            Err(e)
                if e.kind() == ErrorKind::WouldBlock
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
fn missing(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

/// El marcador `.kn/config.json` de `dir`, si lo tiene. Solo lo tienen las sesiones y
/// las principales inicializadas antes del registro.
pub fn marker(dir: &Path) -> Result<Option<Config>> {
    let path = dir.join(".kn").join("config.json");
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if missing(&e) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() || fs::symlink_metadata(dir.join(".kn"))?.is_symlink() {
        return Err(Error::Unsafe(
            "El registro kn debe ser un archivo regular dentro de una carpeta .kn real.".into(),
        ));
    }
    Ok(Some(read_json(&path)?))
}

/// El registro gana: una principal registrada no lee su `.kn`, que puede haber
/// llegado por sincronización desde otra máquina.
fn identity_at(dir: &Path, registry: &Registry) -> Result<Option<Identity>> {
    if let Some(id) = registry.get(dir) {
        return Ok(Some(Identity::Registered(id)));
    }
    Ok(marker(dir)?.map(|config| {
        if config.session.is_some() {
            Identity::Session(config)
        } else {
            Identity::Legacy(config)
        }
    }))
}

/// Find the nearest kn root without treating an independent nested Git repository as documents.
pub fn discover(start: &Path) -> Result<Found> {
    let start = fs::canonicalize(start)?;
    if !start.is_dir() {
        return Err(Error::Invalid("La consulta requiere una carpeta.".into()));
    }
    let registry = Registry::load(&home()?)?;
    for root in start.ancestors() {
        if let Some(identity) = identity_at(root, &registry)? {
            return Ok(Found {
                root: root.to_path_buf(),
                identity,
            });
        }
        match fs::symlink_metadata(root.join(".git")) {
            Ok(_) => break,
            Err(e) if e.kind() == ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::NotWorkspace)
}
impl Workspace {
    pub fn open(start: &Path) -> Result<Self> {
        Self::open_inner(start, false)
    }
    /// Uses an existing shared lock; never creates files, claims a moved root or registers it.
    pub fn open_read_only(start: &Path) -> Result<Self> {
        Self::open_inner(start, true)
    }
    fn open_inner(start: &Path, read_only: bool) -> Result<Self> {
        let Found { root, identity } = discover(start)?;
        let (config, source) = match identity {
            Identity::Registered(workspace_id) => (
                Config {
                    schema_version: 2,
                    workspace_id,
                    session: None,
                },
                Source::Registered,
            ),
            Identity::Legacy(config) => (config, Source::Legacy),
            Identity::Session(config) => (config, Source::Session),
        };
        if config.schema_version != 2 {
            return Err(Error::Invalid(
                "Formato anterior; conserva el historial y usa kn init --fresh.".into(),
            ));
        }
        // Sin KN_HOME en esta máquina también falta el historial; canonicalizar antes daría IO_ERROR.
        let home = home()?;
        let id = config.workspace_id.to_string();
        if !home.join("repos").join(&id).join("HEAD").is_file() {
            return Err(Error::Invalid(if source == Source::Legacy {
                "El marcador .kn de esta carpeta nombra un historial que no está en esta máquina. Si la carpeta llegó por sincronización, kn init le da un historial propio sin tocar el marcador; si es tuya, recupera KN_HOME de tu respaldo.".into()
            } else {
                "No se encontró el historial externo; recupera KN_HOME de tu respaldo.".into()
            }));
        }
        let home = fs::canonicalize(home)?;
        let common = home.join("repos").join(id);
        let guard = if read_only {
            acquire_lock(File::open(common.join("kn.lock"))?, true)?
        } else {
            lock(&common)?
        };
        let location: Location = read_json(&common.join("location.json"))?;
        let elsewhere = |path: &Path| path != root && path.exists();
        let dir =
            match source {
                Source::Session => {
                    let name = config.session.as_deref().unwrap_or_default();
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
                    let dir =
                        PathBuf::from(pointer.trim().strip_prefix("gitdir: ").ok_or_else(
                            || Error::Unsafe("Sesión sin registro Git válido.".into()),
                        )?);
                    let dir = fs::canonicalize(dir)?;
                    if !dir.starts_with(common.join("worktrees")) {
                        return Err(Error::Unsafe(
                            "Registro de sesión fuera del historial.".into(),
                        ));
                    }
                    dir
                }
                Source::Legacy => {
                    // Un marcador copiado no opera sobre el historial de una carpeta que sigue existiendo.
                    if elsewhere(&location.root)
                        || Registry::load(&home)?
                            .roots_of(config.workspace_id)
                            .any(|p| elsewhere(&p))
                    {
                        return Err(Error::Copied);
                    }
                    // Read commands do not update location; mutations claim a moved root explicitly.
                    common.clone()
                }
                Source::Registered => {
                    if elsewhere(&location.root) {
                        return Err(Error::Copied);
                    }
                    common.clone()
                }
            };
        Ok(Self {
            config,
            source,
            git: Git { dir, root, common },
            home,
            _lock: guard,
        })
    }
    /// Una escritura en la principal fija su ubicación y la registra si solo tenía el
    /// marcador anterior. Devuelve si la registró ahora; en una sesión no hace nada.
    pub fn claim(&self) -> Result<bool> {
        if self.source == Source::Session {
            return Ok(false);
        }
        atomic_json(
            &self.git.common.join("location.json"),
            &Location {
                root: self.git.root.clone(),
            },
        )?;
        registry::register(&self.home, &self.git.root, self.config.workspace_id)
    }
    pub fn require_session(&self) -> Result<()> {
        if self.config.session.is_none() {
            return Err(Error::SessionRequired);
        }
        Ok(())
    }
    pub fn primary(&self) -> Result<Git> {
        Ok(self.resolve_primary()?.0)
    }
    /// Como `primary`, antes de escribir en ella: una principal que solo tiene el
    /// marcador anterior queda registrada, sin tocar el marcador.
    pub fn primary_for_write(&self) -> Result<Git> {
        let (git, registered) = self.resolve_primary()?;
        if !registered {
            registry::register(&self.home, &git.root, self.config.workspace_id)?;
        }
        Ok(git)
    }
    /// La principal de este historial y si está registrada. Otra identidad en su
    /// ubicación, registrada o en un marcador, es IdentityChanged.
    fn resolve_primary(&self) -> Result<(Git, bool)> {
        let loc: Location = read_json(&self.git.common.join("location.json"))?;
        if !loc.root.is_dir() {
            return Err(Error::Invalid(format!(
                "La principal ya no está en {}. Si la moviste, ejecuta kn init en la nueva ubicación: el historial solo se conserva si la carpeta todavía tiene su marcador .kn.",
                loc.root.display()
            )));
        }
        let id = self.config.workspace_id;
        let registered = match identity_at(&loc.root, &Registry::load(&self.home)?)? {
            Some(Identity::Registered(found)) if found == id => true,
            Some(Identity::Legacy(config))
                if config.schema_version == 2 && config.workspace_id == id =>
            {
                false
            }
            _ => return Err(Error::IdentityChanged),
        };
        Ok((
            Git {
                dir: self.git.common.clone(),
                root: loc.root,
                common: self.git.common.clone(),
            },
            registered,
        ))
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

/// Lee el marcador sin exigir el formato actual: un marcador de otra versión en un
/// ancestro no debe impedir inicializar, salvo que sea de una sesión.
fn session_marker(dir: &Path) -> Result<bool> {
    match fs::read(dir.join(".kn").join("config.json")) {
        Ok(bytes) => {
            let raw: Value = serde_json::from_slice(&bytes)?;
            Ok(raw.get("session").is_some_and(|v| !v.is_null()))
        }
        Err(e) if missing(&e) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Serializa el primer init de una raíz desde KN_HOME: la carpeta no recibe archivos de kn.
fn init_lock(home: &Path, root: &Path) -> Result<File> {
    // FNV-1a y no DefaultHasher, cuyo algoritmo puede cambiar entre versiones de Rust.
    let hash = root
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
    let dir = home.join("locks");
    fs::create_dir_all(&dir)?;
    lock_file(&dir.join(format!("{hash:016x}.lock")))
}

/// Un marcador anterior sirve si su historial está en esta máquina. Si no, llegó por
/// sincronización desde otra y `init` registra una identidad nueva sin tocarlo.
fn legacy_with_history(home: &Path, root: &Path) -> Result<bool> {
    Ok(marker(root)?.is_some_and(|config| {
        home.join("repos")
            .join(config.workspace_id.to_string())
            .join("HEAD")
            .is_file()
    }))
}

/// Devuelve el espacio y si ya existía. No escribe nada en la carpeta de documentos.
pub fn initialize(root: &Path, fresh: bool) -> Result<(Workspace, bool)> {
    let root = fs::canonicalize(root)?;
    // Sin Git, init crearía KN_HOME antes de fallar.
    crate::git::executable()?;
    if !root.is_dir() {
        return Err(Error::Invalid("kn init requiere una carpeta.".into()));
    }
    for dir in root.ancestors() {
        if session_marker(dir)? {
            return Err(Error::Invalid(
                "No se puede inicializar dentro de una sesión de kn; inicializa la carpeta principal."
                    .into(),
            ));
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
    if root.starts_with(&home_path) {
        return Err(Error::Invalid(
            "La carpeta de documentos debe estar fuera de KN_HOME.".into(),
        ));
    }
    let _init_guard = init_lock(&home_path, &root)?;
    if !fresh
        && (Registry::load(&home_path)?.get(&root).is_some()
            || legacy_with_history(&home_path, &root)?)
    {
        let ws = Workspace::open(&root)?;
        ws.claim()?;
        return Ok((ws, true));
    }
    let id = uuid::Uuid::new_v4();
    let common = home_path.join("repos").join(id.to_string());
    fs::create_dir_all(&common)?;
    // Sin esto cada reintento de un init fallido deja otro historial huérfano en KN_HOME.
    if let Err(e) = record_initial(&root, &common)
        .and_then(|()| registry::register(&home_path, &root, id).map(drop))
    {
        let _ = fs::remove_dir_all(&common);
        return Err(e);
    }
    Ok((Workspace::open(&root)?, false))
}

fn record_initial(root: &Path, common: &Path) -> Result<()> {
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
    crate::ops::check_safe(&git)?;
    let hidden = crate::cloud::Hidden::of(&git)?;
    git.run_with(&hidden.git_config()?, &["add", "-A", "--", "."])?;
    // Empty initial commit anchors worktrees; it is never exposed as a document version.
    git.run(&[
        "commit",
        "--allow-empty",
        "-m",
        "Versión inicial",
        "-m",
        "Kn-Reason: init",
    ])?;
    crate::cloud::remember(&git, &hidden.paths)
}

/// Pasa una principal con marcador al registro y quita de su carpeta lo que kn creó.
/// Registra antes de quitar: si falla a medias, la carpeta sigue identificada.
pub fn migrate(start: &Path) -> Result<Value> {
    let ws = Workspace::open(start)?;
    if ws.source == Source::Session {
        return Err(Error::Invalid(
            "Ejecuta kn migrate en la carpeta principal, no en una sesión.".into(),
        ));
    }
    let root = ws.git.root.clone();
    let id = ws.config.workspace_id;
    let owned = kn_files(&root, id)?;
    let registered = ws.source == Source::Legacy && ws.claim()?;
    let mut removed = vec![];
    for path in owned {
        fs::remove_file(&path)?;
        removed.push(path);
    }
    let control = root.join(".kn");
    if fs::symlink_metadata(&control).is_ok_and(|m| m.is_dir()) {
        fs::remove_dir(&control)?;
        removed.push(control);
    }
    let message = if registered || !removed.is_empty() {
        "La identidad de esta carpeta vive en KN_HOME; la carpeta ya no tiene archivos de kn."
    } else {
        "La carpeta ya estaba migrada; no se cambió nada."
    };
    Ok(
        json!({"workspace_id": id, "root": root, "registered": registered,
        "removed": removed, "message": message}),
    )
}

/// Los archivos de kn en `<root>/.kn`. Si la carpeta tiene otra cosa, o un marcador
/// de otro historial, no se quita nada.
fn kn_files(root: &Path, id: uuid::Uuid) -> Result<Vec<PathBuf>> {
    let control = root.join(".kn");
    match fs::symlink_metadata(&control) {
        Ok(meta) if meta.is_symlink() => {
            return Err(Error::Unsafe(
                ".kn no puede ser un enlace simbólico.".into(),
            ));
        }
        Ok(meta) if meta.is_dir() => (),
        Ok(_) => return Ok(vec![]),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    }
    if let Some(config) = marker(root)?
        && (config.session.is_some() || config.workspace_id != id)
    {
        return Err(Error::Conflict(format!(
            "El marcador {} nombra otro historial ({}); esta máquina identifica la carpeta como {id}. No se quitó nada: puede ser de otra persona que comparte la carpeta.",
            control.join("config.json").display(),
            config.workspace_id
        )));
    }
    let mut ours = vec![];
    let mut others = vec![];
    for entry in fs::read_dir(&control)? {
        let entry = entry?;
        let name = entry.file_name();
        if (name == "config.json" || name == "kn.lock") && entry.file_type()?.is_file() {
            ours.push(entry.path());
        } else {
            others.push(name.to_string_lossy().into_owned());
        }
    }
    if !others.is_empty() {
        others.sort();
        return Err(Error::Conflict(format!(
            "{} contiene archivos que kn no creó: {}. Muévelos fuera de .kn y vuelve a ejecutar kn migrate; no se quitó nada.",
            control.display(),
            others.join(", ")
        )));
    }
    ours.sort();
    Ok(ours)
}
