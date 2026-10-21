//! Model Context Protocol server for devzymer.

pub mod patch;
pub mod server;
pub mod tools;

pub use patch::{apply_patch, PatchOutcome, PatchRequest};
pub use server::{serve_stdio, Server};
pub use tools::{tool_definitions, ToolOutput};
