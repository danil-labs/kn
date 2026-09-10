//! Three-way comparison of documents: B (last reconciled base), L (main) and R
//! (last complete remote observation). Uncommitted work in the primary is separate.
use crate::{
    error::{Error, Result},
    git::{Engine, Git, nul_paths},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub fn observed_ref(alias: &str) -> String {
    format!("refs/kn/remotes/{alias}/observed")
}
pub fn base_ref(alias: &str) -> String {
    format!("refs/kn/remotes/{alias}/base")
}
pub fn read_ref(git: &Git, name: &str) -> Result<Option<String>> {
    let out = git.output(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{name}^{{commit}}"),
    ])?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())),
        Some(1) => Ok(None),
        _ => Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        )),
    }
}
pub fn write_ref(git: &Git, name: &str, commit: &str) -> Result<()> {
    git.run(&["update-ref", name, commit]).map(drop)
}
pub fn delete_ref(git: &Git, name: &str) -> Result<()> {
    if read_ref(git, name)?.is_some() {
        git.run(&["update-ref", "-d", name])?;
    }
    Ok(())
}
pub fn main_commit(git: &Git) -> Result<String> {
    git.text(&["rev-parse", "--verify", "refs/heads/main^{commit}"])
}
pub fn is_ancestor(git: &Git, ancestor: &str, descendant: &str) -> Result<bool> {
    let out = git.output(&["merge-base", "--is-ancestor", ancestor, descendant])?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        )),
    }
}
/// Regular documents of a commit, path → blob. Symlinks and gitlinks are never published.
pub fn tree_files(git: &Git, commit: &str) -> Result<BTreeMap<String, String>> {
    let out = git.run(&["ls-tree", "-r", "-z", "--full-tree", commit])?;
    let bad = || Error::Git("Árbol Git inválido.".into());
    let mut files = BTreeMap::new();
    for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record.iter().position(|b| *b == b'\t').ok_or_else(bad)?;
        let meta = std::str::from_utf8(&record[..tab]).map_err(|_| bad())?;
        let path = String::from_utf8(record[tab + 1..].to_vec())
            .map_err(|_| Error::Unsafe("Hay un nombre que no es UTF-8.".into()))?;
        let mut parts = meta.split(' ');
        let (Some(mode), Some(_), Some(oid)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(bad());
        };
        if mode == "100644" || mode == "100755" {
            files.insert(path, oid.to_owned());
        }
    }
    Ok(files)
}

#[derive(Clone)]
pub struct Snapshot {
    pub base: Option<String>,
    pub local: String,
    pub remote: Option<String>,
}
/// Refs as stored, plus the base a finished integration implies: once main
/// contains the observed commit, that observation is reconciled. Read-only.
pub fn snapshot(git: &Git, alias: &str) -> Result<Snapshot> {
    let local = main_commit(git)?;
    let remote = read_ref(git, &observed_ref(alias))?;
    let mut base = read_ref(git, &base_ref(alias))?;
    if let Some(r) = &remote
        && base.as_ref() != Some(r)
        && is_ancestor(git, r, &local)?
    {
        base = Some(r.clone());
    }
    Ok(Snapshot {
        base,
        local,
        remote,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Synced,
    LocalAhead,
    RemoteAhead,
    Diverged,
    Conflicted,
    Unknown,
}
#[derive(Serialize)]
pub struct Comparison {
    pub state: SyncState,
    pub reason: Option<String>,
    /// Documents whose local version the remote does not have yet.
    pub to_publish: Vec<String>,
    /// Documents whose remote version main does not include yet.
    pub to_incorporate: Vec<String>,
    /// Documents Git would leave in conflict when integrating both sides.
    pub conflicts: Vec<String>,
}
impl Comparison {
    fn only(state: SyncState, reason: Option<&str>) -> Self {
        Self {
            state,
            reason: reason.map(Into::into),
            to_publish: vec![],
            to_incorporate: vec![],
            conflicts: vec![],
        }
    }
}

/// Per document: a difference between L and R is pending publication when L moved
/// away from B, and pending incorporation when R did. Divergence is not conflict:
/// Git decides whether independent changes merge cleanly.
pub fn compare(git: &Git, snap: &Snapshot) -> Result<Comparison> {
    let Some(remote) = &snap.remote else {
        return Ok(Comparison::only(
            SyncState::Unknown,
            Some("Sin observación remota completa; ejecuta kn fetch."),
        ));
    };
    let local = tree_files(git, &snap.local)?;
    let observed = tree_files(git, remote)?;
    if local == observed {
        return Ok(Comparison::only(SyncState::Synced, None));
    }
    let Some(base) = &snap.base else {
        return Ok(Comparison::only(
            SyncState::Unknown,
            Some(
                "Sin base reconciliada: incorpora el remoto con kn pull en una sesión; sin base no se elige un ganador.",
            ),
        ));
    };
    let base = tree_files(git, base)?;
    let paths: BTreeSet<&String> = base
        .keys()
        .chain(local.keys())
        .chain(observed.keys())
        .collect();
    let mut cmp = Comparison::only(SyncState::Synced, None);
    for path in paths {
        let (b, l, r) = (base.get(path), local.get(path), observed.get(path));
        if l != r {
            if l != b {
                cmp.to_publish.push(path.clone());
            }
            if r != b {
                cmp.to_incorporate.push(path.clone());
            }
        }
    }
    cmp.state = if cmp.to_incorporate.is_empty() {
        SyncState::LocalAhead
    } else if cmp.to_publish.is_empty() {
        SyncState::RemoteAhead
    } else {
        cmp.conflicts = merge_conflicts(git, &snap.local, remote)?;
        if cmp.conflicts.is_empty() {
            SyncState::Diverged
        } else {
            SyncState::Conflicted
        }
    };
    Ok(cmp)
}
/// Git's own merge, computed without touching any worktree or index.
fn merge_conflicts(git: &Git, local: &str, remote: &str) -> Result<Vec<String>> {
    let out = git.output(&[
        "merge-tree",
        "--write-tree",
        "--name-only",
        "--no-messages",
        "-z",
        local,
        remote,
    ])?;
    match out.status.code() {
        Some(0) => Ok(vec![]),
        Some(1) => {
            let mut names = nul_paths(&out.stdout)?;
            if !names.is_empty() {
                names.remove(0);
            }
            names.dedup();
            Ok(names)
        }
        _ => Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        )),
    }
}
