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
        }
    }
    pub fn exit(&self) -> i32 {
        match self {
            Self::Conflict(_) => 2,
            Self::Invalid(_) | Self::Unsupported(_) | Self::SessionRequired => 3,
            _ => 1,
        }
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
                    "retryable": matches!(err, Error::Busy),
                    "suggested_next_action": match err {
                        Error::Busy => "Vuelve a intentar cuando termine la otra operación.",
                        Error::Copied => "Ejecuta kn init --fresh en la copia.",
                        Error::SessionRequired => "Ejecuta kn session start <nombre>.",
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
