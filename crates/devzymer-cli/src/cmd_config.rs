use anyhow::{bail, Result};
use devzymer_core::config::Config;
use devzymer_core::paths;

use crate::{ConfigAction, Ctx};

pub fn run(ctx: &Ctx, action: ConfigAction) -> Result<()> {
    let root = ctx.resolve_root();
    match action {
        ConfigAction::Path => {
            let path = paths::project_config_path(&root);
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "project": path.display().to_string(),
                        "user": paths::user_config_path().display().to_string()
                    }))?
                );
            } else {
                println!("projeto: {}", path.display());
                println!("usuário:  {}", paths::user_config_path().display());
            }
            Ok(())
        }
        ConfigAction::List => {
            let text = toml::to_string_pretty(&ctx.config)?;
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "config": toml::from_str::<toml::Value>(&text)?,
                        "source": "efetiva (usuário + projeto)"
                    }))?
                );
            } else {
                print!("{text}");
            }
            Ok(())
        }
        ConfigAction::Get { key } => {
            let value = lookup(&ctx.config, &key);
            match value {
                Some(v) => {
                    if ctx.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &serde_json::json!({ "key": key, "value": v })
                            )?
                        );
                    } else {
                        println!("{key} = {v}");
                    }
                    Ok(())
                }
                None => bail!("chave desconhecida: {key}"),
            }
        }
        ConfigAction::Set { key, value } => {
            let mut config = ctx.config.clone();
            apply(&mut config, &key, &value)?;
            let path = config.save_project(&root)?;
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "key": key,
                        "value": value,
                        "file": path.display().to_string()
                    }))?
                );
            } else {
                println!("{key} = {value} (salvo em {})", path.display());
            }
            Ok(())
        }
    }
}

pub(crate) fn lookup(config: &Config, key: &str) -> Option<String> {
    let value: String = match key {
        "index.max_file_size_bytes" => config.index.max_file_size_bytes.to_string(),
        "index.follow_symlinks" => config.index.follow_symlinks.to_string(),
        "index.respect_gitignore" => config.index.respect_gitignore.to_string(),
        "index.include_hidden" => config.index.include_hidden.to_string(),
        "index.max_workers" => config.index.max_workers.to_string(),
        "index.exclude" => config.index.exclude.join(","),
        "search.max_results" => config.search.max_results.to_string(),
        "search.default_token_budget" => config.search.default_token_budget.to_string(),
        "search.snippet_max_lines" => config.search.snippet_max_lines.to_string(),
        "search.max_fragment_bytes" => config.search.max_fragment_bytes.to_string(),
        "context.default_level" => config.context.default_level.to_string(),
        "context.max_total_bytes" => config.context.max_total_bytes.to_string(),
        "context.dedupe" => config.context.dedupe.to_string(),
        "context.cache_enabled" => config.context.cache_enabled.to_string(),
        "decisions.enabled" => config.decisions.enabled.to_string(),
        "decisions.provider" => config.decisions.provider.clone(),
        "decisions.model" => config.decisions.model.clone(),
        "decisions.base_url" => config.decisions.base_url.clone(),
        "decisions.api_key_env" => config.decisions.api_key_env.clone(),
        "decisions.timeout_secs" => config.decisions.timeout_secs.to_string(),
        "mcp.server_name" => config.mcp.server_name.clone(),
        "mcp.install_scope" => config.mcp.install_scope.clone(),
        "mcp.protocol_version" => config.mcp.protocol_version.clone(),
        "mcp.max_output_tokens" => config.mcp.max_output_tokens.to_string(),
        "privacy.allow_remote_source" => config.privacy.allow_remote_source.to_string(),
        "privacy.redact_secrets" => config.privacy.redact_secrets.to_string(),
        "logging.level" => config.logging.level.clone(),
        "tests.allow_files" => config.tests.allow_files.to_string(),
        "tests.approval_marker" => config.tests.approval_marker.clone(),
        "context.guard_search" => config.context.guard_search.to_string(),
        "routing.mode" => config.routing_mode().0.as_str().to_string(),
        "routing.telemetry" => config.routing.telemetry.to_string(),
        "routing.max_denials_per_session" => config.routing.max_denials_per_session.to_string(),
        "routing.max_advisories_per_session" => {
            config.routing.max_advisories_per_session.to_string()
        }
        "routing.health_cache_secs" => config.routing.health_cache_secs.to_string(),
        "routing.failure_window_secs" => config.routing.failure_window_secs.to_string(),
        "routing.recent_use_window_secs" => config.routing.recent_use_window_secs.to_string(),
        _ => return None,
    };
    Some(value)
}

pub(crate) fn apply(config: &mut Config, key: &str, value: &str) -> Result<()> {
    let parse_bool = |v: &str| -> Result<bool> {
        match v.to_ascii_lowercase().as_str() {
            "true" | "1" | "sim" | "yes" => Ok(true),
            "false" | "0" | "nao" | "não" | "no" => Ok(false),
            other => bail!("valor booleano inválido: {other}"),
        }
    };
    let parse_usize = |v: &str| -> Result<usize> {
        v.parse::<usize>()
            .map_err(|_| anyhow::anyhow!("valor numérico inválido: {v}"))
    };

    match key {
        "index.max_file_size_bytes" => {
            config.index.max_file_size_bytes = value.parse().unwrap_or(0)
        }
        "index.follow_symlinks" => config.index.follow_symlinks = parse_bool(value)?,
        "index.respect_gitignore" => config.index.respect_gitignore = parse_bool(value)?,
        "index.include_hidden" => config.index.include_hidden = parse_bool(value)?,
        "index.max_workers" => config.index.max_workers = parse_usize(value)?,
        "index.exclude" => {
            config.index.exclude = value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        "search.max_results" => config.search.max_results = parse_usize(value)?,
        "search.default_token_budget" => config.search.default_token_budget = parse_usize(value)?,
        "search.snippet_max_lines" => config.search.snippet_max_lines = parse_usize(value)?,
        "search.max_fragment_bytes" => config.search.max_fragment_bytes = parse_usize(value)?,
        "context.default_level" => config.context.default_level = parse_usize(value)? as u8,
        "context.max_total_bytes" => config.context.max_total_bytes = parse_usize(value)?,
        "context.dedupe" => config.context.dedupe = parse_bool(value)?,
        "context.cache_enabled" => config.context.cache_enabled = parse_bool(value)?,
        "decisions.enabled" => config.decisions.enabled = parse_bool(value)?,
        "decisions.provider" => config.decisions.provider = value.to_string(),
        "decisions.model" => config.decisions.model = value.to_string(),
        "decisions.base_url" => config.decisions.base_url = value.to_string(),
        "decisions.api_key_env" => config.decisions.api_key_env = value.to_string(),
        "decisions.timeout_secs" => config.decisions.timeout_secs = parse_usize(value)? as u64,
        "mcp.server_name" => config.mcp.server_name = value.to_string(),
        "mcp.install_scope" => config.mcp.install_scope = value.to_string(),
        "mcp.protocol_version" => config.mcp.protocol_version = value.to_string(),
        "mcp.max_output_tokens" => config.mcp.max_output_tokens = parse_usize(value)?,
        "privacy.allow_remote_source" => config.privacy.allow_remote_source = parse_bool(value)?,
        "privacy.redact_secrets" => config.privacy.redact_secrets = parse_bool(value)?,
        "logging.level" => config.logging.level = value.to_string(),
        "tests.allow_files" => config.tests.allow_files = parse_bool(value)?,
        "tests.approval_marker" => config.tests.approval_marker = value.to_string(),
        "context.guard_search" => config.context.guard_search = parse_bool(value)?,
        "routing.mode" => {
            config.routing.mode = Some(
                devzymer_core::config::RoutingMode::parse(value).ok_or_else(|| {
                    anyhow::anyhow!("modo inválido: {value}; use off, prefer ou guard")
                })?,
            )
        }
        "routing.telemetry" => config.routing.telemetry = parse_bool(value)?,
        "routing.max_denials_per_session" => {
            config.routing.max_denials_per_session = parse_usize(value)? as u32
        }
        "routing.max_advisories_per_session" => {
            config.routing.max_advisories_per_session = parse_usize(value)? as u32
        }
        "routing.health_cache_secs" => {
            config.routing.health_cache_secs = parse_usize(value)? as u64
        }
        "routing.failure_window_secs" => {
            config.routing.failure_window_secs = parse_usize(value)? as u64
        }
        "routing.recent_use_window_secs" => {
            config.routing.recent_use_window_secs = parse_usize(value)? as u64
        }
        other => bail!("chave desconhecida: {other}"),
    }
    Ok(())
}
