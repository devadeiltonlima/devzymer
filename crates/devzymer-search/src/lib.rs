//! Deterministic retrieval and ranking for devzymer.

pub mod rank;
pub mod read;
pub mod resolve;
pub mod resolver;

pub use read::{read_excerpt, read_whole, Excerpt, ExcerptRequest};
pub use resolve::{
    query_tokens, FileMatch, Resolution, ResolutionSet, ResolutionSource, ResolveQuery,
};
pub use resolver::Resolver;
