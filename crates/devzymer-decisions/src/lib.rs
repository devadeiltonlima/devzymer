//! Structured decision routing for devzymer.
//!
//! Decisions follow a strict hierarchy: deterministic rules first, an optional
//! external decision provider second, and escalation to the coding agent last.

pub mod provider;
pub mod router;
pub mod schema;

pub use provider::{DecisionProvider, DeterministicProvider, JevProvider};
pub use router::{CircuitBreaker, DecisionRouter};
pub use schema::{DecisionOutcome, DecisionRequest, DecisionTask};
