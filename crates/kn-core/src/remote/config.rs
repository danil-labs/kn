//! Transfer mode and remote configuration. Stored under KN_HOME next to the
//! history: never inside the documents and never synchronized.
use super::profile::Profile;
use crate::{
    error::{Error, Result},
    workspace::{atomic_json, read_json},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Who transfers the documents of this folder. Priority chooses the publisher;
/// it never grants permission to overwrite the other side automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Nobody transfers; configured remotes are not contacted.
    Local,
    /// A desktop client transfers; kn makes no MCP calls.
    DesktopSync,
    /// A desktop client transfers; kn observes through MCP and never writes.
    DesktopSyncObserved,
    /// kn publishes explicitly through MCP; the primary is outside desktop sync.
    Mcp,
}
impl Mode {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "local" => Ok(Self::Local),
            "desktop_sync" => Ok(Self::DesktopSync),
            "desktop_sync_observed" => Ok(Self::DesktopSyncObserved),
            "mcp" => Ok(Self::Mcp),
            _ => Err(Error::Invalid(
                "Modo: local, desktop_sync, desktop_sync_observed o mcp.".into(),
            )),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::DesktopSync => "desktop_sync",
            Self::DesktopSyncObserved => "desktop_sync_observed",
            Self::Mcp => "mcp",
        }
    }
    /// kn may read the active remote through MCP.
    pub fn observes(self) -> bool {
        matches!(self, Self::DesktopSyncObserved | Self::Mcp)
    }
    /// kn is the publisher and may write through MCP.
    pub fn publishes(self) -> bool {
        self == Self::Mcp
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeState {
    pub schema_version: u32,
    pub mode: Mode,
    pub remote: Option<String>,
    pub changed_at: u64,
}
impl Default for ModeState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            mode: Mode::Local,
            remote: None,
            changed_at: 0,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteConfig {
    pub schema_version: u32,
    pub alias: String,
    pub endpoint: String,
    pub root_id: String,
    pub account_ref: Option<String>,
    /// Private copy: editing the source file later does not change this remote.
    pub profile: Profile,
    pub added_at: u64,
}

pub fn dir(common: &Path) -> PathBuf {
    common.join("remote")
}
pub fn remote_dir(common: &Path, alias: &str) -> PathBuf {
    dir(common).join("remotes").join(alias)
}
pub fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty()
        || alias.len() > 64
        || !alias
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(Error::Invalid(
            "Alias de remoto: 1–64 letras ASCII, números, guiones o guiones bajos.".into(),
        ));
    }
    Ok(())
}
pub fn load_mode(common: &Path) -> Result<ModeState> {
    let path = dir(common).join("mode.json");
    if !path.is_file() {
        return Ok(ModeState::default());
    }
    let state: ModeState = read_json(&path)?;
    if state.schema_version != 1 {
        return Err(Error::Invalid("Formato de modo no admitido.".into()));
    }
    Ok(state)
}
pub fn save_mode(common: &Path, state: &ModeState) -> Result<()> {
    atomic_json(&dir(common).join("mode.json"), state)
}
pub fn load_remote(common: &Path, alias: &str) -> Result<RemoteConfig> {
    validate_alias(alias)?;
    let path = remote_dir(common, alias).join("config.json");
    if !path.is_file() {
        return Err(Error::Invalid(format!("No existe el remoto {alias}.")));
    }
    let config: RemoteConfig = read_json(&path)?;
    if config.schema_version != 1 || config.alias != alias {
        return Err(Error::Invalid(format!(
            "Configuración de remoto no admitida: {alias}."
        )));
    }
    config.profile.validate()?;
    Ok(config)
}
pub fn save_remote(common: &Path, config: &RemoteConfig) -> Result<()> {
    atomic_json(
        &remote_dir(common, &config.alias).join("config.json"),
        config,
    )
}
pub fn list_remotes(common: &Path) -> Result<Vec<RemoteConfig>> {
    let root = dir(common).join("remotes");
    let mut out = vec![];
    if root.is_dir() {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.path().join("config.json").is_file() {
                let alias = entry.file_name().to_string_lossy().into_owned();
                out.push(load_remote(common, &alias)?);
            }
        }
    }
    out.sort_by(|a, b| a.alias.cmp(&b.alias));
    Ok(out)
}
/// The remote named by the transfer mode; the only one kn operates.
pub fn active(common: &Path) -> Result<(ModeState, RemoteConfig)> {
    let mode = load_mode(common)?;
    let alias = mode.remote.clone().ok_or(Error::NoRemote)?;
    let remote = load_remote(common, &alias)?;
    Ok((mode, remote))
}

/// Auxiliary and platform-specific: a known sync-client folder name in the path.
/// Absence is never proof that no client synchronizes the folder.
pub fn sync_client_hint(path: &Path) -> Option<String> {
    path.components().find_map(|c| {
        let name = c.as_os_str().to_string_lossy();
        let lower = name.to_lowercase();
        let known = matches!(
            lower.as_str(),
            "google drive"
                | "my drive"
                | "mi unidad"
                | "onedrive"
                | "cloudstorage"
                | "dropbox"
                | "icloud drive"
                | "mobile documents"
                | "sharepoint"
        ) || lower.starts_with("googledrive")
            || lower.starts_with("onedrive - ")
            || lower.starts_with("onedrive-");
        known.then(|| name.into_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_hints_are_auxiliary_name_matches() {
        assert!(
            sync_client_hint(Path::new(
                "/Users/p/Library/CloudStorage/GoogleDrive-p@x.com/My Drive/docs"
            ))
            .is_some()
        );
        assert!(sync_client_hint(Path::new("C:/Users/p/OneDrive - Contoso/docs")).is_some());
        assert!(sync_client_hint(Path::new("/home/p/documentos")).is_none());
        assert_eq!(Mode::parse("mcp").unwrap(), Mode::Mcp);
        assert!(Mode::parse("drive").is_err());
        assert!(!Mode::DesktopSyncObserved.publishes());
        assert!(!Mode::DesktopSync.observes());
    }
}
