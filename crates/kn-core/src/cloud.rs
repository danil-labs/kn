//! Documentos cuyo contenido sigue en la nube: el proveedor muestra el archivo
//! pero no lo bajó a disco. Leerlo obliga a descargarlo y, sin el cliente de
//! sincronización, la lectura se agota. Se reconocen por metadatos, sin abrirlos,
//! y se ocultan a Git en cada llamada hasta que el proveedor los descargue.
//!
//! La principal recuerda, junto a su historial en KN_HOME, qué seguía en la nube
//! en su última versión: así distingue lo descargado de lo editado por alguien.
//! También recuerda qué descargas fallaron, para no reintentarlas en cada pasada.

use crate::{
    error::{Error, Result},
    git::{Git, git_path, nul_paths},
    ops::Change,
    workspace::{Workspace, acquire_lock, atomic_json},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const RECORD: &str = "cloud-pending.json";
const RECORD_LOCK: &str = "cloud-pending.lock";
const FETCH_LOCK: &str = "cloud-fetch.lock";
/// Documentos por tanda con `--all`; entre tandas se vuelve a recorrer la carpeta.
const BATCH: usize = 32;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    cloud_only: Vec<String>,
    /// Ausente en el schema 1.
    #[serde(default)]
    failed: Vec<Remembered>,
}

/// Un documento cuya descarga falló. Las descargas siguientes lo omiten.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Remembered {
    path: String,
    reason: String,
    at: String,
}

fn load(common: &Path) -> Result<Record> {
    let bytes = match fs::read(common.join(RECORD)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Ok(Record {
                schema_version: 2,
                cloud_only: vec![],
                failed: vec![],
            });
        }
        Err(e) => return Err(e.into()),
    };
    let record: Record = serde_json::from_slice(&bytes)?;
    if !matches!(record.schema_version, 1 | 2) {
        return Err(Error::Invalid(format!(
            "El registro de documentos en la nube usa un formato desconocido: {RECORD}."
        )));
    }
    Ok(record)
}

/// Lee, cambia y escribe el registro bajo su propio lock: `fetch` lo actualiza sin
/// el lock del espacio, mientras una observación puede estar escribiéndolo.
fn update(common: &Path, change: impl FnOnce(&mut Record)) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(common.join(RECORD_LOCK))?;
    let _guard = acquire_lock(file, false)?;
    let mut record = load(common)?;
    change(&mut record);
    record.schema_version = 2;
    atomic_json(&common.join(RECORD), &record)
}

/// Guarda lo que seguía en la nube al registrar una versión de la principal.
/// Las sesiones no tienen documentos en la nube y no escriben el registro.
pub fn remember(git: &Git, paths: &[String]) -> Result<()> {
    if !git.is_primary() {
        return Ok(());
    }
    let pending: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    update(&git.common, |record| {
        // Un fallo se olvida cuando el documento ya no sigue en la nube.
        record.failed.retain(|f| pending.contains(f.path.as_str()));
        record.cloud_only = paths.to_vec();
    })
}

fn recorded(git: &Git) -> Result<BTreeSet<String>> {
    Ok(load(&git.common)?.cloud_only.into_iter().collect())
}

/// Los documentos de `cloud_only` cuya última descarga falló. Solo lee.
pub fn failed(git: &Git, cloud_only: &[String]) -> Result<Vec<String>> {
    if !git.is_primary() {
        return Ok(vec![]);
    }
    let pending: BTreeSet<&str> = cloud_only.iter().map(String::as_str).collect();
    Ok(load(&git.common)?
        .failed
        .into_iter()
        .map(|f| f.path)
        .filter(|path| pending.contains(path.as_str()))
        .collect())
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

/// Opciones de `cloud fetch`.
pub struct Fetch {
    pub timeout: Duration,
    pub max_bytes: Option<u64>,
    pub all: bool,
    pub retry_failed: bool,
}

/// Descarga los documentos pendientes de la principal leyéndolos enteros, del más
/// pequeño al más grande. No crea versiones: la siguiente observación las crea.
/// Con `all` sigue por tandas hasta que no quede nada que intentar; cada documento
/// se intenta una vez por ejecución. `progress` recibe una línea por documento.
pub fn fetch(
    start: &Path,
    options: &Fetch,
    progress: &mut dyn FnMut(&Value) -> Result<()>,
) -> Result<Value> {
    // Las lecturas pueden tardar minutos; el lock del espacio se suelta antes de empezar.
    let main = {
        let ws = Workspace::open(start)?;
        if ws.config.session.is_some() {
            ws.primary()?
        } else {
            ws.git.clone()
        }
    };
    let (root, common) = (main.root, main.common);
    let _fetching = fetch_lock(&common)?;
    let remembered: BTreeSet<String> = load(&common)?.failed.into_iter().map(|f| f.path).collect();
    let mut attempted = BTreeSet::new();
    let mut fetched = vec![];
    let mut failed = vec![];
    let mut skipped_budget = vec![];
    let mut bytes_fetched: u64 = 0;
    let mut spent: u64 = 0;
    'run: loop {
        let mut queue: Vec<(u64, String)> = pending_sized(&root)?
            .into_iter()
            .filter(|(rel, _)| {
                !attempted.contains(rel) && (options.retry_failed || !remembered.contains(rel))
            })
            .map(|(rel, size)| (size, rel))
            .collect();
        if queue.is_empty() {
            break;
        }
        queue.sort();
        let mut remaining_count = queue.len();
        let mut bytes_remaining = queue
            .iter()
            .fold(0_u64, |total, (size, _)| total.saturating_add(*size));
        let batch = if options.all { BATCH } else { queue.len() };
        let mut queue = queue.into_iter();
        for _ in 0..batch {
            let Some((size, rel)) = queue.next() else {
                break;
            };
            // Con presupuesto cero no se lee nada: abrir un documento de tamaño aparente
            // cero también pide su descarga al proveedor.
            if options
                .max_bytes
                .is_some_and(|max| max == 0 || spent.saturating_add(size) > max)
            {
                skipped_budget.push(rel);
                skipped_budget.extend(queue.map(|(_, rel)| rel));
                break 'run;
            }
            spent = spent.saturating_add(size);
            remaining_count -= 1;
            bytes_remaining = bytes_remaining.saturating_sub(size);
            attempted.insert(rel.clone());
            let (outcome, bytes) = match read_fully(native(&root, &rel), options.timeout) {
                Ok(bytes) => {
                    bytes_fetched = bytes_fetched.saturating_add(bytes);
                    fetched.push(rel.clone());
                    ("fetched", bytes)
                }
                Err(reason) => {
                    remember_failure(&common, &rel, &reason)?;
                    failed.push(Failure {
                        path: rel.clone(),
                        reason,
                    });
                    ("failed", 0)
                }
            };
            progress(&json!({"path": rel, "outcome": outcome, "bytes": bytes,
                "fetched_count": fetched.len(), "failed_count": failed.len(),
                "remaining_count": remaining_count, "bytes_fetched": bytes_fetched,
                "bytes_remaining": bytes_remaining}))?;
        }
        if !options.all {
            break;
        }
    }
    let remaining = pending(&root)?;
    let still: BTreeSet<&str> = remaining.iter().map(String::as_str).collect();
    let done: BTreeSet<&str> = fetched.iter().map(String::as_str).collect();
    // Un fallo se olvida cuando el documento se descargó o ya no sigue en la nube.
    let settled = |path: &str| !still.contains(path) || done.contains(path);
    if load(&common)?.failed.iter().any(|f| settled(&f.path)) {
        update(&common, |record| {
            record.failed.retain(|f| !settled(&f.path))
        })?;
    }
    let omitted = if options.retry_failed {
        0
    } else {
        remembered
            .iter()
            .filter(|path| still.contains(path.as_str()))
            .count()
    };
    let message = format!(
        "Descargados: {} ({bytes_fetched} bytes). Fallaron: {}. Fuera del presupuesto: {}. Omitidos por fallos anteriores: {omitted}. Siguen en la nube: {}.",
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

fn remember_failure(common: &Path, path: &str, reason: &str) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let entry = Remembered {
        path: path.to_owned(),
        reason: reason.to_owned(),
        at: rfc3339(now),
    };
    update(common, |record| {
        record.failed.retain(|f| f.path != path);
        record.failed.push(entry);
        record.failed.sort_by(|a, b| a.path.cmp(&b.path));
    })
}

/// Dos descargas del mismo historial no corren a la vez; la segunda no espera.
fn fetch_lock(common: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(common.join(FETCH_LOCK))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(file),
        Err(e)
            if e.kind() == ErrorKind::WouldBlock
                || e.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            Err(Error::FetchBusy)
        }
        Err(e) => Err(e.into()),
    }
}

/// Segundos desde 1970 en RFC 3339, UTC. Los días se convierten con el algoritmo
/// `civil_from_days` de Howard Hinnant, para no depender de un crate de fechas.
fn rfc3339(secs: u64) -> String {
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
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
    Ok(pending_sized(root)?
        .into_iter()
        .map(|(rel, _)| rel)
        .collect())
}

/// Como `pending`, con el tamaño lógico de cada documento. Leer los metadatos de
/// un documento sin datos no pide su descarga.
pub fn pending_sized(root: &Path) -> Result<Vec<(String, u64)>> {
    let mut out = vec![];
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

/// Los documentos pendientes de una carpeta y la configuración que los oculta a Git.
pub struct Hidden {
    pub paths: Vec<String>,
    excludes: Option<tempfile::NamedTempFile>,
}

impl Hidden {
    /// Los nuevos se ocultan con un `core.excludesFile` de esta llamada. Los ya
    /// versionados, que los patrones de ignore no alcanzan, quedan marcados
    /// `assume-unchanged` en el índice de `git`: ver `mark`.
    pub fn of(git: &Git) -> Result<Hidden> {
        let paths = pending(&git.root)?;
        let (set, clear) = plan(git, None, &paths)?;
        apply(git, None, &set, &clear)?;
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

/// Un índice temporal para lecturas con el lock compartido, que no pueden escribir
/// el índice propio: la copia lleva las marcas de `mark` que a este le faltan.
pub struct Overlay {
    index: Option<tempfile::TempPath>,
}

impl Overlay {
    pub fn of(git: &Git) -> Result<Overlay> {
        let (set, clear) = plan(git, None, &pending(&git.root)?)?;
        if set.is_empty() && clear.is_empty() {
            return Ok(Overlay { index: None });
        }
        let index = tempfile::NamedTempFile::new()?.into_temp_path();
        fs::copy(git.dir.join("index"), &index)?;
        apply(git, Some(&index), &set, &clear)?;
        Ok(Overlay { index: Some(index) })
    }
    /// El índice que deben usar las llamadas; `None` es el propio.
    pub fn index(&self) -> Option<&Path> {
        self.index.as_deref()
    }
}

/// Un documento versionado que el proveedor liberó ya está entero en el historial,
/// pero su ctime cambió y Git lo relee para indexarlo: con el proveedor sin
/// entregarlo, la lectura se agota y falla toda la orden. Se marca
/// `assume-unchanged` mientras siga en la nube: Git lo da por igual a HEAD sin
/// leerlo en status, add, diff, commit ni merge. La marca vive en el índice, así
/// que la respetan todas las órdenes sin repetir rutas en cada una; cada llamada
/// la reconcilia con lo que hay en la nube y la quita en cuanto se descarga. Sin
/// la marca Git compara el contenido: igual al versionado no es un cambio; editado
/// en la nube, es una modificación normal.
///
/// Devuelve qué rutas marcar y cuáles desmarcar en `index` (el propio si es `None`).
fn plan(git: &Git, index: Option<&Path>, pending: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let pending: BTreeSet<&str> = pending.iter().map(String::as_str).collect();
    let listed = git.run_on(index, &["ls-files", "-v", "-z"], &[])?;
    let (mut set, mut clear) = (vec![], vec![]);
    for record in nul_paths(&listed)? {
        // `-v` antepone una letra y un espacio; minúscula si tiene la marca.
        let Some((tag, path)) = record.split_at_checked(2) else {
            return Err(Error::Git("Listado del índice inválido.".into()));
        };
        // Un documento en conflicto no se marca: se resuelve con Git.
        if tag.eq_ignore_ascii_case("m ") {
            continue;
        }
        let marked = tag.starts_with(|c: char| c.is_ascii_lowercase());
        match (pending.contains(path), marked) {
            (true, false) => set.push(path.to_owned()),
            (false, true) => clear.push(path.to_owned()),
            _ => (),
        }
    }
    Ok((set, clear))
}

fn apply(git: &Git, index: Option<&Path>, set: &[String], clear: &[String]) -> Result<()> {
    for (flag, paths) in [
        ("--assume-unchanged", set),
        ("--no-assume-unchanged", clear),
    ] {
        if paths.is_empty() {
            continue;
        }
        let mut input = vec![];
        for path in paths {
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
        git.run_on(index, &["update-index", flag, "-z", "--stdin"], &input)?;
    }
    Ok(())
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

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) -> Result<()> {
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
            out.push((rel, meta.len()));
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
    use super::{pattern, rfc3339};

    #[test]
    fn timestamps_are_rfc3339_in_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    }

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
