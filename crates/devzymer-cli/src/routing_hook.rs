use std::path::PathBuf;

use anyhow::Result;
use devzymer_core::config::{Config, RoutingMode};
use devzymer_core::store::Store;
use devzymer_core::{paths, util, workspace_root};
use devzymer_routing::classify::{self, Intent};
use devzymer_routing::health;
use devzymer_routing::host::Host;
use devzymer_routing::policy::{self, Decision, Env, IndexLookup, Request, Verdict};
use devzymer_routing::prompt;
use devzymer_routing::session;
use devzymer_routing::signals::{self, McpOutcome};
use devzymer_routing::telemetry::{self, kind, observed, Event};
use serde_json::{json, Value};

const TRIVIAL_ALLOWS: [&str; 4] = [
    "mode_off",
    "not_answerable",
    "outside_coverage",
    "read_allowed",
];

pub struct Outcome {
    pub host: Host,
    pub mode: RoutingMode,
    pub intent: Intent,
    pub decision: Decision,
}

pub fn resolve_host(flag: Option<&str>, input: &Value) -> Host {
    flag.and_then(Host::parse)
        .unwrap_or_else(|| Host::from_hook_input(input))
}

fn workspace(input: &Value) -> Option<PathBuf> {
    let cwd = cwd_of(input);
    let start = workspace_root::canonical(&cwd).unwrap_or(cwd);
    let mut current = Some(start.as_path());
    while let Some(dir) = current {
        if workspace_root::is_broad(dir) {
            return None;
        }
        if paths::project_dir(dir).is_dir() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn cwd_of(input: &Value) -> PathBuf {
    input
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(paths::current_dir)
}

fn host_source(host: Host) -> &'static str {
    if host == Host::OpenCode {
        "plugin"
    } else {
        "hook"
    }
}

pub fn evaluate(
    input: &Value,
    host_flag: Option<&str>,
    dry_run: bool,
    forced_mode: Option<RoutingMode>,
) -> Option<Outcome> {
    let host = resolve_host(host_flag, input);
    let tool = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tool_input = input
        .get("tool_input")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let intent = classify::classify(tool, &tool_input, &cwd_of(input))?;

    let root = workspace(input)?;
    let config = Config::load(&root).unwrap_or_default();
    let mode = forced_mode.unwrap_or_else(|| config.routing_mode().0);
    if mode == RoutingMode::Off {
        return None;
    }

    let now = util::now_millis();
    let key = session::key(input.get("session_id").and_then(Value::as_str));
    let mut state = session::load(&root, &key);
    let db = paths::db_path(&root);
    let store = if db.exists() {
        Store::open_read_only(&db).ok()
    } else {
        None
    };
    let lookup: Option<&dyn IndexLookup> = store.as_ref().map(|store| store as &dyn IndexLookup);
    let probe = || health::probe(&root, &config, now);

    let decision = policy::decide(
        &Request {
            host,
            intent: &intent,
            root: &root,
        },
        &Env {
            mode,
            config: &config.routing,
            now_ms: now,
            index: lookup,
            health: &probe,
            session: &state,
        },
    );

    if !dry_run {
        if decision.native_search {
            state.record_native();
        }
        match (&decision.verdict, &decision.fingerprint) {
            (Verdict::Deny(_), Some(print)) => {
                *state.denied.entry(print.clone()).or_default() += 1;
                state.denials += 1;
            }
            (Verdict::Advise(_), _) => state.advisories += 1,
            _ => {}
        }
        let _ = session::save(&root, &key, &mut state, now, &config.routing);
        record_decision(&root, &config, host, mode, tool, &decision, &key, now);
    }

    Some(Outcome {
        host,
        mode,
        intent,
        decision,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_decision(
    root: &std::path::Path,
    config: &Config,
    host: Host,
    mode: RoutingMode,
    tool: &str,
    decision: &Decision,
    key: &str,
    now: u64,
) {
    let base = Event {
        ts: now,
        host: host.as_str().to_string(),
        host_source: host_source(host).to_string(),
        mode: mode.as_str().to_string(),
        tool: tool.to_string(),
        category: decision.category.to_string(),
        reason: decision.reason.to_string(),
        observed: observed::HOST.to_string(),
        sid: Some(key.to_string()),
        ..Event::default()
    };
    let emit = |kind: &str| {
        telemetry::append(
            root,
            &config.routing,
            &Event {
                kind: kind.to_string(),
                ..base.clone()
            },
        );
    };
    if decision.native_search {
        emit(kind::NATIVE_SEARCH);
    }
    match &decision.verdict {
        Verdict::Deny(_) => emit(kind::REDIRECTED),
        Verdict::Advise(_) => emit(kind::ADVISED),
        Verdict::Allow => {
            if decision.native_search && !TRIVIAL_ALLOWS.contains(&decision.reason) {
                emit(kind::FALLBACK_ALLOWED);
            }
        }
    }
}

pub fn render(host: Host, decision: &Decision) -> Option<String> {
    if host == Host::OpenCode {
        let (action, message) = match &decision.verdict {
            Verdict::Allow => ("allow", String::new()),
            Verdict::Advise(message) => ("advise", message.clone()),
            Verdict::Deny(message) => ("deny", message.clone()),
        };
        return Some(
            json!({ "action": action, "message": message, "reason": decision.reason }).to_string(),
        );
    }
    match &decision.verdict {
        Verdict::Allow => None,
        Verdict::Advise(message) => Some(
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "additionalContext": message
                }
            })
            .to_string(),
        ),
        Verdict::Deny(message) => Some(
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": message
                }
            })
            .to_string(),
        ),
    }
}

pub fn guard(input: &Value, host_flag: Option<&str>) -> Result<()> {
    if let Some(outcome) = evaluate(input, host_flag, false, None) {
        if let Some(text) = render(outcome.host, &outcome.decision) {
            println!("{text}");
        }
    }
    Ok(())
}

fn short_tool(name: &str) -> &str {
    name.rsplit("__").next().unwrap_or(name)
}

pub fn note_mcp(input: &Value, host_flag: Option<&str>) -> Result<()> {
    let tool = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !signals::is_devzymer_tool(tool) {
        return Ok(());
    }
    let Some(root) = workspace(input) else {
        return Ok(());
    };
    let config = Config::load(&root).unwrap_or_default();
    let mode = config.routing_mode().0;
    if mode == RoutingMode::Off {
        return Ok(());
    }
    let host = resolve_host(host_flag, input);
    let outcome = match input.get("tool_response") {
        Some(response) => signals::classify_response(tool, response),
        None if input.get("error").is_some() => McpOutcome::Error,
        None => McpOutcome::Ok,
    };
    let now = util::now_millis();
    let key = session::key(input.get("session_id").and_then(Value::as_str));
    let mut state = session::load(&root, &key);
    state.record_mcp(now, outcome);
    let _ = session::save(&root, &key, &mut state, now, &config.routing);

    let base = Event {
        ts: now,
        host: host.as_str().to_string(),
        host_source: host_source(host).to_string(),
        mode: mode.as_str().to_string(),
        tool: short_tool(tool).to_string(),
        reason: outcome.as_str().to_string(),
        observed: observed::HOST.to_string(),
        sid: Some(key),
        ..Event::default()
    };
    telemetry::append(
        &root,
        &config.routing,
        &Event {
            kind: kind::MCP_CALL.to_string(),
            ..base.clone()
        },
    );
    if outcome.is_failure() {
        telemetry::append(
            &root,
            &config.routing,
            &Event {
                kind: kind::RETRIEVAL_FAILURE.to_string(),
                ..base
            },
        );
    }
    Ok(())
}

pub fn note_prompt(input: &Value, host_flag: Option<&str>) -> Result<()> {
    let Some(text) = input.get("prompt").and_then(Value::as_str) else {
        return Ok(());
    };
    if !prompt::requests_native_search(text) {
        return Ok(());
    }
    let Some(root) = workspace(input) else {
        return Ok(());
    };
    let config = Config::load(&root).unwrap_or_default();
    if config.routing_mode().0 == RoutingMode::Off {
        return Ok(());
    }
    let _ = resolve_host(host_flag, input);
    let now = util::now_millis();
    let key = session::key(input.get("session_id").and_then(Value::as_str));
    let mut state = session::load(&root, &key);
    state.bypass_until_ms = now
        + config
            .routing
            .explicit_request_window_secs
            .saturating_mul(1000);
    let _ = session::save(&root, &key, &mut state, now, &config.routing);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use devzymer_indexer::scan::ScanOptions;
    use devzymer_indexer::sync::reconcile;

    fn project(name: &str, mode: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dz-routing-hook-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src").join("billing.rs"),
            "pub fn compute_invoice_total(items: &[u64]) -> u64 { items.iter().sum() }\n",
        )
        .unwrap();
        std::fs::create_dir_all(paths::project_dir(&root)).unwrap();
        std::fs::write(
            paths::project_config_path(&root),
            format!("[routing]\nmode = \"{mode}\"\n"),
        )
        .unwrap();
        let store = Store::open(&paths::db_path(&root)).unwrap();
        reconcile(&store, &root, &ScanOptions::default()).unwrap();
        let now = util::now_millis();
        health::write_heartbeat(
            &root,
            &health::Heartbeat {
                pid: 1,
                started_ms: now,
                updated_ms: now,
                host: "claude".into(),
            },
        );
        root
    }

    fn grep_call(root: &std::path::Path, session: &str, pattern: &str) -> Value {
        json!({
            "tool_name": "Grep",
            "tool_input": { "pattern": pattern },
            "cwd": root.display().to_string(),
            "session_id": session
        })
    }

    #[test]
    fn guard_redirects_once_and_then_lets_the_same_search_through() {
        let root = project("once", "guard");
        let call = grep_call(&root, "s1", "compute_invoice_total");
        let first = evaluate(&call, Some("claude"), false, None).unwrap();
        assert!(matches!(first.decision.verdict, Verdict::Deny(_)));
        let second = evaluate(&call, Some("claude"), false, None).unwrap();
        assert_eq!(second.decision.verdict, Verdict::Allow);
        assert_eq!(second.decision.reason, "retry_after_deny");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn prefer_only_advises_and_off_stays_silent() {
        let root = project("prefer", "prefer");
        let call = grep_call(&root, "s1", "compute_invoice_total");
        let outcome = evaluate(&call, Some("claude"), false, None).unwrap();
        assert!(matches!(outcome.decision.verdict, Verdict::Advise(_)));
        let rendered = render(outcome.host, &outcome.decision).unwrap();
        assert!(rendered.contains("additionalContext"));
        assert!(!rendered.contains("permissionDecision"));
        assert!(evaluate(&call, Some("claude"), false, Some(RoutingMode::Off)).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failed_lookup_unlocks_the_native_search() {
        let root = project("failure", "guard");
        let miss = json!({
            "tool_name": "mcp__devzymer__context_resolve",
            "tool_response": [{ "type": "text", "text": "nao_encontrado: 'x' nao corresponde" }],
            "cwd": root.display().to_string(),
            "session_id": "s1"
        });
        note_mcp(&miss, Some("claude")).unwrap();
        let call = grep_call(&root, "s1", "compute_invoice_total");
        let outcome = evaluate(&call, Some("claude"), false, None).unwrap();
        assert_eq!(outcome.decision.reason, "recent_devzymer_failure");
        assert_eq!(outcome.decision.verdict, Verdict::Allow);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_dead_mcp_server_never_traps_the_agent() {
        let root = project("dead", "guard");
        std::fs::remove_file(health::heartbeat_path(&root)).unwrap();
        let call = grep_call(&root, "s1", "compute_invoice_total");
        let outcome = evaluate(&call, Some("claude"), false, None).unwrap();
        assert_eq!(outcome.decision.reason, "mcp_unavailable");
        assert_eq!(outcome.decision.verdict, Verdict::Allow);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_workspace_is_found_from_a_subdirectory_but_never_from_the_home() {
        let root = project("subdir", "guard");
        let nested = json!({ "cwd": root.join("src").display().to_string() });
        assert_eq!(
            workspace(&nested).map(|found| found.display().to_string()),
            Some(
                workspace_root::canonical(&root)
                    .unwrap()
                    .display()
                    .to_string()
            )
        );
        let home = json!({ "cwd": paths::user_home().display().to_string() });
        assert!(workspace(&home).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_project_without_devzymer_is_left_alone() {
        let root = std::env::temp_dir().join("dz-routing-hook-bare");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let call = grep_call(&root, "s1", "compute_invoice_total");
        assert!(evaluate(&call, Some("claude"), false, None).is_none());
        assert!(!paths::project_dir(&root).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hooks_never_touch_the_index_or_the_sources() {
        let root = project("readonly", "guard");
        let db = paths::db_path(&root);
        let source = root.join("src").join("billing.rs");
        let before = (std::fs::read(&db).unwrap(), std::fs::read(&source).unwrap());
        let call = grep_call(&root, "s1", "compute_invoice_total");
        let _ = evaluate(&call, Some("claude"), false, None);
        let after = (std::fs::read(&db).unwrap(), std::fs::read(&source).unwrap());
        assert!(before.0 == after.0, "index bytes changed");
        assert!(before.1 == after.1, "source changed");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn each_host_gets_its_own_output_shape() {
        let decision = Decision {
            verdict: Verdict::Deny("use devzymer".into()),
            reason: "redirected",
            fingerprint: None,
            category: "grep",
            native_search: true,
        };
        let claude: Value =
            serde_json::from_str(&render(Host::Claude, &decision).unwrap()).unwrap();
        assert_eq!(claude["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(
            claude["hookSpecificOutput"]["permissionDecisionReason"],
            "use devzymer"
        );
        let opencode: Value =
            serde_json::from_str(&render(Host::OpenCode, &decision).unwrap()).unwrap();
        assert_eq!(opencode["action"], "deny");
        let allow = Decision {
            verdict: Verdict::Allow,
            ..decision
        };
        assert!(render(Host::Claude, &allow).is_none());
        assert!(render(Host::Codex, &allow).is_none());
    }
}
