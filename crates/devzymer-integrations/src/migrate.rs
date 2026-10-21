use std::path::Path;

use serde::Serialize;

use crate::claude::{self, InstallScope};
use crate::codegraph::{self, CodeGraphReport};

#[derive(Debug, Clone, Default, Serialize)]
pub struct MigrationPlan {
    pub codegraph_detected: bool,
    pub index_dir: Option<String>,
    pub mcp_entries: Vec<String>,
    pub mcp_server_names: Vec<String>,
    pub executable: Option<String>,
    pub actions: Vec<String>,
    pub warnings: Vec<String>,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MigrationReport {
    pub applied: bool,
    pub dry_run: bool,
    pub plan: MigrationPlan,
    pub changes: Vec<claude::Change>,
    pub backups: Vec<String>,
    pub codegraph_disabled: Vec<String>,
    pub errors: Vec<String>,
}

/// Builds the migration plan without touching any file.
pub fn plan_migration(root: &Path, _server_name: &str) -> MigrationPlan {
    let report: CodeGraphReport = codegraph::detect_codegraph(root);
    let mut actions = Vec::new();
    let mut warnings = Vec::new();

    if report.present {
        actions.push("registrar o servidor MCP do devzymer no projeto".into());
        actions.push("instalar as regras de uso do devzymer".into());
        actions.push("instalar os hooks de compressão e de guarda de arquivos de teste".into());
        if !report.mcp_server_names.is_empty() {
            actions.push(format!(
                "desativar a entrada MCP do CodeGraph: {}",
                report.mcp_server_names.join(", ")
            ));
        }
        warnings.push(
            "a desativação do CodeGraph é local: o pacote continua instalado e pode ser reativado"
                .into(),
        );
    } else {
        actions.push("registrar o servidor MCP do devzymer no projeto".into());
        actions.push("instalar as regras de uso do devzymer".into());
    }

    MigrationPlan {
        codegraph_detected: report.present,
        index_dir: report.index_dir,
        mcp_entries: report.mcp_entries,
        mcp_server_names: report.mcp_server_names,
        executable: report.executable,
        actions,
        warnings,
        requires_approval: report.present,
    }
}

/// Executes the migration. Without `confirm`, the plan is only reported.
pub fn migrate(
    root: &Path,
    config: &devzymer_core::config::Config,
    scope: InstallScope,
    confirm: bool,
    dry_run: bool,
) -> MigrationReport {
    let plan = plan_migration(root, &config.mcp.server_name);
    let mut report = MigrationReport {
        dry_run,
        plan: plan.clone(),
        ..Default::default()
    };

    if dry_run || !confirm {
        return report;
    }

    match claude::install(root, config, scope, None) {
        Ok(install) => {
            report.applied = true;
            report.changes = install.changes;
            report.backups = install.backups;
        }
        Err(err) => report.errors.push(err.to_string()),
    }

    if plan.codegraph_detected {
        for entry in &plan.mcp_entries {
            for name in &plan.mcp_server_names {
                match codegraph::disable_mcp_entry(Path::new(entry), name) {
                    Ok(true) => report.codegraph_disabled.push(format!("{entry} :: {name}")),
                    Ok(false) => {}
                    Err(err) => report.errors.push(err),
                }
            }
        }
    }

    report
}
