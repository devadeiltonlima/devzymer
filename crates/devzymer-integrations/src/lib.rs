//! Integração do devzymer com agentes de código.

pub mod agents;
pub mod claude;
pub mod codegraph;
pub mod codex;
pub mod jsonc;
pub mod migrate;
pub mod opencode;

pub use agents::{
    detect, detect_one, install, probe_server, uninstall_agent, AgentDetection, AgentKind,
    AgentOutcome, AgentStatus,
};
pub use claude::{install as install_claude, install_server, uninstall as uninstall_claude};
pub use claude::{InstallReport, InstallScope};
pub use codegraph::{detect_codegraph, CodeGraphReport};
pub use migrate::{migrate, MigrationPlan, MigrationReport};
