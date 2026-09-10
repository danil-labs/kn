use crate::{
    error::{Error, Result},
    git::{Engine, Git, nul_paths, version},
    workspace::Workspace,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{fs, path::Path};

#[derive(Serialize)]
pub struct Change {
    pub path: String,
    pub kind: &'static str,
    pub status: String,
}

pub fn changes(git: &Git) -> Result<Vec<Change>> {
    let out = git.run(&[
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
    ])?;
    nul_paths(&out)?
        .into_iter()
        .map(|record| {
            if record.len() < 4 || !record.is_char_boundary(3) {
                return Err(Error::Git("Estado Git inválido.".into()));
            }
            let code = &record[..2];
            let kind = if code == "??" || code.contains('A') {
                "added"
            } else if code.contains('D') {
                "deleted"
            } else {
                "modified"
            };
            Ok(Change {
                path: record[3..].into(),
                kind,
                status: code.into(),
            })
        })
        .collect()
}
fn inspect(
    root: &Path,
    dir: &Path,
    unsafe_paths: &mut Vec<String>,
    empty: &mut Vec<String>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".kn" || name == ".git" {
            continue;
        }
        let p = entry.path();
        let kind = entry.file_type()?;
        let rel = p
            .strip_prefix(root)
            .map_err(|_| Error::Unsafe("Ruta fuera de carpeta.".into()))?
            .to_string_lossy()
            .into_owned();
        if kind.is_symlink() {
            let target = fs::read_link(&p)?;
            if target.is_absolute()
                || target.has_root()
                || target
                    .components()
                    .any(|c| matches!(c, std::path::Component::Prefix(_)))
                || !fs::canonicalize(&p).is_ok_and(|target| target.starts_with(root))
            {
                unsafe_paths.push(rel);
            }
        } else if kind.is_dir() {
            if fs::read_dir(&p)?.next().is_none() {
                empty.push(rel);
            }
            inspect(root, &p, unsafe_paths, empty)?;
        }
    }
    Ok(())
}
pub fn check_safe(git: &Git) -> Result<()> {
    let mut unsafe_paths = vec![];
    inspect(&git.root, &git.root, &mut unsafe_paths, &mut vec![])?;
    if !unsafe_paths.is_empty() {
        return Err(Error::Unsafe(format!(
            "Enlaces absolutos, externos o rotos: {}",
            unsafe_paths.join(", ")
        )));
    }
    // Reject nested repositories: Git otherwise stores gitlinks without their document content.
    fn nested(dir: &Path) -> Result<()> {
        for e in fs::read_dir(dir)? {
            let e = e?;
            if e.file_name() == ".git" || e.file_name() == ".kn" {
                continue;
            }
            if e.file_type()?.is_dir() {
                if e.path().join(".git").exists() {
                    return Err(Error::Unsafe(
                        "Hay un repositorio anidado; sepáralo antes de guardar.".into(),
                    ));
                }
                nested(&e.path())?;
            }
        }
        Ok(())
    }
    nested(&git.root)
}
pub fn status(ws: &Workspace) -> Result<Value> {
    let local = changes(&ws.git)?;
    let mut unsafe_paths = vec![];
    let mut empty = vec![];
    inspect(&ws.git.root, &ws.git.root, &mut unsafe_paths, &mut empty)?;
    let conflicts = nul_paths(
        &ws.git
            .run(&["diff", "--name-only", "--diff-filter=U", "-z"])?,
    )?;
    Ok(
        json!({"workspace_id": ws.config.workspace_id, "session": ws.config.session,
        "clean": local.is_empty(), "local_changes": local, "pending_sync": [], "conflicts": conflicts,
        "unsafe_paths": unsafe_paths, "unversioned_empty_folders": empty,
        "existing_user_git": ws.config.session.is_none() && ws.git.root.join(".git").exists(),
        "capabilities": {"local_versioning": true, "sessions": true, "cloud_connect": false,
            "pull": false, "push": false, "remote_refresh": false},
        "last_remote_observed": null, "remote_freshness": "unknown", "sync_baseline_id": null,
        "latest_version_id": version(&ws.git.head()?),
        "message": if ws.config.session.is_some() { "Sesión local; los cambios todavía no se publican." }
            else { "Carpeta compartida. Los cambios manuales se reconocen; abre una sesión para trabajar con agentes." }}),
    )
}
pub fn diff(ws: &Workspace, patch: bool) -> Result<Value> {
    let changes = changes(&ws.git)?;
    let patch_text = if patch {
        Some(
            ws.git
                .text(&["diff", "--no-ext-diff", "--no-textconv", "HEAD", "--"])?,
        )
    } else {
        None
    };
    Ok(
        json!({"base_version_id": version(&ws.git.head()?), "base_label": "latest_snapshot",
        "changes": changes, "patch": patch_text,
        "message": "Cambios desde la última versión. El parche muestra documentos ya versionados."}),
    )
}
pub fn snapshot(ws: &Workspace, message: &str) -> Result<Value> {
    ws.require_session()?;
    let (sha, count, created) = commit(&ws.git, message, "manual_snapshot")?;
    Ok(
        json!({"version_id": version(&sha), "changed_document_count": count, "created": created,
        "timestamp": ws.git.text(&["show", "-s", "--format=%cI", &sha])?, "reason": "manual_snapshot",
        "message": if created { "Versión guardada en la sesión." } else { "No hay cambios que guardar." }}),
    )
}
pub fn commit(git: &Git, message: &str, reason: &str) -> Result<(String, usize, bool)> {
    check_safe(git)?;
    let unresolved = git.run(&["diff", "--name-only", "--diff-filter=U", "-z"])?;
    if !unresolved.is_empty() {
        return Err(Error::Conflict(
            "Resuelve los documentos y marca la resolución con Git antes de guardar.".into(),
        ));
    }
    git.run(&["add", "-A", "--", "."])?;
    let count =
        nul_paths(&git.run(&["diff", "--cached", "--name-only", "-z", "HEAD", "--"])?)?.len();
    let merging = git.dir.join("MERGE_HEAD").exists();
    if count == 0 && !merging {
        return Ok((git.head()?, 0, false));
    }
    git.run(&[
        "commit",
        "-m",
        if message.is_empty() {
            "Versión guardada"
        } else {
            message
        },
        "-m",
        &format!("Kn-Reason: {reason}"),
    ])?;
    Ok((git.head()?, count, true))
}
pub fn history(ws: &Workspace, limit: usize, offset: usize) -> Result<Value> {
    if limit == 0 || limit > 1000 {
        return Err(Error::Invalid("--limit debe estar entre 1 y 1000.".into()));
    }
    let ids = ws.git.text(&["rev-list", "--first-parent", "HEAD"])?;
    let mut versions = vec![];
    let mut visible = 0;
    let mut more = false;
    for sha in ids.lines() {
        let count = nul_paths(&ws.git.run(&[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-only",
            "-z",
            "-r",
            "-m",
            "--first-parent",
            sha,
        ])?)?
        .len();
        if count == 0 {
            continue;
        }
        if visible < offset {
            visible += 1;
            continue;
        }
        if versions.len() == limit {
            more = true;
            break;
        }
        versions.push(json!({"id": version(sha), "timestamp": ws.git.text(&["show", "-s", "--format=%cI", sha])?,
            "message": ws.git.text(&["show", "-s", "--format=%s", sha])?,
            "reason": ws.git.text(&["show", "-s", "--format=%(trailers:key=Kn-Reason,valueonly)", sha])?,
            "changed_document_count": count}));
    }
    Ok(
        json!({"versions": versions, "next_offset": if more { Some(offset + limit) } else { None },
        "message": "Historial de versiones de esta carpeta."}),
    )
}
pub fn restore(ws: &Workspace, id: &str) -> Result<Value> {
    ws.require_session()?;
    if id.len() != 14
        || !id.starts_with("v_")
        || !id[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid(
            "Usa una versión de kn history: v_ y 12 caracteres hexadecimales.".into(),
        ));
    }
    let target = ws
        .git
        .text(&["rev-parse", "--verify", &format!("{}^{{commit}}", &id[2..])])?;
    if !ws
        .git
        .output(&["merge-base", "--is-ancestor", &target, "HEAD"])?
        .status
        .success()
    {
        return Err(Error::Invalid(
            "La versión no pertenece al historial de esta sesión.".into(),
        ));
    }
    let (before, _, _) = commit(&ws.git, "Antes de restaurar", "pre_restore_snapshot")?;
    protect_untracked(&ws.git, &target)?;
    // Two-tree checkout delegates tracked deletions to Git after protecting ignored obstructions.
    // Unlike checkout + filesystem sweep, it preserves ignored and unrelated files.
    ws.git.run(&["read-tree", "-m", "-u", &before, &target])?;
    let (after, count, created) = commit(&ws.git, &format!("Restaurar {id}"), "restore")?;
    Ok(
        json!({"restored_version_id": id, "new_version_id": version(&after),
        "pre_restore_snapshot_id": version(&before), "sync_baseline_id": null,
        "changed_document_count": count, "created": created,
        "message": "Restauración guardada en la sesión; la carpeta principal sigue igual."}),
    )
}

/// Git may overwrite ignored files on checkout; preserve every untracked obstruction.
pub fn protect_untracked(git: &Git, target: &str) -> Result<()> {
    let untracked = nul_paths(&git.run(&["ls-files", "--others", "-z"])?)?;
    let target_paths = nul_paths(&git.run(&["ls-tree", "-r", "--name-only", "-z", target])?)?;
    for existing in &untracked {
        for wanted in &target_paths {
            if existing == wanted
                || existing.starts_with(&format!("{wanted}/"))
                || wanted.starts_with(&format!("{existing}/"))
            {
                return Err(Error::Conflict(format!(
                    "Un archivo sin versionar ocupa una ruta necesaria: {existing:?}. Consérvalo fuera de esa ruta antes de continuar."
                )));
            }
        }
    }
    Ok(())
}
