use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("io error: {0}")]
    PlainIo(#[from] std::io::Error),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml deserialization error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),
    #[error("ambiguous match for '{query}': {count} candidates")]
    AmbiguousMatch { query: String, count: usize },
    #[error("stale index: {0}")]
    StaleIndex(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("PATCH_STALE_REVISION: {0}")]
    PatchStaleRevision(String),
    #[error("PATCH_AMBIGUOUS_TARGET: {0}")]
    PatchAmbiguousTarget(String),
    #[error("PATCH_OUTSIDE_WORKSPACE: {0}")]
    PatchOutsideWorkspace(String),
    #[error("PATCH_CONTENT_MISMATCH: {0}")]
    PatchContentMismatch(String),
    #[error("decision provider unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),
    #[error("not initialized: {0}")]
    NotInitialized(String),
    #[error("path escapes workspace: {0}")]
    PathEscape(String),
    #[error("workspace not resolved: {0}")]
    WorkspaceNotResolved(String),
    #[error("workspace too broad: {0}")]
    WorkspaceTooBroad(String),
    #[error("workspace mismatch: {0}")]
    WorkspaceMismatch(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }

    /// Stable machine-readable code used by the MCP surface.
    pub fn code(&self) -> &'static str {
        match self {
            Error::SymbolNotFound(_) => "SYMBOL_NOT_FOUND",
            Error::AmbiguousMatch { .. } => "AMBIGUOUS_MATCH",
            Error::StaleIndex(_) => "STALE_INDEX",
            Error::PermissionDenied(_) => "PERMISSION_DENIED",
            Error::UnsupportedLanguage(_) => "UNSUPPORTED_LANGUAGE",
            Error::InvalidPatch(_) => "INVALID_PATCH",
            Error::PatchStaleRevision(_) => "PATCH_STALE_REVISION",
            Error::PatchAmbiguousTarget(_) => "PATCH_AMBIGUOUS_TARGET",
            Error::PatchOutsideWorkspace(_) => "PATCH_OUTSIDE_WORKSPACE",
            Error::PatchContentMismatch(_) => "PATCH_CONTENT_MISMATCH",
            Error::ProviderUnavailable(_) => "PROVIDER_UNAVAILABLE",
            Error::ResourceLimitExceeded(_) => "RESOURCE_LIMIT_EXCEEDED",
            Error::NotInitialized(_) => "NOT_INITIALIZED",
            Error::PathEscape(_) => "PATH_ESCAPE",
            Error::WorkspaceNotResolved(_) => "WORKSPACE_NOT_RESOLVED",
            Error::WorkspaceTooBroad(_) => "WORKSPACE_TOO_BROAD",
            Error::WorkspaceMismatch(_) => "WORKSPACE_MISMATCH",
            Error::Config(_) => "CONFIG_ERROR",
            Error::Database(_) | Error::Json(_) | Error::TomlDe(_) | Error::TomlSer(_) => {
                "INTERNAL_ERROR"
            }
            _ => "INTERNAL_ERROR",
        }
    }
}
