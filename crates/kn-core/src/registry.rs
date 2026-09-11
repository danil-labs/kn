//! Registro de principales en `KN_HOME/roots.json`: raíz canónica → `workspace_id`.
//! Es la fuente de identidad de una principal; nada de kn se escribe en su carpeta.
//! Una raíz tiene una sola identidad y una identidad una sola raíz, igual que
//! `location.json`. Las escrituras son atómicas y se hacen bajo `roots.lock`.

use crate::{
    error::{Error, Result},
    git::utf8_path,
    workspace::{acquire_lock, atomic_json},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub const FILE: &str = "roots.json";
const LOCK: &str = "roots.lock";
const SCHEMA: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    schema_version: u32,
    roots: BTreeMap<String, uuid::Uuid>,
}

impl Registry {
    /// Solo lee: sin registro, o sin KN_HOME, no hay raíces registradas.
    pub fn load(home: &Path) -> Result<Self> {
        let bytes = match fs::read(home.join(FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    schema_version: SCHEMA,
                    roots: BTreeMap::new(),
                });
            }
            Err(e) => return Err(e.into()),
        };
        let registry: Self = serde_json::from_slice(&bytes)?;
        if registry.schema_version != SCHEMA {
            return Err(Error::Invalid(format!(
                "El registro de carpetas usa un formato desconocido: {FILE}."
            )));
        }
        Ok(registry)
    }

    pub fn get(&self, root: &Path) -> Option<uuid::Uuid> {
        self.roots.get(root.to_str()?).copied()
    }

    /// Raíces registradas con esa identidad.
    pub fn roots_of(&self, id: uuid::Uuid) -> impl Iterator<Item = PathBuf> + '_ {
        self.roots
            .iter()
            .filter(move |(_, v)| **v == id)
            .map(|(k, _)| PathBuf::from(k))
    }
}

/// Registra `root` con `id` y quita cualquier otra raíz de esa identidad o identidad
/// de esa raíz. Devuelve si cambió algo; sin cambios no escribe.
pub fn register(home: &Path, root: &Path, id: uuid::Uuid) -> Result<bool> {
    let key = utf8_path(root)?.to_owned();
    fs::create_dir_all(home)?;
    let _guard = lock(home)?;
    let mut registry = Registry::load(home)?;
    if registry.roots.get(&key) == Some(&id) && registry.roots_of(id).count() == 1 {
        return Ok(false);
    }
    registry.roots.retain(|_, v| *v != id);
    registry.roots.insert(key, id);
    atomic_json(&home.join(FILE), &registry)?;
    Ok(true)
}

fn lock(home: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join(LOCK))?;
    acquire_lock(file, false)
}
