//! Documentos cuyo contenido sigue en la nube: el proveedor muestra el archivo
//! pero no lo bajó a disco. Leerlo obliga a descargarlo y, sin el cliente de
//! sincronización, la lectura se agota. Se reconocen por metadatos, sin abrirlos,
//! y se ocultan a Git en cada llamada hasta que el proveedor los descargue.

use crate::{
    error::{Error, Result},
    git::git_path,
};
use std::{fs, io::Write, path::Path};

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
