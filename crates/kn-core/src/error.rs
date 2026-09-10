use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("No se encontró una carpeta kn; ejecuta kn init.")]
    NotWorkspace,
    #[error("La carpeta está ocupada por otra operación; vuelve a intentar.")]
    Busy,
    #[error("Esta identidad pertenece a otra carpeta existente; usa kn init --fresh en la copia.")]
    Copied,
    #[error("Trabaja en una sesión: kn session start <nombre>.")]
    SessionRequired,
    #[error(
        "La principal pertenece a otro espacio. Conserva esta sesión y abre una nueva desde la principal actual."
    )]
    IdentityChanged,
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Unsafe(String),
    #[error("Falló Git: {0}")]
    Git(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("No hay un remoto activo. Configura uno con kn remote add y kn mode set.")]
    NoRemote,
    #[error("{0}")]
    ModeForbids(String),
    #[error("{0}")]
    AuthRequired(String),
    #[error("{0}")]
    RemoteUnavailable(String),
    #[error("{0}")]
    Remote(String),
    #[error("{0}")]
    RemoteIncomplete(String),
    #[error("{0}")]
    ProfileMismatch(String),
}
impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "INVALID_INPUT",
            Self::Unsupported(_) => "UNSUPPORTED_CAPABILITY",
            Self::NotWorkspace => "NOT_A_WORKSPACE",
            Self::Busy => "WORKSPACE_BUSY",
            Self::Copied => "WORKSPACE_COPIED",
            Self::SessionRequired => "SESSION_REQUIRED",
            Self::IdentityChanged => "WORKSPACE_IDENTITY_CHANGED",
            Self::Conflict(_) => "CONFLICT",
            Self::Unsafe(_) => "UNSAFE_PATH",
            Self::Git(_) => "GIT_FAILED",
            Self::Io(_) => "IO_ERROR",
            Self::Json(_) => "INVALID_STATE",
            Self::NoRemote => "REMOTE_NOT_CONFIGURED",
            Self::ModeForbids(_) => "MODE_FORBIDS_OPERATION",
            Self::AuthRequired(_) => "AUTH_REQUIRED",
            Self::RemoteUnavailable(_) => "REMOTE_UNAVAILABLE",
            Self::Remote(_) => "REMOTE_ERROR",
            Self::RemoteIncomplete(_) => "REMOTE_INCOMPLETE",
            Self::ProfileMismatch(_) => "PROFILE_MISMATCH",
        }
    }
    pub fn exit(&self) -> i32 {
        match self {
            Self::Conflict(_) => 2,
            Self::Invalid(_)
            | Self::Unsupported(_)
            | Self::SessionRequired
            | Self::NoRemote
            | Self::ModeForbids(_)
            | Self::ProfileMismatch(_) => 3,
            _ => 1,
        }
    }
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Busy | Self::RemoteUnavailable(_) | Self::RemoteIncomplete(_)
        )
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Serialize)]
pub struct Envelope {
    pub schema_version: &'static str,
    pub operation_id: uuid::Uuid,
    pub status: &'static str,
    pub data: serde_json::Value,
    pub conflicts: Vec<serde_json::Value>,
    pub errors: Vec<serde_json::Value>,
}
impl Envelope {
    pub fn new(result: &Result<serde_json::Value>) -> Self {
        let mut e = Self {
            schema_version: "1.0",
            operation_id: uuid::Uuid::new_v4(),
            status: "ok",
            data: serde_json::Value::Null,
            conflicts: vec![],
            errors: vec![],
        };
        match result {
            Ok(data) => e.data = data.clone(),
            Err(err) => {
                e.status = match err.exit() {
                    2 => "conflict",
                    3 => "unsupported",
                    _ => "error",
                };
                e.errors.push(
                    serde_json::json!({"code": err.code(), "message": err.to_string(),
                    "retryable": err.retryable(),
                    "suggested_next_action": match err {
                        Error::Busy => "Vuelve a intentar cuando termine la otra operación.",
                        Error::Copied => "Ejecuta kn init --fresh en la copia.",
                        Error::SessionRequired => "Ejecuta kn session start <nombre>.",
                        Error::NoRemote => "Ejecuta kn remote add y kn mode set <modo> --remote <alias>.",
                        Error::ModeForbids(_) => "Revisa kn mode; cambiar de modo no transfiere documentos.",
                        Error::AuthRequired(_) => "Ejecuta kn remote login <alias> o entrega KN_MCP_ACCESS_TOKEN en el entorno.",
                        Error::RemoteUnavailable(_) => "Vuelve a intentar; nada se marcó como publicado sin verificarlo.",
                        Error::RemoteIncomplete(_) => "Vuelve a observar con kn fetch; no se infirieron borrados.",
                        Error::ProfileMismatch(_) => "Actualiza y prueba el perfil del servidor antes de usar esa operación.",
                        Error::Conflict(_) => "Revisa kn status; los conflictos se resuelven dentro de una sesión.",
                        _ => "Consulta kn --help y kn status antes de volver a intentar.",
                    }}),
                );
                if matches!(err, Error::Conflict(_)) {
                    e.conflicts
                        .push(serde_json::json!({"message": err.to_string()}));
                }
            }
        }
        e
    }
}
