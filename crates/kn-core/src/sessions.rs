use crate::{
    error::{Error, Result},
    git::{Engine, Git, git_path, version},
    ops,
    workspace::{Config, Workspace, atomic_json, validate_name},
};
use serde_json::{Value, json};
use std::fs;

pub fn start(ws: &Workspace, name: &str) -> Result<Value> {
    validate_name(name)?;
    ws.claim()?;
    let main = ws.primary_for_write()?;
    let dir = ws
        .home
        .join("sessions")
        .join(ws.config.workspace_id.to_string())
        .join(name);
    if dir.starts_with(&main.root) {
        return Err(Error::Unsafe(
            "Las sesiones deben vivir fuera de la carpeta principal.".into(),
        ));
    }
    if dir.exists() {
        return Err(Error::Invalid(
            "Ya existe una sesión con ese nombre.".into(),
        ));
    }
    fs::create_dir_all(
        dir.parent()
            .ok_or_else(|| Error::Invalid("Ruta de sesión inválida.".into()))?,
    )?;
    let downloaded = observe_external(&main)?;
    main.run(&[
        "worktree",
        "add",
        "-b",
        &format!("sessions/{name}"),
        &git_path(&dir)?,
        "main",
    ])?;
    atomic_json(
        &dir.join(".kn/config.json"),
        &Config {
            schema_version: 2,
            workspace_id: ws.config.workspace_id,
            session: Some(name.into()),
        },
    )?;
    let cloud_only = crate::cloud::pending(&main.root)?;
    let mut message = crate::cloud::with_notice(
        "Sesión creada. Abre esa carpeta para trabajar.",
        &cloud_only,
    );
    if !downloaded.is_empty() {
        message = format!(
            "{message} {} documento(s) descargados de la nube quedaron en su propia versión.",
            downloaded.len()
        );
    }
    Ok(
        json!({"session": name, "path": dir, "cloud_only": cloud_only,
        "downloaded_since_last_observation": downloaded, "message": message}),
    )
}
pub fn list(ws: &Workspace) -> Result<Value> {
    let root = ws
        .home
        .join("sessions")
        .join(ws.config.workspace_id.to_string());
    let mut sessions = vec![];
    if root.is_dir() {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join(".kn/config.json").is_file() {
                sessions.push(
                    json!({"name": entry.file_name().to_string_lossy(), "path": entry.path()}),
                );
            }
        }
    }
    Ok(
        json!({"sessions": sessions, "message": "Sesiones locales; ninguna se sincroniza automáticamente."}),
    )
}
pub fn update(ws: &Workspace) -> Result<Value> {
    ws.require_session()?;
    require_clean(&ws.git)?;
    ops::check_safe(&ws.git)?;
    if ws.git.dir.join("MERGE_HEAD").exists() {
        return Err(Error::Conflict(
            "Hay una integración pendiente en esta sesión.".into(),
        ));
    }
    observe_external(&ws.primary_for_write()?)?;
    let out = ws.git.output(&[
        "merge",
        "--no-overwrite-ignore",
        "--no-edit",
        "-m",
        "Actualizar sesión\n\nKn-Reason: session_update",
        "main",
    ])?;
    if !out.status.success() {
        if ws.git.dir.join("MERGE_HEAD").exists() {
            return Err(Error::Conflict("Hay documentos en conflicto dentro de la sesión. Resuélvelos con Git o usa git merge --abort dentro de la sesión; main no cambió.".into()));
        }
        return Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(
        json!({"version_id": version(&ws.git.head()?), "message": "La sesión incluye la versión principal."}),
    )
}
pub fn finish(ws: &Workspace) -> Result<Value> {
    ws.require_session()?;
    if ws.git.dir.join("MERGE_HEAD").exists() {
        return Err(Error::Conflict(
            "Termina la integración pendiente en la sesión.".into(),
        ));
    }
    // Para quien trabaja en documentos, integrar es guardar: lo pendiente se registra aquí.
    let (_, registrados, _) =
        ops::commit(&ws.git, "Versión antes de integrar", "pre_finish_snapshot")?;
    let main = ws.primary_for_write()?;
    observe_external(&main)?;
    let tip = ws.git.head()?;
    let old = main.head()?;
    let ancestry = main.output(&["merge-base", "--is-ancestor", &old, &tip])?;
    match ancestry.status.code() {
        Some(0) => (),
        Some(1) => return Err(Error::Conflict("La principal avanzó. Ejecuta kn session update en esta sesión, revisa y vuelve a integrar.".into())),
        _ => return Err(Error::Git(String::from_utf8_lossy(&ancestry.stderr).into_owned())),
    }
    ops::protect_untracked(&main, &tip)?;
    main.run(&["merge", "--no-overwrite-ignore", "--ff-only", &tip])?;
    Ok(
        json!({"version_id": version(&tip), "session": ws.config.session,
        "recorded_document_count": registrados,
        "message": "Versión integrada a la principal. La sesión se conserva; no se hizo ningún envío a la nube."}),
    )
}
fn require_clean(git: &Git) -> Result<()> {
    if !ops::changes(git)?.is_empty() {
        return Err(Error::Conflict(
            "Hay documentos sin versión registrada. Regístralos con kn commit y vuelve a intentarlo.".into(),
        ));
    }
    Ok(())
}

/// Observe shared-folder changes as a new baseline without rewriting its documents.
/// The commit identifies kn as the observer, never as the author of external edits.
/// Lo descargado de la nube va antes, en su propia versión; devuelve esas rutas.
fn observe_external(main: &Git) -> Result<Vec<String>> {
    ops::check_safe(main)?;
    let changes = ops::changes(main)?;
    let downloaded = crate::cloud::downloaded(main, &changes)?;
    if changes.is_empty() {
        // Sin registrar aquí, un documento que aparece en la nube sin otros cambios
        // se versionaría al descargarse como external_observation.
        crate::cloud::remember(main, &crate::cloud::pending(&main.root)?)?;
    } else {
        if !downloaded.is_empty() {
            ops::commit_paths(
                main,
                &downloaded,
                "Documentos descargados de la nube",
                "cloud_download",
            )?;
        }
        ops::commit(main, "Cambios externos observados", "external_observation")?;
    }
    require_clean(main)?;
    Ok(downloaded)
}
