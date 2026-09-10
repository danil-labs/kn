//! Observation: read the complete remote tree through MCP and import it into
//! reserved Git refs. A fetch never changes main's documents; the base only
//! advances on a reconciliation the refs prove.
use super::{
    config::{RemoteConfig, remote_dir},
    driver::{ItemKind, Remote},
    now, publish, state,
};
use crate::{
    error::{Error, Result},
    git::{Git, git_path, nul_paths},
    workspace::{Workspace, atomic_json, read_json},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

const MAX_PAGES: usize = 100_000;

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub id: String,
    pub revision: String,
    pub blob: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Folder {
    pub path: String,
    pub id: String,
}
/// Visible remotely but never read, written or deleted by kn.
#[derive(Clone, Serialize, Deserialize)]
pub struct Unmanaged {
    pub path: String,
    pub id: String,
    pub reason: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub at: u64,
    pub complete: bool,
    pub issues: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema_version: u32,
    /// Commit of the last complete observation.
    pub commit: Option<String>,
    pub observed_at: Option<u64>,
    pub entries: Vec<Entry>,
    pub folders: Vec<Folder>,
    pub unmanaged: Vec<Unmanaged>,
    pub last_attempt: Option<Attempt>,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            schema_version: 1,
            commit: None,
            observed_at: None,
            entries: vec![],
            folders: vec![],
            unmanaged: vec![],
            last_attempt: None,
        }
    }
}
fn file(common: &Path, alias: &str) -> PathBuf {
    remote_dir(common, alias).join("observation.json")
}
pub fn load(common: &Path, alias: &str) -> Result<Observation> {
    let path = file(common, alias);
    if !path.is_file() {
        return Ok(Observation::default());
    }
    read_json(&path)
}
fn save(common: &Path, alias: &str, obs: &Observation) -> Result<()> {
    atomic_json(&file(common, alias), obs)
}

struct Walk {
    files: Vec<(String, String, String)>,
    folders: Vec<Folder>,
    unmanaged: Vec<Unmanaged>,
    issues: Vec<String>,
}
/// Errors that no retry of the observation can fix.
fn fatal(e: &Error) -> bool {
    matches!(
        e,
        Error::AuthRequired(_) | Error::ProfileMismatch(_) | Error::Unsupported(_)
    )
}
fn shown(prefix: &str) -> String {
    if prefix.is_empty() {
        "la carpeta raíz".into()
    } else {
        format!("{prefix:?}")
    }
}
fn unrepresentable(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        Some("unrepresentable_name")
    } else if lower == ".git" || lower == ".kn" {
        Some("reserved_name")
    } else {
        None
    }
}
/// Complete listing from the root, or the reasons it is incomplete.
fn walk(remote: &mut Remote, root_id: &str) -> Result<Walk> {
    let mut walk = Walk {
        files: vec![],
        folders: vec![Folder {
            path: String::new(),
            id: root_id.into(),
        }],
        unmanaged: vec![],
        issues: vec![],
    };
    let mut queue = VecDeque::from([(root_id.to_owned(), String::new())]);
    let mut seen = HashSet::from([root_id.to_owned()]);
    let mut pages = 0;
    while let Some((folder, prefix)) = queue.pop_front() {
        let mut names: HashMap<String, String> = HashMap::new();
        let mut cursor: Option<String> = None;
        loop {
            pages += 1;
            if pages > MAX_PAGES {
                walk.issues
                    .push("La enumeración remota excede el límite de páginas.".into());
                return Ok(walk);
            }
            let page = match remote.list(&folder, cursor.as_deref()) {
                Ok(page) => page,
                Err(e) if fatal(&e) => return Err(e),
                Err(e) => {
                    walk.issues
                        .push(format!("No se pudo enumerar {}: {e}", shown(&prefix)));
                    return Ok(walk);
                }
            };
            for item in page.items {
                let path = if prefix.is_empty() {
                    item.name.clone()
                } else {
                    format!("{prefix}/{}", item.name)
                };
                if !seen.insert(item.id.clone()) {
                    walk.issues
                        .push(format!("El elemento {} aparece más de una vez.", item.id));
                    continue;
                }
                if let Some(reason) = unrepresentable(&item.name) {
                    walk.unmanaged.push(Unmanaged {
                        path,
                        id: item.id,
                        reason: reason.into(),
                    });
                    continue;
                }
                if let Some(other) = names.insert(item.name.to_lowercase(), item.name.clone()) {
                    walk.issues.push(format!(
                        "{other:?} y {:?} coinciden sin distinguir mayúsculas en {}.",
                        item.name,
                        shown(&prefix)
                    ));
                    continue;
                }
                match item.kind {
                    ItemKind::Folder => {
                        walk.folders.push(Folder {
                            path: path.clone(),
                            id: item.id.clone(),
                        });
                        queue.push_back((item.id, path));
                    }
                    ItemKind::File => {
                        walk.files
                            .push((path, item.id, item.revision.unwrap_or_default()))
                    }
                    ItemKind::RemoteOnly => walk.unmanaged.push(Unmanaged {
                        path,
                        id: item.id,
                        reason: "remote_only".into(),
                    }),
                }
            }
            match page.next_cursor {
                Some(next) if cursor.as_ref() == Some(&next) => {
                    walk.issues
                        .push(format!("La paginación de {} no avanza.", shown(&prefix)));
                    return Ok(walk);
                }
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
    }
    Ok(walk)
}

/// Observe the remote completely and import it as a commit whose parent is the
/// reconciled base, so Git merges against the right ancestor. An incomplete
/// observation is recorded and reported; nothing is inferred from it.
pub fn fetch(ws: &Workspace, cfg: &RemoteConfig, remote: &mut Remote) -> Result<Observation> {
    let git = &ws.git;
    let (common, alias) = (git.common.as_path(), cfg.alias.as_str());
    reconcile_base(git, alias)?;
    let mut obs = load(common, alias)?;
    let walk = walk(remote, &cfg.root_id)?;
    let mut issues = walk.issues;
    let mut unmanaged = walk.unmanaged;
    let mut entries = vec![];
    if issues.is_empty() {
        let ignored = ignored(git, &walk.files)?;
        let previous: HashMap<&str, &Entry> =
            obs.entries.iter().map(|e| (e.id.as_str(), e)).collect();
        for (path, id, revision) in walk.files {
            if ignored.contains(&path) {
                unmanaged.push(Unmanaged {
                    path,
                    id,
                    reason: "ignored_locally".into(),
                });
                continue;
            }
            if let Some(prev) = previous.get(id.as_str())
                && prev.revision == revision
            {
                entries.push(Entry {
                    blob: prev.blob.clone(),
                    path,
                    id,
                    revision,
                });
                continue;
            }
            match remote.read(&id) {
                // The revision bound to these bytes, not the one the listing announced.
                Ok(file) => entries.push(Entry {
                    blob: hash_object(git, &file.bytes)?,
                    path,
                    id,
                    revision: file.revision,
                }),
                Err(e) if fatal(&e) => return Err(e),
                Err(e) => {
                    issues.push(format!("No se pudo leer {path:?}: {e}"));
                    break;
                }
            }
        }
    }
    // A document kn versioned that the remote no longer delivers must not look deleted.
    if issues.is_empty()
        && let Some(base) = state::read_ref(git, &state::base_ref(alias))?
    {
        let known = state::tree_files(git, &base)?;
        for u in &unmanaged {
            if known.contains_key(&u.path) {
                issues.push(format!(
                    "{:?} estaba versionado y el remoto ya no lo entrega como archivo.",
                    u.path
                ));
            }
        }
    }
    if !issues.is_empty() {
        obs.last_attempt = Some(Attempt {
            at: now(),
            complete: false,
            issues: issues.clone(),
        });
        save(common, alias, &obs)?;
        return Err(Error::RemoteIncomplete(format!(
            "Observación remota incompleta; no se infirieron borrados. {}",
            issues.join(" ")
        )));
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    publish::reconcile(git, alias, &entries, &walk.folders)?;
    let files: BTreeMap<String, String> = entries
        .iter()
        .map(|e| (e.path.clone(), e.blob.clone()))
        .collect();
    let main = state::main_commit(git)?;
    let base = state::read_ref(git, &state::base_ref(alias))?;
    let commit = if files == state::tree_files(git, &main)? {
        // Same documents as main: the observation is main itself, reconciled.
        state::write_ref(git, &state::base_ref(alias), &main)?;
        main
    } else {
        match base {
            Some(b) if files == state::tree_files(git, &b)? => b,
            parent => commit_tree(git, &build_tree(git, &entries)?, parent.as_deref(), alias)?,
        }
    };
    state::write_ref(git, &state::observed_ref(alias), &commit)?;
    let at = now();
    let obs = Observation {
        schema_version: 1,
        commit: Some(commit),
        observed_at: Some(at),
        entries,
        folders: walk.folders,
        unmanaged,
        last_attempt: Some(Attempt {
            at,
            complete: true,
            issues: vec![],
        }),
    };
    save(common, alias, &obs)?;
    Ok(obs)
}
/// Persist the base that a finished integration implies (see state::snapshot).
fn reconcile_base(git: &Git, alias: &str) -> Result<()> {
    let snap = state::snapshot(git, alias)?;
    if let Some(base) = &snap.base
        && state::read_ref(git, &state::base_ref(alias))?.as_ref() != Some(base)
    {
        state::write_ref(git, &state::base_ref(alias), base)?;
    }
    Ok(())
}
/// Remote files the local document set ignores stay unmanaged on both sides.
fn ignored(git: &Git, files: &[(String, String, String)]) -> Result<HashSet<String>> {
    if files.is_empty() {
        return Ok(HashSet::new());
    }
    let mut input = vec![];
    for (path, ..) in files {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    // check-ignore rejects pathspec magic; its input paths are matched, not globbed.
    let out = git.output_with(
        &["check-ignore", "-z", "--stdin"],
        &[("GIT_LITERAL_PATHSPECS", "0".into())],
        Some(&input),
    )?;
    match out.status.code() {
        Some(0 | 1) => Ok(nul_paths(&out.stdout)?.into_iter().collect()),
        _ => Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        )),
    }
}
fn hash_object(git: &Git, bytes: &[u8]) -> Result<String> {
    let out = git.run_with(
        &["hash-object", "-w", "--no-filters", "--stdin"],
        &[],
        Some(bytes),
    )?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}
/// Tree from blobs through a temporary index; the session and main indexes are untouched.
fn build_tree(git: &Git, entries: &[Entry]) -> Result<String> {
    let dir = tempfile::tempdir_in(&git.common)?;
    let env = [("GIT_INDEX_FILE", git_path(&dir.path().join("index"))?)];
    if !entries.is_empty() {
        let mut input = vec![];
        for e in entries {
            input.extend_from_slice(format!("100644 {}\t{}", e.blob, e.path).as_bytes());
            input.push(0);
        }
        git.run_with(&["update-index", "-z", "--index-info"], &env, Some(&input))?;
    }
    let out = git.run_with(&["write-tree"], &env, None)?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}
fn commit_tree(git: &Git, tree: &str, parent: Option<&str>, alias: &str) -> Result<String> {
    let message = format!("Observación remota {alias}\n\nKn-Reason: remote_observation");
    let mut args = vec!["commit-tree", tree, "-m", message.as_str()];
    if let Some(parent) = parent {
        args.extend(["-p", parent]);
    }
    git.text(&args)
}
