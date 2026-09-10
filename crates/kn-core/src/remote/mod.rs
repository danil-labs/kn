//! Remotos documentales mediante MCP. kn actúa como cliente: un programa decide
//! cada operación con reglas explícitas y respuestas verificadas; ningún modelo de
//! lenguaje interviene. Git sigue siendo el único motor de versiones y merge.
pub mod config;
pub mod credentials;
pub mod driver;
pub mod http;
pub mod mcp;
pub mod oauth;
pub mod observe;
pub mod profile;
pub mod publish;
pub mod state;

use crate::{
    error::{Error, Result},
    git::{Engine, nul_paths, version},
    ops, sessions,
    workspace::Workspace,
};
use config::{Mode, ModeState, RemoteConfig};
use credentials::{ACCESS_TOKEN_ENV, CredentialStore, Keychain, credential_key};
use driver::{ItemKind, Remote};
use serde_json::{Value, json};
use state::{Comparison, SyncState};
use std::{fs, path::Path, time::Duration};

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// RFC 3339 UTC timestamp, without a date dependency (Hinnant's civil_from_days).
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Connect with the credential the environment or the secure store provides.
pub(crate) fn open(ws: &Workspace, cfg: &RemoteConfig, allow_writes: bool) -> Result<Remote> {
    let http = http::Http::new();
    let token = access_token(ws, cfg, &http, &Keychain)?;
    Remote::open(
        http,
        &cfg.endpoint,
        token,
        cfg.profile.clone(),
        allow_writes,
    )
}
fn access_token(
    ws: &Workspace,
    cfg: &RemoteConfig,
    http: &http::Http,
    store: &dyn CredentialStore,
) -> Result<Option<String>> {
    if let Some(token) = std::env::var_os(ACCESS_TOKEN_ENV).filter(|t| !t.is_empty()) {
        return token
            .into_string()
            .map(Some)
            .map_err(|_| Error::Invalid(format!("{ACCESS_TOKEN_ENV} no es texto válido.")));
    }
    let key = credential_key(ws.config.workspace_id, &cfg.alias);
    let Some(mut tokens) = store.load(&key)? else {
        return Ok(None);
    };
    if tokens.expires_soon(now()) {
        tokens = oauth::refresh(http, &tokens)?;
        store.save(&key, &tokens)?;
    }
    Ok(Some(tokens.access_token))
}
fn require_observing(mode: &ModeState) -> Result<()> {
    match mode.mode {
        Mode::Local => Err(Error::ModeForbids(
            "En modo local kn no contacta remotos. Elige quién transfiere con kn mode set desktop_sync_observed o mcp --remote <alias>."
                .into(),
        )),
        Mode::DesktopSync => Err(Error::ModeForbids(
            "En desktop_sync kn no usa MCP: el cliente de escritorio transfiere. Usa desktop_sync_observed para observar el remoto."
                .into(),
        )),
        Mode::DesktopSyncObserved | Mode::Mcp => Ok(()),
    }
}
fn active_observed(ws: &Workspace) -> Result<RemoteConfig> {
    let common = &ws.git.common;
    require_observing(&config::load_mode(common)?)?;
    Ok(config::active(common)?.1)
}

pub fn add(
    ws: &Workspace,
    alias: &str,
    endpoint: &str,
    profile: &Path,
    root_id: &str,
    account_ref: Option<&str>,
) -> Result<Value> {
    config::validate_alias(alias)?;
    http::validate_url(endpoint)?;
    if root_id.is_empty() || root_id.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "--root debe identificar la carpeta remota.".into(),
        ));
    }
    let common = &ws.git.common;
    if config::remote_dir(common, alias)
        .join("config.json")
        .exists()
    {
        return Err(Error::Invalid(format!("Ya existe el remoto {alias}.")));
    }
    let profile = profile::Profile::parse(&fs::read(profile)?)?;
    let cfg = RemoteConfig {
        schema_version: 1,
        alias: alias.into(),
        endpoint: endpoint.into(),
        root_id: root_id.into(),
        account_ref: account_ref.map(Into::into),
        profile,
        added_at: now(),
    };
    config::save_remote(common, &cfg)?;
    Ok(
        json!({"alias": alias, "endpoint": endpoint, "root_id": root_id, "profile": cfg.profile.name,
        "message": "Remoto configurado sin contactar al servidor. Autoriza con kn remote login y comprueba con kn remote verify."}),
    )
}
pub fn list(ws: &Workspace) -> Result<Value> {
    let common = &ws.git.common;
    let mode = config::load_mode(common)?;
    let remotes: Vec<Value> = config::list_remotes(common)?
        .iter()
        .map(|r| {
            json!({"alias": r.alias, "endpoint": r.endpoint, "root_id": r.root_id,
                "profile": r.profile.name, "active": mode.remote.as_deref() == Some(r.alias.as_str())})
        })
        .collect();
    Ok(json!({"remotes": remotes, "mode": mode.mode,
        "message": "Remotos configurados; kn solo opera el remoto del modo."}))
}
pub fn show(ws: &Workspace, alias: &str) -> Result<Value> {
    let common = &ws.git.common;
    let cfg = config::load_remote(common, alias)?;
    let obs = observe::load(common, alias)?;
    Ok(
        json!({"alias": alias, "endpoint": cfg.endpoint, "root_id": cfg.root_id,
        "account_ref": cfg.account_ref,
        "profile": {"name": cfg.profile.name, "description": cfg.profile.description,
            "operations": cfg.profile.operations.keys().collect::<Vec<_>>()},
        "observed_at": obs.observed_at.map(rfc3339), "documents": obs.entries.len(),
        "unmanaged": obs.unmanaged, "last_attempt": obs.last_attempt.as_ref().map(attempt),
        "open_publications": publish::open_journals(common, alias)?.len(),
        "message": "Configuración y último estado conocido; no se contactó al servidor."}),
    )
}
fn attempt(a: &observe::Attempt) -> Value {
    json!({"at": rfc3339(a.at), "complete": a.complete, "issues": a.issues})
}
/// Tools, pinned schemas and root folder against the profile. Never writes remotely.
pub fn verify(ws: &Workspace, alias: &str) -> Result<Value> {
    let cfg = config::load_remote(&ws.git.common, alias)?;
    let mut remote = open(ws, &cfg, false)?;
    let root = if remote.require("identify").is_ok() {
        let item = remote.identify(&cfg.root_id)?;
        if item.kind != ItemKind::Folder {
            return Err(Error::Invalid(
                "El destino remoto no es una carpeta.".into(),
            ));
        }
        Some(json!({"id": item.id, "name": item.name}))
    } else {
        None
    };
    let caps = remote.capabilities();
    let ready = |ops: &[&str]| {
        ops.iter()
            .all(|op| caps.iter().any(|c| c.operation == *op && c.available))
    };
    Ok(
        json!({"alias": alias, "server": remote.server(), "protocol_version": remote.protocol(),
        "root": root, "capabilities": caps,
        "observation_ready": ready(&["list", "read"]),
        "publication_ready": ready(&["update", "create", "create_folder", "delete"]),
        "certified": false,
        "message": "Capacidades comprobadas contra el perfil. Certificar un proveedor exige además pruebas reales con dos cuentas."}),
    )
}
pub fn login(
    ws: &Workspace,
    alias: &str,
    store: &dyn CredentialStore,
    open_browser: &dyn Fn(&str) -> Result<()>,
) -> Result<Value> {
    let cfg = config::load_remote(&ws.git.common, alias)?;
    let tokens = oauth::login(
        &http::Http::new(),
        &cfg.endpoint,
        open_browser,
        Duration::from_secs(300),
    )?;
    store.save(&credential_key(ws.config.workspace_id, alias), &tokens)?;
    Ok(
        json!({"alias": alias, "expires_at": tokens.expires_at.map(rfc3339),
        "refreshable": tokens.refresh_token.is_some(),
        "message": "Autorización guardada en el almacén seguro del sistema; kn no la escribe en KN_HOME ni en los documentos."}),
    )
}
pub fn logout(ws: &Workspace, alias: &str, store: &dyn CredentialStore) -> Result<Value> {
    config::load_remote(&ws.git.common, alias)?;
    let removed = store.delete(&credential_key(ws.config.workspace_id, alias))?;
    Ok(json!({"alias": alias, "removed": removed,
        "message": if removed { "Credencial eliminada del almacén del sistema." }
            else { "No había una credencial guardada para este remoto." }}))
}
pub fn remove(ws: &Workspace, alias: &str, store: &dyn CredentialStore) -> Result<Value> {
    let common = &ws.git.common;
    config::load_remote(common, alias)?;
    if config::load_mode(common)?.remote.as_deref() == Some(alias) {
        return Err(Error::Invalid(format!(
            "{alias} es el remoto activo; cambia el modo antes de quitarlo."
        )));
    }
    if !publish::open_journals(common, alias)?.is_empty() {
        return Err(Error::Conflict(
            "Hay una publicación sin verificar en este remoto; actívalo y ejecuta kn fetch antes de quitarlo."
                .into(),
        ));
    }
    let credential = match store.delete(&credential_key(ws.config.workspace_id, alias)) {
        Ok(removed) => json!({"removed": removed}),
        Err(e) => json!({"removed": false, "error": e.to_string()}),
    };
    state::delete_ref(&ws.git, &state::observed_ref(alias))?;
    state::delete_ref(&ws.git, &state::base_ref(alias))?;
    fs::remove_dir_all(config::remote_dir(common, alias))?;
    Ok(json!({"alias": alias, "credential": credential,
        "message": "Remoto quitado. Las versiones locales se conservan."}))
}

fn publisher(mode: Mode) -> &'static str {
    match mode {
        Mode::Local => "none",
        Mode::DesktopSync | Mode::DesktopSyncObserved => "desktop_client",
        Mode::Mcp => "kn",
    }
}
pub fn mode(ws: &Workspace) -> Result<Value> {
    let state = config::load_mode(&ws.git.common)?;
    Ok(
        json!({"mode": state.mode, "remote": state.remote, "publisher": publisher(state.mode),
        "changed_at": (state.changed_at > 0).then(|| rfc3339(state.changed_at)),
        "message": format!("Modo {}: {}.", state.mode.as_str(), match state.mode {
            Mode::Local => "nadie transfiere; kn no contacta remotos",
            Mode::DesktopSync => "el cliente de escritorio transfiere; kn no usa MCP",
            Mode::DesktopSyncObserved => "el cliente de escritorio transfiere; kn observa mediante MCP sin escribir",
            Mode::Mcp => "kn publica mediante MCP con operaciones explícitas",
        })}),
    )
}
/// Explicit transition. Changing mode never transfers documents and never
/// turns an incomplete observation into a base.
pub fn set_mode(
    ws: &Workspace,
    mode: Mode,
    alias: Option<&str>,
    primary_outside_sync: bool,
) -> Result<Value> {
    let common = &ws.git.common;
    let previous = config::load_mode(common)?;
    let remote = match mode {
        Mode::Local | Mode::DesktopSync => {
            if alias.is_some() {
                return Err(Error::Invalid(format!(
                    "El modo {} no usa un remoto MCP.",
                    mode.as_str()
                )));
            }
            None
        }
        Mode::DesktopSyncObserved | Mode::Mcp => {
            let alias = alias
                .ok_or_else(|| Error::Invalid("Indica el remoto con --remote <alias>.".into()))?;
            config::load_remote(common, alias)?;
            Some(alias.to_owned())
        }
    };
    for r in config::list_remotes(common)? {
        if !publish::open_journals(common, &r.alias)?.is_empty() {
            return Err(Error::Conflict(format!(
                "Hay una publicación sin verificar en {}; ejecuta kn fetch antes de cambiar de modo.",
                r.alias
            )));
        }
    }
    if mode.observes()
        && let Some(hint) = config::sync_client_hint(&ws.home)
    {
        return Err(Error::ModeForbids(format!(
            "KN_HOME parece estar en una carpeta sincronizada ({hint}); muévelo fuera antes de usar remotos."
        )));
    }
    if mode == Mode::Mcp {
        let primary = ws.primary()?.root;
        if let Some(hint) = config::sync_client_hint(&primary) {
            return Err(Error::ModeForbids(format!(
                "La principal parece estar dentro de una carpeta sincronizada ({hint}). Muévela fuera, ejecuta kn init en la nueva ubicación y vuelve a cambiar el modo; la transición queda pendiente."
            )));
        }
        if !primary_outside_sync {
            return Err(Error::Invalid(
                "Confirma con --primary-outside-sync que ningún cliente de escritorio sincroniza la principal; kn no puede detener a ese cliente."
                    .into(),
            ));
        }
    }
    config::save_mode(
        common,
        &ModeState {
            schema_version: 1,
            mode,
            remote: remote.clone(),
            changed_at: now(),
        },
    )?;
    Ok(
        json!({"mode": mode, "remote": remote, "previous_mode": previous.mode,
        "previous_remote": previous.remote, "publisher": publisher(mode),
        "message": "Modo actualizado. No se transfirió ningún documento."}),
    )
}

/// Observe the active remote; main's documents do not change.
pub fn fetch(ws: &Workspace) -> Result<Value> {
    let cfg = active_observed(ws)?;
    let mut remote = open(ws, &cfg, false)?;
    let obs = observe::fetch(ws, &cfg, &mut remote)?;
    let cmp = state::compare(&ws.git, &state::snapshot(&ws.git, &cfg.alias)?)?;
    Ok(
        json!({"remote": cfg.alias, "observed_version_id": obs.commit.as_deref().map(version),
        "observed_at": obs.observed_at.map(rfc3339), "documents": obs.entries.len(),
        "unmanaged": obs.unmanaged, "state": cmp.state, "to_publish": cmp.to_publish,
        "to_incorporate": cmp.to_incorporate, "conflicts": cmp.conflicts,
        "message": "Observación remota completa. Los documentos de la principal no cambiaron."}),
    )
}
/// Observe and merge the remote into the current session with Git; conflicts stay there.
pub fn pull(ws: &Workspace) -> Result<Value> {
    ws.require_session()?;
    let cfg = active_observed(ws)?;
    sessions::prepare_merge(ws)?;
    let mut remote = open(ws, &cfg, false)?;
    observe::fetch(ws, &cfg, &mut remote)?;
    let snap = state::snapshot(&ws.git, &cfg.alias)?;
    let observed = snap
        .remote
        .clone()
        .ok_or_else(|| Error::RemoteIncomplete("Sin observación remota completa.".into()))?;
    let message = format!("Incorporar remoto {}\n\nKn-Reason: remote_pull", cfg.alias);
    let mut args = vec![
        "merge",
        "--no-overwrite-ignore",
        "--no-edit",
        "-m",
        message.as_str(),
    ];
    if snap.base.is_none() {
        // First reconciliation: Git compares both sides against nothing, so
        // differing documents conflict instead of one side winning.
        args.push("--allow-unrelated-histories");
    }
    args.push(&observed);
    sessions::merge_into_session(
        ws,
        &args,
        "Hay documentos en conflicto con el remoto dentro de la sesión. Resuélvelos, márcalos con Git y guarda una versión; main no cambió.",
    )?;
    Ok(
        json!({"remote": cfg.alias, "observed_version_id": version(&observed),
        "version_id": version(&ws.git.head()?),
        "message": "La sesión incluye la observación remota. Integra con kn worktree finish; la base avanza cuando la principal la contiene."}),
    )
}
pub fn push(ws: &Workspace, dry_run: bool, allow_deletes: bool) -> Result<Value> {
    publish::push(ws, dry_run, allow_deletes)
}

/// Implemented features for this folder's configuration, not permissions.
pub fn capabilities(common: &Path) -> Result<Value> {
    let mode = config::load_mode(common)?;
    let active = mode.remote.is_some();
    Ok(
        json!({"local_versioning": true, "sessions": true, "cloud_connect": true,
        "pull": active && mode.mode.observes(), "push": active && mode.mode.publishes(),
        "remote_refresh": active && mode.mode.observes(), "certified_providers": []}),
    )
}
fn state_message(mode: Mode, cmp: &Comparison) -> String {
    match cmp.state {
        SyncState::Synced => {
            "Sin diferencias documentales en la última observación; no confirma el estado actual.".into()
        }
        SyncState::LocalAhead if mode != Mode::Mcp => {
            "Hay cambios locales que el remoto todavía no refleja; en este modo los sube el cliente de escritorio, no kn.".into()
        }
        SyncState::LocalAhead => "Hay cambios locales por publicar con kn push.".into(),
        SyncState::RemoteAhead => {
            "El remoto tiene cambios; incorpóralos con kn pull en una sesión.".into()
        }
        SyncState::Diverged => {
            "Local y remoto cambiaron documentos distintos; Git puede integrarlos con kn pull en una sesión.".into()
        }
        SyncState::Conflicted => {
            "Local y remoto cambiaron los mismos documentos; kn pull los deja en conflicto dentro de una sesión para resolverlos.".into()
        }
        SyncState::Unknown => cmp.reason.clone().unwrap_or_default(),
    }
}
/// Remote section of `status`, from stored refs and observation only (no network).
pub fn summary(ws: &Workspace, refreshed: bool) -> Result<Value> {
    let common = &ws.git.common;
    let mode = config::load_mode(common)?;
    let Some(alias) = mode.remote.clone() else {
        return Ok(
            json!({"mode": mode.mode, "remote": null, "state": "not_configured",
            "publisher": publisher(mode.mode), "freshness": "unknown",
            "message": "Sin remoto activo; los documentos solo se versionan localmente."}),
        );
    };
    let cfg = config::load_remote(common, &alias)?;
    let obs = observe::load(common, &alias)?;
    let snap = state::snapshot(&ws.git, &alias)?;
    let cmp = state::compare(&ws.git, &snap)?;
    // Manual edits not yet observed are shown apart; they are never published implicitly.
    let uncommitted = if ws.config.session.is_none() {
        Some(ops::changes(&ws.git)?.len())
    } else {
        match ws.primary() {
            Ok(main) => Some(ops::changes(&main)?.len()),
            Err(_) => None,
        }
    };
    Ok(
        json!({"mode": mode.mode, "remote": alias, "endpoint": cfg.endpoint,
        "profile": cfg.profile.name, "publisher": publisher(mode.mode),
        "state": cmp.state, "reason": cmp.reason, "to_publish": cmp.to_publish,
        "to_incorporate": cmp.to_incorporate, "conflicts": cmp.conflicts,
        "base_version_id": snap.base.as_deref().map(version),
        "local_version_id": version(&snap.local),
        "observed_version_id": snap.remote.as_deref().map(version),
        "observed_at": obs.observed_at.map(rfc3339),
        "age_seconds": obs.observed_at.map(|t| now().saturating_sub(t)),
        "freshness": if obs.observed_at.is_none() { "unknown" } else if refreshed { "refreshed" } else { "stored" },
        "last_attempt": obs.last_attempt.as_ref().map(attempt),
        "unmanaged_remote": obs.unmanaged, "uncommitted_local_changes": uncommitted,
        "open_publications": publish::open_journals(common, &alias)?.len(),
        "message": state_message(mode.mode, &cmp)}),
    )
}
/// Committed differences between HEAD and the remote observation or the base.
pub fn diff(ws: &Workspace, against_remote: bool, patch: bool) -> Result<Value> {
    let (_, cfg) = config::active(&ws.git.common)?;
    let snap = state::snapshot(&ws.git, &cfg.alias)?;
    let (other, label) = if against_remote {
        let r = snap.remote.ok_or_else(|| {
            Error::RemoteIncomplete("Sin observación remota completa; ejecuta kn fetch.".into())
        })?;
        (r, "remote_observation")
    } else {
        let b = snap.base.ok_or_else(|| {
            Error::Conflict(
                "Sin base reconciliada; incorpora el remoto con kn pull en una sesión.".into(),
            )
        })?;
        (b, "sync_baseline")
    };
    let head = ws.git.head()?;
    let records = nul_paths(&ws.git.run(&[
        "diff",
        "--no-renames",
        "--name-status",
        "-z",
        &other,
        &head,
        "--",
    ])?)?;
    if records.len() % 2 != 0 {
        return Err(Error::Git("Diferencia Git inválida.".into()));
    }
    let changes: Vec<Value> = records
        .chunks(2)
        .map(|pair| {
            let kind = match pair[0].as_str() {
                "A" => "added",
                "D" => "deleted",
                _ => "modified",
            };
            json!({"path": pair[1], "kind": kind, "status": pair[0]})
        })
        .collect();
    let patch = if patch {
        Some(ws.git.text(&[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            &other,
            &head,
            "--",
        ])?)
    } else {
        None
    };
    Ok(
        json!({"base_version_id": version(&other), "base_label": label, "remote": cfg.alias,
        "changes": changes, "patch": patch,
        "message": if against_remote {
            "Cambios de la versión actual frente a la última observación remota: added existe aquí y no en el remoto. No incluye cambios sin guardar."
        } else {
            "Cambios de la versión actual frente a la base reconciliada. No incluye cambios sin guardar."
        }}),
    )
}

#[cfg(test)]
mod tests {
    use super::rfc3339;
    #[test]
    fn rfc3339_matches_known_instants() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(rfc3339(1_789_084_799), "2026-09-10T23:59:59Z");
    }
}
