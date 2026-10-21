//! devzymer core: domain model, configuration, persistence and shared utilities.

pub mod config;
pub mod error;
pub mod logging;
pub mod model;
pub mod paths;
pub mod privacy;
pub mod store;
pub mod util;
pub mod workspace;
pub mod workspace_root;

pub use error::{Error, Result};
pub use model::{
    Confidence, Entity, Language, Relation, RelationKind, Span, Symbol, SymbolId, SymbolKind,
};
