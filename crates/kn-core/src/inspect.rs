//! Read-only integration contract. Classification outside kn belongs to the caller.
use crate::{
    error::{Error, Result},
    git::version,
    workspace::{Location, Workspace, read_json},
};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderKind {
    Documents,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    Primary,
    Session,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginKind {
    Unknown,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    NotConnected,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sharing {
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct Origin {
    /// Local disk is not evidence that the documents have no cloud origin.
    pub kind: OriginKind,
    pub provider: Option<String>,
    pub connection: Connection,
    pub sharing: Sharing,
}
#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub local_versioning: bool,
    pub sessions: bool,
    pub cloud_connect: bool,
    pub pull: bool,
    pub push: bool,
    pub remote_refresh: bool,
}
#[derive(Debug, Serialize)]
pub struct Inspection {
    pub contract_version: &'static str,
    pub managed: bool,
    pub kind: Option<FolderKind>,
    /// Local workspace identity, NOT a shared cloud-space identity.
    pub workspace_id: Option<uuid::Uuid>,
    pub root: Option<PathBuf>,
    pub primary_root: Option<PathBuf>,
    pub primary_available: Option<bool>,
    pub role: Option<FolderRole>,
    pub session: Option<String>,
    pub version_id: Option<String>,
    pub commit_id: Option<String>,
    pub origin: Option<Origin>,
    /// Implemented features, not permissions or a promise that a mutation will succeed.
    pub capabilities: Capabilities,
    pub message: &'static str,
}

/// Inspect an existing directory. No init, scan, commit, cloud request or filesystem write.
/// Unmanaged directories return Ok(managed=false); damaged managed workspaces return Err.
pub fn inspect(path: &Path) -> Result<Inspection> {
    let ws = match Workspace::open_read_only(path) {
        Ok(ws) => Some(ws),
        Err(Error::NotWorkspace) => None,
        Err(err) => return Err(err),
    };
    let mut info = Inspection {
        contract_version: "1.0",
        managed: ws.is_some(),
        kind: None,
        workspace_id: None,
        root: None,
        primary_root: None,
        primary_available: None,
        role: None,
        session: None,
        version_id: None,
        commit_id: None,
        origin: None,
        capabilities: Capabilities {
            local_versioning: ws.is_some(),
            sessions: ws.is_some(),
            cloud_connect: false,
            pull: false,
            push: false,
            remote_refresh: false,
        },
        message: "Esta carpeta no está administrada por kn.",
    };
    if let Some(ws) = ws {
        let primary = if ws.config.session.is_some() {
            read_json::<Location>(&ws.git.common.join("location.json"))?.root
        } else {
            ws.git.root.clone()
        };
        info.primary_available = Some(primary.is_dir());
        info.primary_root = Some(primary);
        info.root = Some(ws.git.root.clone());
        info.kind = Some(FolderKind::Documents);
        info.workspace_id = Some(ws.config.workspace_id);
        info.role = Some(if ws.config.session.is_some() {
            FolderRole::Session
        } else {
            FolderRole::Primary
        });
        info.session = ws.config.session.clone();
        let sha = ws.git.head()?;
        info.version_id = Some(version(&sha));
        info.commit_id = Some(sha);
        info.origin = Some(Origin {
            kind: OriginKind::Unknown,
            provider: None,
            connection: Connection::NotConnected,
            sharing: Sharing::Unknown,
        });
        info.message = "Documentos administrados por kn; conexión cloud no configurada y origen no verificado.";
    }
    Ok(info)
}
