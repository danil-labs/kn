//! Documentos cuyo contenido sigue en la nube: el proveedor muestra el archivo
//! pero no lo bajó a disco. Leerlo obliga a descargarlo y, sin el cliente de
//! sincronización, la lectura se agota. Se reconocen por metadatos, sin abrirlos,
//! y se ocultan a Git en cada llamada hasta que el proveedor los descargue.
//!
//! La principal recuerda, junto a su historial en KN_HOME, qué seguía en la nube
//! en su última versión: así distingue lo descargado de lo editado por alguien.

use crate::{
    error::{Error, Result},
    git::{Git, git_path},
    ops::Change,
    workspace::{Workspace, atomic_json},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

const RECORD: &str = "cloud-pending.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    cloud_only: Vec<String>,
}

/// Guarda lo que seguía en la nube al registrar una versión de la principal.
/// Las sesiones no tienen documentos en la nube y no escriben el registro.
pub fn remember(git: &Git, paths: &[String]) -> Result<()> {
    if !git.is_primary() {
        return Ok(());
    }
    atomic_json(
        &git.common.join(RECORD),
        &Record {
            schema_version: 1,
            cloud_only: paths.to_vec(),
        },
    )
}

fn recorded(git: &Git) -> Result<BTreeSet<String>> {
    let bytes = match fs::read(git.common.join(RECORD)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(e.into()),
    };
    let record: Record = serde_json::from_slice(&bytes)?;
    if record.schema_version != 1 {
        return Err(Error::Invalid(format!(
            "El registro de documentos en la nube usa un formato desconocido: {RECORD}."
        )));
    }
    Ok(record.cloud_only.into_iter().collect())
}

/// Lo que seguía en la nube en la última versión de la principal y Git ya ve como
/// nuevo: la próxima observación lo versiona como `cloud_download`. Solo lee.
pub fn downloaded(git: &Git, changes: &[Change]) -> Result<Vec<String>> {
    if !git.is_primary() {
        return Ok(vec![]);
    }
    let pending = recorded(git)?;
    Ok(changes
        .iter()
        .filter(|c| c.kind == "added" && pending.contains(&c.path))
        .map(|c| c.path.clone())
        .collect())
}

#[derive(Serialize)]
struct Failure {
    path: String,
    reason: String,
}

/// Descarga los documentos pendientes de la principal leyéndolos enteros, del más
/// pequeño al más grande. No crea versiones: la siguiente observación las crea.
pub fn fetch(start: &Path, timeout: Duration, max_bytes: Option<u64>) -> Result<Value> {
    // Las lecturas pueden tardar minutos; el lock se suelta antes de empezar.
    let root = {
        let ws = Workspace::open(start)?;
        if ws.config.session.is_some() {
            ws.primary()?.root
        } else {
            ws.git.root.clone()
        }
    };
    let mut failed = vec![];
    let mut queue = vec![];
    for rel in pending(&root)? {
        match fs::symlink_metadata(native(&root, &rel)) {
            Ok(meta) => queue.push((meta.len(), rel)),
            Err(e) => failed.push(Failure {
                path: rel,
                reason: e.to_string(),
            }),
        }
    }
    queue.sort();
    let mut fetched = vec![];
    let mut skipped_budget = vec![];
    let mut bytes_fetched: u64 = 0;
    let mut spent: u64 = 0;
    for (size, rel) in queue {
        if !skipped_budget.is_empty()
            || max_bytes.is_some_and(|max| spent.saturating_add(size) > max)
        {
            skipped_budget.push(rel);
            continue;
        }
        spent = spent.saturating_add(size);
        match read_fully(native(&root, &rel), timeout) {
            Ok(bytes) => {
                bytes_fetched = bytes_fetched.saturating_add(bytes);
                fetched.push(rel);
            }
            Err(reason) => failed.push(Failure { path: rel, reason }),
        }
    }
    let remaining = pending(&root)?;
    let message = format!(
        "Descargados: {} ({bytes_fetched} bytes). Fallaron: {}. Fuera del presupuesto: {}. Siguen en la nube: {}.",
        fetched.len(),
        failed.len(),
        skipped_budget.len(),
        remaining.len()
    );
    Ok(
        json!({"fetched": fetched, "failed": failed, "skipped_budget": skipped_budget,
        "remaining": remaining, "bytes_fetched": bytes_fetched, "message": message}),
    )
}

fn native(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// Una lectura agotada no se puede cancelar: su hilo queda suelto y termina con
/// el proceso, para que kn pueda salir aunque el proveedor no conteste.
fn read_fully(path: PathBuf, timeout: Duration) -> std::result::Result<u64, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kn-cloud-fetch".into())
        .spawn(move || {
            let read = fs::File::open(&path)
                .and_then(|mut file| std::io::copy(&mut file, &mut std::io::sink()));
            // Tras un timeout nadie espera el resultado; perderlo es lo previsto.
            tx.send(read).ok();
        })
        .map_err(|e| e.to_string())?;
    match rx.recv_timeout(timeout) {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(e)) => Err(e.to_string()),
        Err(mpsc::RecvTimeoutError::Timeout) => Err("timeout".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("La lectura terminó sin resultado.".into())
        }
    }
}

/// Rutas relativas a `root`, con `/`, de los documentos que siguen en la nube.
pub fn pending(root: &Path) -> Result<Vec<String>> {
    let mut paths = vec![];
    walk(root, root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

/// Los documentos pendientes de una carpeta y la configuración que los oculta a Git.
pub struct Hidden {
    pub paths: Vec<String>,
    excludes: Option<tempfile::NamedTempFile>,
}

impl Hidden {
    pub fn of(root: &Path) -> Result<Hidden> {
        let paths = pending(root)?;
        let excludes = if paths.is_empty() {
            None
        } else {
            let mut file = tempfile::NamedTempFile::new()?;
            for path in &paths {
                writeln!(file, "{}", pattern(path)?)?;
            }
            file.flush()?;
            Some(file)
        };
        Ok(Hidden { paths, excludes })
    }
    pub fn git_config(&self) -> Result<Vec<String>> {
        match &self.excludes {
            None => Ok(vec![]),
            Some(file) => Ok(vec![
                "-c".into(),
                format!("core.excludesFile={}", git_path(file.path())?),
            ]),
        }
    }
}

/// El mensaje humano cuenta los documentos que quedaron fuera.
pub fn with_notice(message: &str, pending: &[String]) -> String {
    match pending.len() {
        0 => message.to_owned(),
        n => format!(
            "{message} {n} documento(s) siguen en la nube; kn los versiona cuando se descarguen."
        ),
    }
}

/// El mensaje humano cuenta lo descargado desde la última versión de la principal.
pub fn with_downloads(message: &str, downloaded: &[String]) -> String {
    match downloaded.len() {
        0 => message.to_owned(),
        n => format!(
            "{message} {n} documento(s) se descargaron de la nube desde la última observación; se versionan aparte."
        ),
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".kn" || name == ".git" {
            continue;
        }
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let rel = relative(root, &path)?;
        if only_in_cloud(&meta) || simulated(&rel) {
            out.push(rel);
        }
    }
    Ok(())
}

fn relative(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| Error::Unsafe("Ruta fuera de carpeta.".into()))?
        .to_str()
        .ok_or_else(|| Error::Unsafe("Hay un nombre que no es UTF-8.".into()))?;
    Ok(if cfg!(windows) {
        rel.replace('\\', "/")
    } else {
        rel.to_owned()
    })
}

/// Un patrón de gitignore anclado que nombra exactamente esa ruta.
fn pattern(rel: &str) -> Result<String> {
    if rel.contains(['\n', '\r']) {
        return Err(Error::Unsafe(format!(
            "Un documento que sigue en la nube lleva un salto de línea en el nombre y Git no puede excluirlo: {rel:?}"
        )));
    }
    let mut out = String::from("/");
    for c in rel.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ' ') {
            out.push('\\');
        }
        out.push(c);
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
fn only_in_cloud(meta: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const SF_DATALESS: u32 = 0x4000_0000;
    meta.st_flags() & SF_DATALESS != 0
}

#[cfg(windows)]
fn only_in_cloud(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x0000_1000;
    const RECALL_ON_OPEN: u32 = 0x0004_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    meta.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(any(target_os = "macos", windows)))]
fn only_in_cloud(_: &fs::Metadata) -> bool {
    false
}

// Solo el proveedor puede marcar un archivo sin datos, así que las regresiones lo
// simulan por nombre. Un build de release no lee esta variable.
#[cfg(debug_assertions)]
fn simulated(rel: &str) -> bool {
    std::env::var("KN_TEST_CLOUD_ONLY").is_ok_and(|v| v.split('\n').any(|p| p == rel))
}

#[cfg(not(debug_assertions))]
fn simulated(_: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::pattern;

    #[cfg(unix)]
    #[test]
    fn a_blocked_read_times_out_and_leaves_the_caller_free() {
        use super::read_fully;
        use std::{
            fs,
            time::{Duration, Instant},
        };
        let tmp = tempfile::tempdir().unwrap();
        let fifo = tmp.path().join("sin escritor");
        assert!(
            crate::git::process("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let start = Instant::now();
        assert_eq!(
            read_fully(fifo.clone(), Duration::from_millis(200)),
            Err("timeout".into())
        );
        assert!(start.elapsed() < Duration::from_secs(5));
        // Abrir el otro extremo desbloquea al lector suelto.
        drop(fs::OpenOptions::new().write(true).open(&fifo).unwrap());
        let doc = tmp.path().join("listo.txt");
        fs::write(&doc, "abc").unwrap();
        assert_eq!(read_fully(doc, Duration::from_secs(5)), Ok(3));
        assert!(read_fully(tmp.path().join("no existe"), Duration::from_secs(5)).is_err());
    }

    #[test]
    fn patterns_name_exactly_one_path() {
        assert_eq!(
            pattern("anexos/informe final.pdf").unwrap(),
            r"/anexos/informe\ final.pdf"
        );
        assert_eq!(pattern("[v2]*?.docx").unwrap(), r"/\[v2]\*\?.docx");
        assert_eq!(pattern(r"a\b").unwrap(), r"/a\\b");
        assert!(pattern("línea\nnueva").is_err());
    }
}
