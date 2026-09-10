//! Publication of a fixed main commit through MCP. Every write carries a revision
//! precondition or a create-only guarantee, and a journal records each operation
//! before and after it is sent. Preconditions protect the files in the plan; they
//! do not make the folder transactional.
use super::{
    config::{self, Mode, remote_dir},
    driver::{Rejection, Remote, WriteOutcome},
    now,
    observe::{self, Entry, Folder, Observation},
    profile::ContentEncoding,
    state::{self, SyncState},
};
use crate::{
    error::{Error, Result},
    git::{Engine, Git, version},
    workspace::{Workspace, atomic_json, read_json},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    CreateFolder,
    Create,
    Update,
    Delete,
}
impl OpKind {
    fn operation(self) -> &'static str {
        match self {
            Self::CreateFolder => "create_folder",
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Planned,
    Sending,
    Applied,
    Rejected,
    Unconfirmed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalStatus {
    InProgress,
    Partial,
    Complete,
    Superseded,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct JournalOp {
    pub kind: OpKind,
    pub path: String,
    pub target_blob: Option<String>,
    pub remote_id: Option<String>,
    pub expected_revision: Option<String>,
    pub status: OpStatus,
    pub result_id: Option<String>,
    pub result_revision: Option<String>,
    pub message: Option<String>,
    /// Set by the first complete observation after the attempt.
    pub visible: Option<bool>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Journal {
    pub schema_version: u32,
    pub operation_id: uuid::Uuid,
    pub alias: String,
    pub target_commit: String,
    pub created_at: u64,
    pub status: JournalStatus,
    pub ops: Vec<JournalOp>,
}
fn dir(common: &Path, alias: &str) -> PathBuf {
    remote_dir(common, alias).join("journal")
}
fn journal_file(common: &Path, alias: &str, id: uuid::Uuid) -> PathBuf {
    dir(common, alias).join(format!("{id}.json"))
}
fn save(common: &Path, journal: &Journal) -> Result<()> {
    atomic_json(
        &journal_file(common, &journal.alias, journal.operation_id),
        journal,
    )
}
pub fn journals(common: &Path, alias: &str) -> Result<Vec<Journal>> {
    let path = dir(common, alias);
    let mut out = vec![];
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|x| x == "json") {
                out.push(read_json::<Journal>(&entry.path())?);
            }
        }
    }
    out.sort_by_key(|j| j.created_at);
    Ok(out)
}
pub fn open_journals(common: &Path, alias: &str) -> Result<Vec<Journal>> {
    Ok(journals(common, alias)?
        .into_iter()
        .filter(|j| matches!(j.status, JournalStatus::InProgress | JournalStatus::Partial))
        .collect())
}

/// Close open journals against a complete observation. A publication whose every
/// operation is visible becomes the reconciled base; otherwise it is superseded and
/// the next plan starts from what the remote shows, so a lost response is never
/// retried blindly: preconditions reject a write that already landed.
pub fn reconcile(git: &Git, alias: &str, entries: &[Entry], folders: &[Folder]) -> Result<()> {
    let files: HashMap<&str, &str> = entries
        .iter()
        .map(|e| (e.path.as_str(), e.blob.as_str()))
        .collect();
    let dirs: HashSet<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    for mut journal in open_journals(&git.common, alias)? {
        let mut all = true;
        for op in &mut journal.ops {
            let visible = match op.kind {
                OpKind::CreateFolder => dirs.contains(op.path.as_str()),
                OpKind::Create | OpKind::Update => {
                    files.get(op.path.as_str()).copied() == op.target_blob.as_deref()
                }
                OpKind::Delete => !files.contains_key(op.path.as_str()),
            };
            all &= visible;
            op.visible = Some(visible);
        }
        journal.status = if all {
            JournalStatus::Complete
        } else {
            JournalStatus::Superseded
        };
        if all && state::is_ancestor(git, &journal.target_commit, &state::main_commit(git)?)? {
            state::write_ref(git, &state::base_ref(alias), &journal.target_commit)?;
        }
        save(&git.common, &journal)?;
    }
    Ok(())
}

fn op(kind: OpKind, path: &str, blob: Option<&String>) -> JournalOp {
    JournalOp {
        kind,
        path: path.into(),
        target_blob: blob.cloned(),
        remote_id: None,
        expected_revision: None,
        status: OpStatus::Planned,
        result_id: None,
        result_revision: None,
        message: None,
        visible: None,
    }
}
/// Operations that turn the observed remote into the target tree. Only documents
/// of that fixed commit are sent; `.kn`, `.git`, KN_HOME and symlinks never are.
fn plan(git: &Git, obs: &Observation, target: &str) -> Result<Vec<JournalOp>> {
    let local = state::tree_files(git, target)?;
    let remote: HashMap<&str, &Entry> = obs.entries.iter().map(|e| (e.path.as_str(), e)).collect();
    let files_lower: HashSet<String> = obs
        .entries
        .iter()
        .map(|e| e.path.to_lowercase())
        .chain(obs.unmanaged.iter().map(|u| u.path.to_lowercase()))
        .collect();
    let mut folders: HashMap<String, String> = obs
        .folders
        .iter()
        .map(|f| (f.path.to_lowercase(), f.path.clone()))
        .collect();
    let collision = |path: &str| {
        Error::Conflict(format!(
            "{path:?} coincide con un elemento remoto que kn no administra o que difiere solo en mayúsculas; no se sobrescribe."
        ))
    };
    let (mut make_dirs, mut writes, mut deletes) = (vec![], vec![], vec![]);
    for (path, blob) in &local {
        match remote.get(path.as_str()) {
            Some(e) if e.blob == *blob => (),
            Some(e) => writes.push(JournalOp {
                remote_id: Some(e.id.clone()),
                expected_revision: Some(e.revision.clone()),
                ..op(OpKind::Update, path, Some(blob))
            }),
            None => {
                if files_lower.contains(&path.to_lowercase()) {
                    return Err(collision(path));
                }
                let parts: Vec<&str> = path.split('/').collect();
                for depth in 1..parts.len() {
                    let dir = parts[..depth].join("/");
                    match folders.get(&dir.to_lowercase()) {
                        Some(existing) if *existing == dir => (),
                        Some(_) => return Err(collision(&dir)),
                        None => {
                            if files_lower.contains(&dir.to_lowercase()) {
                                return Err(collision(&dir));
                            }
                            folders.insert(dir.to_lowercase(), dir.clone());
                            make_dirs.push(op(OpKind::CreateFolder, &dir, None));
                        }
                    }
                }
                writes.push(op(OpKind::Create, path, Some(blob)));
            }
        }
    }
    for (path, e) in &remote {
        if !local.contains_key(*path) {
            deletes.push(JournalOp {
                remote_id: Some(e.id.clone()),
                expected_revision: Some(e.revision.clone()),
                ..op(OpKind::Delete, path, None)
            });
        }
    }
    deletes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(make_dirs.into_iter().chain(writes).chain(deletes).collect())
}

/// Publish main's current commit to the active remote (mode `mcp` only).
pub fn push(ws: &Workspace, dry_run: bool, allow_deletes: bool) -> Result<Value> {
    let git = &ws.git;
    let common = git.common.clone();
    let mode = config::load_mode(&common)?;
    if !mode.mode.publishes() {
        return Err(Error::ModeForbids(format!(
            "En el modo {} kn no publica mediante MCP{}.",
            mode.mode.as_str(),
            match mode.mode {
                Mode::DesktopSync | Mode::DesktopSyncObserved =>
                    ": el cliente de escritorio transfiere los documentos",
                _ => "; elige kn mode set mcp --remote <alias>",
            }
        )));
    }
    let (_, cfg) = config::active(&common)?;
    let alias = cfg.alias.clone();
    let mut remote = super::open(ws, &cfg, true)?;
    let obs = observe::fetch(ws, &cfg, &mut remote)?;
    let snap = state::snapshot(git, &alias)?;
    let cmp = state::compare(git, &snap)?;
    match cmp.state {
        SyncState::Synced => {
            return Ok(
                json!({"remote": alias, "version_id": version(&snap.local), "published": 0,
                "plan": [], "state": cmp.state,
                "message": "El remoto ya refleja la versión principal; no hay nada que publicar."}),
            );
        }
        SyncState::LocalAhead => (),
        SyncState::Unknown => return Err(Error::Conflict(cmp.reason.unwrap_or_default())),
        _ => {
            return Err(Error::Conflict(format!(
                "El remoto tiene cambios que la principal no incluye ({}); incorpóralos con kn pull en una sesión antes de publicar.",
                cmp.to_incorporate.join(", ")
            )));
        }
    }
    let target = snap.local;
    let ops = plan(git, &obs, &target)?;
    // The whole plan is checked before the first write.
    for kind in ops.iter().map(|o| o.kind).collect::<BTreeSet<_>>() {
        remote.require(kind.operation())?;
    }
    if remote.content_encoding() == ContentEncoding::Utf8 {
        for op in &ops {
            if let Some(blob) = &op.target_blob
                && String::from_utf8(git.run(&["cat-file", "blob", blob])?).is_err()
            {
                return Err(Error::Unsupported(format!(
                    "{:?} no es texto UTF-8 y el perfil solo transmite texto.",
                    op.path
                )));
            }
        }
    }
    let deletes: Vec<&str> = ops
        .iter()
        .filter(|o| o.kind == OpKind::Delete)
        .map(|o| o.path.as_str())
        .collect();
    if dry_run {
        return Ok(
            json!({"remote": alias, "dry_run": true, "version_id": version(&target),
            "plan": ops, "requires_allow_deletes": !deletes.is_empty(),
            "message": "Plan de publicación; no se escribió nada en el remoto."}),
        );
    }
    if !deletes.is_empty() && !allow_deletes {
        return Err(Error::Invalid(format!(
            "El plan borra {} documento(s) remoto(s): {}. Revísalo con --dry-run y repite con --allow-deletes si es lo esperado.",
            deletes.len(),
            deletes.join(", ")
        )));
    }
    let mut journal = Journal {
        schema_version: 1,
        operation_id: uuid::Uuid::new_v4(),
        alias: alias.clone(),
        target_commit: target.clone(),
        created_at: now(),
        status: JournalStatus::InProgress,
        ops,
    };
    save(&common, &journal)?;
    let mut folders: HashMap<String, String> = obs
        .folders
        .iter()
        .map(|f| (f.path.clone(), f.id.clone()))
        .collect();
    for i in 0..journal.ops.len() {
        journal.ops[i].status = OpStatus::Sending;
        save(&common, &journal)?;
        let outcome = match execute(git, &mut remote, &journal.ops[i], &folders) {
            Ok(outcome) => outcome,
            Err(e) => return fail(&common, &mut journal, i, OpStatus::Rejected, e),
        };
        let path = journal.ops[i].path.clone();
        match outcome {
            WriteOutcome::Applied { id, revision } => {
                let op = &mut journal.ops[i];
                op.status = OpStatus::Applied;
                op.result_id.clone_from(&id);
                op.result_revision = revision;
                if op.kind == OpKind::CreateFolder {
                    let Some(id) = id else {
                        let e = Error::RemoteUnavailable(format!(
                            "El servidor no devolvió el identificador de la carpeta {path:?}; la próxima observación lo verificará."
                        ));
                        return fail(&common, &mut journal, i, OpStatus::Unconfirmed, e);
                    };
                    folders.insert(path, id);
                }
            }
            WriteOutcome::Rejected { reason, message } => {
                let e = match reason {
                    Rejection::PreconditionFailed
                    | Rejection::AlreadyExists
                    | Rejection::NotFound => Error::Conflict(format!(
                        "El remoto cambió durante la publicación en {path:?}; no se sobrescribió. Ejecuta kn pull en una sesión y vuelve a publicar."
                    )),
                    Rejection::Other => {
                        Error::Remote(format!("El servidor rechazó {path:?}: {message}"))
                    }
                };
                journal.ops[i].message = Some(message);
                return fail(&common, &mut journal, i, OpStatus::Rejected, e);
            }
            WriteOutcome::Unconfirmed(message) => {
                let e = Error::RemoteUnavailable(format!(
                    "No se confirmó {path:?} ({message}). La próxima observación verificará el resultado; kn no repite escrituras a ciegas."
                ));
                journal.ops[i].message = Some(message);
                return fail(&common, &mut journal, i, OpStatus::Unconfirmed, e);
            }
        }
        save(&common, &journal)?;
    }
    // A new complete observation verifies every operation and closes the journal.
    observe::fetch(ws, &cfg, &mut remote)?;
    let journal: Journal = read_json(&journal_file(&common, &alias, journal.operation_id))?;
    if journal.status != JournalStatus::Complete {
        return Err(Error::RemoteIncomplete(
            "Las escrituras terminaron, pero la observación posterior no las muestra todas; ejecuta kn fetch más tarde. No se marcó como publicado."
                .into(),
        ));
    }
    let snap = state::snapshot(git, &alias)?;
    let cmp = state::compare(git, &snap)?;
    Ok(
        json!({"remote": alias, "operation_id": journal.operation_id, "version_id": version(&target),
        "published": journal.ops.len(), "plan": journal.ops,
        "base_version_id": snap.base.as_deref().map(version), "state": cmp.state,
        "to_incorporate": cmp.to_incorporate,
        "message": "Publicación verificada en el remoto."}),
    )
}
fn fail(
    common: &Path,
    journal: &mut Journal,
    i: usize,
    status: OpStatus,
    error: Error,
) -> Result<Value> {
    let op = &mut journal.ops[i];
    op.status = status;
    if op.message.is_none() {
        op.message = Some(error.to_string());
    }
    journal.status = JournalStatus::Partial;
    save(common, journal)?;
    Err(error)
}
fn execute(
    git: &Git,
    remote: &mut Remote,
    op: &JournalOp,
    folders: &HashMap<String, String>,
) -> Result<WriteOutcome> {
    let (parent, name) = op.path.rsplit_once('/').unwrap_or(("", op.path.as_str()));
    let parent_id = || {
        folders
            .get(parent)
            .cloned()
            .ok_or_else(|| Error::Remote(format!("Falta la carpeta remota de {:?}.", op.path)))
    };
    let bytes = || -> Result<Vec<u8>> {
        let blob = op
            .target_blob
            .as_deref()
            .ok_or_else(|| Error::Git("Operación sin contenido.".into()))?;
        git.run(&["cat-file", "blob", blob])
    };
    let id = || {
        op.remote_id
            .as_deref()
            .ok_or_else(|| Error::Remote("Operación sin identificador remoto.".into()))
    };
    let revision = || {
        op.expected_revision
            .as_deref()
            .ok_or_else(|| Error::Remote("Operación sin revisión esperada.".into()))
    };
    match op.kind {
        OpKind::CreateFolder => remote.create_folder(&parent_id()?, name),
        OpKind::Create => remote.create(&parent_id()?, name, &bytes()?),
        OpKind::Update => remote.update(id()?, &bytes()?, revision()?),
        OpKind::Delete => remote.delete(id()?, revision()?),
    }
}
