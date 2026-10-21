use anyhow::Result;
use devzymer_core::paths;

use crate::Ctx;

pub fn run(ctx: &Ctx, confirm: bool, dry_run: bool, scope: &str) -> Result<()> {
    let root = ctx.resolve_root();
    let scope = devzymer_integrations::InstallScope::parse(scope)
        .unwrap_or(devzymer_integrations::InstallScope::Project);

    let report = devzymer_integrations::migrate(&root, &ctx.config, scope, confirm, dry_run);

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("plano de migração");
    println!("  codegraph detectado: {}", report.plan.codegraph_detected);
    if let Some(dir) = &report.plan.index_dir {
        println!("  índice do codegraph: {dir}");
    }
    for action in &report.plan.actions {
        println!("  ação: {action}");
    }
    for warning in &report.plan.warnings {
        println!("  aviso: {warning}");
    }

    if !report.applied {
        println!("\nnenhuma alteração aplicada. Use `devzymer migrate --confirm` para executar.");
        return Ok(());
    }

    println!("\naplicado:");
    for change in &report.changes {
        println!("  {} -> {}", change.file, change.detail);
    }
    for backup in &report.backups {
        println!("  backup: {backup}");
    }
    for disabled in &report.codegraph_disabled {
        println!("  codegraph desativado: {disabled}");
    }
    for error in &report.errors {
        println!("  erro: {error}");
    }
    Ok(())
}

pub fn uninstall(ctx: &Ctx, scope: &str, purge: bool) -> Result<()> {
    let root = ctx.resolve_root();
    let scope = devzymer_integrations::InstallScope::parse(scope)
        .unwrap_or(devzymer_integrations::InstallScope::Project);

    let mut report =
        devzymer_integrations::uninstall_claude(&root, &ctx.config.mcp.server_name, scope)?;
    for agent in [
        devzymer_integrations::AgentKind::Codex,
        devzymer_integrations::AgentKind::OpenCode,
    ] {
        let other = devzymer_integrations::uninstall_agent(&root, agent, scope)?;
        report.changes.extend(other.changes);
        report.backups.extend(other.backups);
    }

    // The project files belong to the project scope only. A user scope
    // uninstall never reaches into the current directory.
    if scope != devzymer_integrations::InstallScope::User {
        report
            .changes
            .extend(devzymer_integrations::setup::uninstall(&root));
    }

    if purge {
        let dir = paths::project_dir(&root);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| devzymer_core::Error::io(&dir, e))?;
        }
    }

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    for change in &report.changes {
        println!("{} -> {}", change.file, change.detail);
    }
    for backup in &report.backups {
        println!("backup: {backup}");
    }
    if report.changes.is_empty() {
        println!("nenhuma integração do devzymer encontrada");
    }
    if purge {
        println!("índice local removido");
    }
    Ok(())
}
