use anyhow::{bail, Result};
use devzymer_core::config::{Config, RoutingMode};
use devzymer_core::util;
use devzymer_integrations::routing as host_routing;
use devzymer_routing::classify::{self, IntentKind};
use devzymer_routing::health;
use devzymer_routing::policy::Verdict;
use devzymer_routing::telemetry::{self, Summary};
use serde_json::{json, Value};

use crate::routing_hook;
use crate::{Ctx, RoutingAction};

const DAY_MS: u64 = 24 * 3600 * 1000;

pub fn run(ctx: &Ctx, action: RoutingAction) -> Result<()> {
    match action {
        RoutingAction::Status => status(ctx),
        RoutingAction::Mode { value } => mode(ctx, value),
        RoutingAction::Explain {
            tool,
            input,
            host,
            mode,
        } => explain(ctx, &tool, &input, host.as_deref(), mode.as_deref()),
        RoutingAction::Stats { days } => stats(ctx, days),
        RoutingAction::Reset => reset(ctx),
    }
}

fn status(ctx: &Ctx) -> Result<()> {
    let root = ctx.resolve_root();
    let (mode, source) = ctx.config.routing_mode();
    let now = util::now_millis();
    let hosts = host_routing::inspect(&root, devzymer_integrations::opencode::installed_major());
    let health = health::probe_uncached(&root, &ctx.config, now);
    let summary = telemetry::summarize(&telemetry::read_since(&root, now - 7 * DAY_MS), 7);

    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "mode": mode.as_str(),
                "source": source,
                "hosts": hosts,
                "health": health,
                "blocker": health.blocker(),
                "summary": summary,
            }))?
        );
        return Ok(());
    }

    println!("modo de roteamento: {} (origem: {source})", mode.as_str());
    if mode == RoutingMode::Guard {
        println!("  guard ativo: buscas redundantes são redirecionadas no máximo uma vez");
    }
    println!("hosts:");
    for host in &hosts {
        println!(
            "  {:<9} {:<13} mecanismo {} | orientação {} | {}",
            format!("{:?}", host.agent).to_lowercase(),
            host.enforcement,
            if host.mechanism_installed {
                host.mechanism
            } else {
                "ausente"
            },
            if host.guidance_installed {
                "instalada"
            } else {
                "ausente"
            },
            host.note
        );
    }
    println!("saúde do índice:");
    println!(
        "  presente {} | íntegro {} | workspace {} | esquema {} | FTS {}",
        yes_no(health.index_present),
        yes_no(health.index_ok),
        yes_no(health.workspace_match),
        yes_no(health.schema_current),
        yes_no(health.fts_ok)
    );
    println!(
        "  atualidade {:?} ({} arquivo(s) alterado(s)) | servidor MCP ativo {}",
        health.freshness,
        health.stale_files,
        yes_no(health.mcp_alive)
    );
    match health.blocker() {
        Some(reason) => println!("  o guard permitiria busca nativa agora: {reason}"),
        None => println!("  o guard poderia redirecionar buscas agora"),
    }
    print_summary(&summary);
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "sim"
    } else {
        "não"
    }
}

fn print_summary(summary: &Summary) {
    println!("últimos {} dias:", summary.window_days);
    if summary.events == 0 {
        println!("  sem eventos registrados");
        return;
    }
    for (host, counts) in &summary.by_host {
        println!(
            "  {host}: devzymer {} | buscas nativas {} | redirecionadas {} | fallback {} | orientadas {} | falhas do devzymer {}",
            counts.mcp_calls,
            counts.native_searches,
            counts.redirected,
            counts.fallback_allowed,
            counts.advised,
            counts.retrieval_failures
        );
    }
    match summary.devzymer_first_rate {
        Some(rate) => println!(
            "  devzymer primeiro em {:.0}% das {} sessão(ões) observadas pelo host ({} buscas nativas antes da primeira chamada)",
            rate * 100.0,
            summary.devzymer_first_sessions + summary.native_first_sessions,
            summary.native_before_first_devzymer
        ),
        None => println!("  taxa de devzymer-primeiro indisponível: sem sessões observadas pelo host"),
    }
    if let Some(p50) = summary.latency_p50_ms {
        println!(
            "  latência observada pelo servidor: p50 {p50} ms, p95 {} ms",
            summary.latency_p95_ms.unwrap_or(p50)
        );
    }
    if !summary.fallback_reasons.is_empty() {
        let reasons: Vec<String> = summary
            .fallback_reasons
            .iter()
            .map(|(reason, count)| format!("{reason}={count}"))
            .collect();
        println!("  motivos de fallback: {}", reasons.join(", "));
    }
    println!(
        "  origem dos dados: host {} | servidor {} | inferido {} (economia de tokens não é medida)",
        summary.host_observed, summary.server_observed, summary.inferred
    );
}

fn mode(ctx: &Ctx, value: Option<String>) -> Result<()> {
    let root = ctx.resolve_root();
    let Some(value) = value else {
        let (mode, source) = ctx.config.routing_mode();
        if ctx.json {
            println!("{}", json!({ "mode": mode.as_str(), "source": source }));
        } else {
            println!("{} (origem: {source})", mode.as_str());
        }
        return Ok(());
    };
    let Some(parsed) = RoutingMode::parse(&value) else {
        bail!("modo inválido: {value}; use off, prefer ou guard");
    };
    let mut config: Config = ctx.config.clone();
    config.routing.mode = Some(parsed);
    let path = config.save_project(&root)?;
    if ctx.json {
        println!(
            "{}",
            json!({ "mode": parsed.as_str(), "file": path.display().to_string() })
        );
    } else {
        println!(
            "modo de roteamento: {} (salvo em {})",
            parsed.as_str(),
            path.display()
        );
        if parsed == RoutingMode::Guard {
            println!("guard redireciona uma vez cada busca que o índice responde; a repetição é sempre permitida");
        }
    }
    Ok(())
}

fn explain(
    ctx: &Ctx,
    tool: &str,
    input: &str,
    host: Option<&str>,
    forced_mode: Option<&str>,
) -> Result<()> {
    let root = ctx.resolve_root();
    let tool_input: Value = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("--input deve ser JSON: {error}"))?;
    let forced =
        match forced_mode {
            Some(text) => Some(RoutingMode::parse(text).ok_or_else(|| {
                anyhow::anyhow!("modo inválido: {text}; use off, prefer ou guard")
            })?),
            None => None,
        };
    let payload = json!({
        "tool_name": tool,
        "tool_input": tool_input,
        "cwd": root.display().to_string(),
        "session_id": "routing-explain"
    });
    let Some(outcome) = routing_hook::evaluate(&payload, host, true, forced) else {
        let search = classify::classify(tool, &payload["tool_input"], &root).is_some();
        let message = if search {
            "sem decisão: modo off ou projeto sem .devzymer"
        } else {
            "não é uma busca de código no repositório: permitido sem interferência"
        };
        if ctx.json {
            println!(
                "{}",
                json!({ "classified": search, "verdict": "allow", "reason": message })
            );
        } else {
            println!("{message}");
        }
        return Ok(());
    };

    let (verdict, message) = match &outcome.decision.verdict {
        Verdict::Allow => ("allow", String::new()),
        Verdict::Advise(text) => ("advise", text.clone()),
        Verdict::Deny(text) => ("deny", text.clone()),
    };
    let kind = match outcome.intent.kind {
        IntentKind::CodeSearch => "code_search",
        IntentKind::FileDiscovery => "file_discovery",
        IntentKind::FileRead => "file_read",
    };
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "host": outcome.host.as_str(),
                "mode": outcome.mode.as_str(),
                "intent": kind,
                "category": outcome.intent.category,
                "terms": outcome.intent.terms,
                "scope": outcome.intent.scope,
                "verdict": verdict,
                "reason": outcome.decision.reason,
                "message": message,
            }))?
        );
    } else {
        println!(
            "host {} | modo {} | {kind} ({})",
            outcome.host.as_str(),
            outcome.mode.as_str(),
            outcome.intent.category
        );
        println!("decisão: {verdict} ({})", outcome.decision.reason);
        if !message.is_empty() {
            println!("mensagem: {message}");
        }
    }
    Ok(())
}

fn stats(ctx: &Ctx, days: u64) -> Result<()> {
    let root = ctx.resolve_root();
    let now = util::now_millis();
    let events = telemetry::read_since(&root, now.saturating_sub(days.max(1) * DAY_MS));
    let summary = telemetry::summarize(&events, days.max(1));
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print_summary(&summary);
    }
    Ok(())
}

fn reset(ctx: &Ctx) -> Result<()> {
    let root = ctx.resolve_root();
    let dir = devzymer_routing::routing_dir(&root);
    let mut removed = 0usize;
    for name in ["events.jsonl", "events.1.jsonl", "health.json"] {
        if std::fs::remove_file(dir.join(name)).is_ok() {
            removed += 1;
        }
    }
    let sessions = dir.join("sessions");
    if sessions.exists() && std::fs::remove_dir_all(&sessions).is_ok() {
        removed += 1;
    }
    if ctx.json {
        println!("{}", json!({ "removed": removed }));
    } else {
        println!("estado de roteamento limpo ({removed} item(ns)); o índice não foi tocado");
    }
    Ok(())
}
