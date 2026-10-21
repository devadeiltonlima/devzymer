//! Dependency graph construction and traversal for devzymer.

pub mod build;
pub mod query;

pub use build::{build_graph, GraphReport};
pub use query::{GraphQuery, ImpactReport};
