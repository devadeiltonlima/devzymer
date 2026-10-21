use anyhow::Result;
use devzymer_integrations::{AgentKind, InstallScope};

use crate::{Ctx, IntegrationAction};

/// Gerencia as integrações do devzymer com os agentes de código.
pub fn run(ctx: &Ctx, action: IntegrationAction) -> Result<()> {
    let root = ctx.resolve_root();
    match action {
        IntegrationAction::List => list(ctx, &root),
        IntegrationAction::Add { agent } => add(ctx, &root, &agent),
        IntegrationAction::Remove { agent } => remove(ctx, &root, &agent),
        IntegrationAction::Repair => repair(ctx, &root),
        IntegrationAction::Verify => verify(ctx, &root),
        IntegrationAction::Scope { agent, scope } => change_scope(ctx, &root, &agent, &scope),
    }
}

fn list(ctx: &Ctx, root: &std::path::Path) -> Result<()> {
    let detections = devzymer_integrations::detect(root);
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "root": root.display().to_string(),
                "agents": detections
            }))?
        );
        return Ok(());
    }
    println!("integrações do devzymer em {}", root.display());
    for detection in &detections {
        let where_ = detection
            .registered_at
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "não registrado".to_string());
        println!(
            "  {:<12} {:<14} {}",
            detection.agent.key(),
            detection.status.display(),
            where_
        );
        for note in &detection.notes {
            println!("               {note}");
        }
    }
    Ok(())
}

fn add(ctx: &Ctx, root: &std::path::Path, agent: &str) -> Result<()> {
    let kind = parse_agent(agent)?;
    let scope = InstallScope::parse(&ctx.config.mcp.install_scope).unwrap_or(InstallScope::Project);
    let executable = std::env::current_exe()?;
    let outcomes =
        devzymer_integrations::install(root, &ctx.config, &[kind], scope, &executable, false)?;
    for outcome in outcomes {
        println!(
            "[{}] {}: {}",
            if outcome.ok { "✓" } else { "✗" },
            outcome.agent.display(),
            outcome.detail
        );
        for change in &outcome.changes {
            println!("      {} -> {}", change.file, change.detail);
        }
    }
    Ok(())
}

fn remove(ctx: &Ctx, root: &std::path::Path, agent: &str) -> Result<()> {
    let kind = parse_agent(agent)?;
    let scope = InstallScope::parse(&ctx.config.mcp.install_scope).unwrap_or(InstallScope::Project);
    let report = devzymer_integrations::uninstall_agent(root, kind, scope)?;
    if report.changes.is_empty() {
        println!("nada a remover em {} ({})", kind.display(), scope.display());
    }
    for change in &report.changes {
        println!("  {} -> {}", change.file, change.detail);
    }
    Ok(())
}

fn repair(ctx: &Ctx, root: &std::path::Path) -> Result<()> {
    let detections = devzymer_integrations::detect(root);
    let configured: Vec<AgentKind> = detections
        .iter()
        .filter(|d| d.registered_at.is_some())
        .map(|d| d.agent)
        .collect();
    if configured.is_empty() {
        println!("nenhuma integração instalada para reparar.");
        return Ok(());
    }
    let scope = InstallScope::parse(&ctx.config.mcp.install_scope).unwrap_or(InstallScope::Project);
    let executable = std::env::current_exe()?;
    let outcomes =
        devzymer_integrations::install(root, &ctx.config, &configured, scope, &executable, false)?;
    for outcome in outcomes {
        println!(
            "[{}] {}: {}",
            if outcome.verified { "✓" } else { "✗" },
            outcome.agent.display(),
            outcome.detail
        );
    }
    Ok(())
}

fn verify(ctx: &Ctx, root: &std::path::Path) -> Result<()> {
    let executable = std::env::current_exe()?;
    let connected = devzymer_integrations::probe_server(&executable, root);
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "root": root.display().to_string(),
                "mcp_connected": connected
            }))?
        );
        return Ok(());
    }
    if connected {
        println!("[✓] servidor MCP respondeu ao handshake");
    } else {
        println!("[✗] servidor MCP não respondeu ao handshake");
    }
    Ok(())
}

fn change_scope(ctx: &Ctx, root: &std::path::Path, agent: &str, scope: &str) -> Result<()> {
    let kind = parse_agent(agent)?;
    let target = InstallScope::parse(scope)
        .ok_or_else(|| anyhow::anyhow!("escopo desconhecido '{scope}'. Use project ou user."))?;
    let executable = std::env::current_exe()?;
    let outcomes =
        devzymer_integrations::install(root, &ctx.config, &[kind], target, &executable, false)?;
    for outcome in outcomes {
        println!(
            "[{}] {}: {}",
            if outcome.ok { "✓" } else { "✗" },
            outcome.agent.display(),
            outcome.detail
        );
    }
    for other in [
        InstallScope::Project,
        InstallScope::Local,
        InstallScope::User,
    ] {
        if other == target {
            continue;
        }
        let report = devzymer_integrations::uninstall_agent(root, kind, other)?;
        for change in &report.changes {
            println!("  {} -> {}", change.file, change.detail);
        }
    }
    Ok(())
}

fn parse_agent(agent: &str) -> Result<AgentKind> {
    AgentKind::parse(agent).ok_or_else(|| {
        anyhow::anyhow!("agente desconhecido '{agent}'. Use claude, codex ou opencode.")
    })
}
