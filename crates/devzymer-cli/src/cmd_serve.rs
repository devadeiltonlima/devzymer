use anyhow::Result;
use std::path::Path;

use crate::Ctx;

pub async fn run(root: Option<&Path>) -> Result<()> {
    let root = devzymer_mcp::server::resolve_root(root)?;
    let config = devzymer_core::config::Config::load(&root).unwrap_or_default();
    let server = devzymer_mcp::Server::new(root, config)?;
    devzymer_mcp::serve_stdio(server).await?;
    Ok(())
}

/// Keeps the context type visible for the other command modules.
pub fn _unused(ctx: &Ctx) -> &Ctx {
    ctx
}
