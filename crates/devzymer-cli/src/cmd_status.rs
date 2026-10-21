use anyhow::Result;
use devzymer_core::paths;
use devzymer_core::store::Store;
use devzymer_core::workspace::Workspace;
use devzymer_indexer::index::{IndexOptions, Indexer};
use devzymer_indexer::scan::ScanOptions;

use crate::Ctx;

pub fn run(ctx: &Ctx) -> Result<()> {
    let root = ctx.resolve_root();
    let db = paths::db_path(&root);

    if !db.exists() {
        let payload = serde_json::json!({
            "initialized": false,
            "root": root.display().to_string()
        });
        if ctx.json {
            println!("{}", serde_json::to_string_pretty(&payload)?);
        } else {
            println!("índice não inicializado em {}", root.display());
            println!("execute `devzymer init` para criar o índice");
        }
        return Ok(());
    }

    let store = Store::open(&db)?;
    let stats = store.stats()?;
    let workspace = Workspace::detect(&root);
    let indexer = Indexer::new(&store, &root);
    let changed = indexer
        .changed_files(&IndexOptions {
            root: root.clone(),
            scan: scan_options(ctx),
            force_full: false,
            chunk_size: 256,
        })
        .unwrap_or_default();

    let scope = ctx.config.mcp.install_scope.clone();
    let mcp_installed = match scope.as_str() {
        "project" => workspace
            .claude
            .project_mcp
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false),
        _ => workspace.claude.user_config.is_some(),
    };

    let permission = mcp_permission(&root, &scope, &ctx.config.mcp.server_name);

    let payload = serde_json::json!({
        "initialized": true,
        "root": root.display().to_string(),
        "files": stats.files,
        "symbols": stats.symbols,
        "relations": stats.relations,
        "indexed_bytes": stats.indexed_bytes,
        "revision": stats.revision,
        "schema_version": stats.schema_version,
        "parser_version": stats.parser_version,
        "last_index_ms": stats.last_index_ms,
        "languages": stats.languages,
        "pending_changes": changed.len(),
        "mcp_installed": mcp_installed,
        "mcp_permission": permission,
        "claude_cli": workspace.claude.cli_installed,
        "codegraph_present": workspace.codegraph.present,
        "decisions_enabled": ctx.config.decisions.enabled
    });

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("raiz: {}", root.display());
    println!(
        "índice: {} arquivos, {} símbolos, {} relações, revisão {}",
        stats.files, stats.symbols, stats.relations, stats.revision
    );
    println!(
        "idiomas: {}",
        stats
            .languages
            .iter()
            .map(|(l, c)| format!("{l}={c}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "atualidade: {} arquivo(s) alterado(s) desde o último índice",
        changed.len()
    );
    println!(
        "integração MCP: {} | Claude Code CLI: {}",
        if mcp_installed {
            "instalada"
        } else {
            "ausente"
        },
        if workspace.claude.cli_installed {
            "presente"
        } else {
            "ausente"
        }
    );
    println!(
        "permissão das ferramentas MCP: {}",
        if permission {
            "concedida"
        } else {
            "ausente: as chamadas são rejeitadas até `devzymer init` conceder a regra"
        }
    );
    println!(
        "CodeGraph: {} | decisões externas: {}",
        if workspace.codegraph.present {
            "detectado"
        } else {
            "ausente"
        },
        if ctx.config.decisions.enabled {
            "habilitadas"
        } else {
            "desabilitadas"
        }
    );
    Ok(())
}

/// True when the agent is allowed to call the tools of the server.
///
/// Without the rule the host rejects every call, and the agent never reaches
/// the index.
fn mcp_permission(root: &std::path::Path, scope: &str, server_name: &str) -> bool {
    let settings = match scope {
        "project" => root.join(".claude").join("settings.json"),
        _ => paths::user_home().join(".claude").join("settings.json"),
    };
    let Ok(raw) = std::fs::read_to_string(&settings) else {
        return false;
    };
    let Ok(document) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let rule = format!("mcp__{server_name}__*");
    document
        .get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(|a| a.as_array())
        .map(|list| {
            list.iter()
                .any(|value| value.as_str() == Some(rule.as_str()))
        })
        .unwrap_or(false)
}

/// Reports the freshness of the index, used by the doctor command.
pub fn pending_changes(root: &std::path::Path) -> usize {
    let db = paths::db_path(root);
    if !db.exists() {
        return 0;
    }
    let Ok(store) = Store::open(&db) else {
        return 0;
    };
    let indexer = Indexer::new(&store, root);
    indexer
        .changed_files(&IndexOptions {
            root: root.to_path_buf(),
            scan: ScanOptions {
                exclude: devzymer_core::config::Config::load(root)
                    .unwrap_or_default()
                    .index
                    .exclude,
                max_duration_ms: None,
                ..ScanOptions::default()
            },
            force_full: false,
            chunk_size: 256,
        })
        .map(|f| f.len())
        .unwrap_or(0)
}

/// Scan options derived from the effective configuration.
pub fn scan_options(ctx: &Ctx) -> ScanOptions {
    ScanOptions {
        exclude: ctx.config.index.exclude.clone(),
        exclude_secret_files: ctx.config.privacy.exclude_secret_files,
        max_file_size: ctx.config.index.max_file_size_bytes,
        follow_symlinks: ctx.config.index.follow_symlinks,
        respect_gitignore: ctx.config.index.respect_gitignore,
        include_hidden: ctx.config.index.include_hidden,
        languages: Vec::new(),
        max_workers: ctx.config.effective_workers(),
        max_duration_ms: None,
    }
}
