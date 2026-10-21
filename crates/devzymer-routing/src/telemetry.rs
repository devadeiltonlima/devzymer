use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use devzymer_core::config::RoutingConfig;
use serde::{Deserialize, Serialize};

pub mod kind {
    pub const MCP_CALL: &str = "mcp_call";
    pub const NATIVE_SEARCH: &str = "native_search";
    pub const REDIRECTED: &str = "redirected";
    pub const FALLBACK_ALLOWED: &str = "fallback_allowed";
    pub const RETRIEVAL_FAILURE: &str = "retrieval_failure";
    pub const ADVISED: &str = "advised";
}

pub mod observed {
    pub const HOST: &str = "host";
    pub const SERVER: &str = "server";
    pub const INFERRED: &str = "inferred";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Event {
    pub v: u8,
    pub ts: u64,
    pub host: String,
    pub host_source: String,
    pub mode: String,
    pub kind: String,
    pub tool: String,
    pub category: String,
    pub reason: String,
    pub latency_ms: Option<u64>,
    pub observed: String,
    pub sid: Option<String>,
}

impl Default for Event {
    fn default() -> Self {
        Event {
            v: 1,
            ts: 0,
            host: "unknown".into(),
            host_source: "unknown".into(),
            mode: String::new(),
            kind: String::new(),
            tool: String::new(),
            category: String::new(),
            reason: String::new(),
            latency_ms: None,
            observed: observed::INFERRED.into(),
            sid: None,
        }
    }
}

pub fn events_path(root: &Path) -> PathBuf {
    crate::routing_dir(root).join("events.jsonl")
}

pub fn append(root: &Path, config: &RoutingConfig, event: &Event) {
    if !config.telemetry {
        return;
    }
    let path = events_path(root);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > config.telemetry_max_bytes {
            let _ = std::fs::rename(&path, path.with_extension("1.jsonl"));
        }
    }
    let Ok(mut line) = serde_json::to_string(event) else {
        return;
    };
    line.push('\n');
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

pub fn read_since(root: &Path, since_ms: u64) -> Vec<Event> {
    let mut events = Vec::new();
    let path = events_path(root);
    for candidate in [path.with_extension("1.jsonl"), path] {
        let Ok(text) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        for line in text.lines() {
            if let Ok(event) = serde_json::from_str::<Event>(line) {
                if event.ts >= since_ms {
                    events.push(event);
                }
            }
        }
    }
    events.sort_by_key(|event| event.ts);
    events
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct HostSummary {
    pub mcp_calls: u64,
    pub native_searches: u64,
    pub redirected: u64,
    pub fallback_allowed: u64,
    pub advised: u64,
    pub retrieval_failures: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Summary {
    pub window_days: u64,
    pub events: u64,
    pub by_host: BTreeMap<String, HostSummary>,
    pub by_mode: BTreeMap<String, HostSummary>,
    pub fallback_reasons: BTreeMap<String, u64>,
    pub sessions: u64,
    pub devzymer_first_sessions: u64,
    pub native_first_sessions: u64,
    pub devzymer_first_rate: Option<f64>,
    pub native_before_first_devzymer: u64,
    pub host_observed: u64,
    pub server_observed: u64,
    pub inferred: u64,
    pub latency_p50_ms: Option<u64>,
    pub latency_p95_ms: Option<u64>,
}

fn bump(summary: &mut HostSummary, event: &Event) {
    match event.kind.as_str() {
        kind::MCP_CALL if event.observed == observed::SERVER => {
            summary.mcp_calls += 1;
            if matches!(
                event.reason.as_str(),
                "not_found" | "error" | "budget_exhausted"
            ) {
                summary.retrieval_failures += 1;
            }
        }
        kind::MCP_CALL => {}
        kind::NATIVE_SEARCH => summary.native_searches += 1,
        kind::REDIRECTED => summary.redirected += 1,
        kind::FALLBACK_ALLOWED => summary.fallback_allowed += 1,
        kind::ADVISED => summary.advised += 1,
        _ => {}
    }
}

fn percentile(sorted: &[u64], fraction: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted.get(index).copied()
}

pub fn summarize(events: &[Event], window_days: u64) -> Summary {
    let mut summary = Summary {
        window_days,
        events: events.len() as u64,
        ..Summary::default()
    };
    let mut latencies: Vec<u64> = Vec::new();
    let mut per_session: BTreeMap<&str, (bool, u64, u64)> = BTreeMap::new();

    for event in events {
        bump(
            summary.by_host.entry(event.host.clone()).or_default(),
            event,
        );
        bump(
            summary.by_mode.entry(event.mode.clone()).or_default(),
            event,
        );
        match event.observed.as_str() {
            observed::HOST => summary.host_observed += 1,
            observed::SERVER => summary.server_observed += 1,
            _ => summary.inferred += 1,
        }
        if event.kind == kind::FALLBACK_ALLOWED && !event.reason.is_empty() {
            *summary
                .fallback_reasons
                .entry(event.reason.clone())
                .or_default() += 1;
        }
        if event.kind == kind::MCP_CALL && event.observed == observed::SERVER {
            if let Some(latency) = event.latency_ms {
                latencies.push(latency);
            }
        }
        if let Some(sid) = event.sid.as_deref() {
            let is_native = event.kind == kind::NATIVE_SEARCH;
            let is_mcp = event.kind == kind::MCP_CALL && event.observed == observed::HOST;
            if is_native || is_mcp {
                let entry = per_session.entry(sid).or_insert((false, 0, 0));
                if is_mcp {
                    entry.0 = true;
                    entry.2 += 1;
                } else if !entry.0 {
                    entry.1 += 1;
                }
            }
        }
    }

    for (devzymer_seen, native_before, _) in per_session.values() {
        summary.sessions += 1;
        if *native_before == 0 && *devzymer_seen {
            summary.devzymer_first_sessions += 1;
        } else if *native_before > 0 {
            summary.native_first_sessions += 1;
            summary.native_before_first_devzymer += native_before;
        }
    }
    let decided = summary.devzymer_first_sessions + summary.native_first_sessions;
    if decided > 0 {
        summary.devzymer_first_rate = Some(summary.devzymer_first_sessions as f64 / decided as f64);
    }
    latencies.sort_unstable();
    summary.latency_p50_ms = percentile(&latencies, 0.5);
    summary.latency_p95_ms = percentile(&latencies, 0.95);
    summary
}

pub fn distinct_sessions(events: &[Event]) -> usize {
    events
        .iter()
        .filter_map(|event| event.sid.as_deref())
        .collect::<BTreeSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, sid: &str, ts: u64, observed: &str) -> Event {
        Event {
            ts,
            host: "claude".into(),
            mode: "guard".into(),
            kind: kind.into(),
            observed: observed.into(),
            sid: Some(sid.into()),
            ..Event::default()
        }
    }

    #[test]
    fn a_session_that_asks_devzymer_first_counts_as_devzymer_first() {
        let events = vec![
            event(kind::MCP_CALL, "a", 1, observed::HOST),
            event(kind::NATIVE_SEARCH, "a", 2, observed::HOST),
            event(kind::NATIVE_SEARCH, "b", 3, observed::HOST),
            event(kind::MCP_CALL, "b", 4, observed::HOST),
        ];
        let summary = summarize(&events, 7);
        assert_eq!(summary.sessions, 2);
        assert_eq!(summary.devzymer_first_sessions, 1);
        assert_eq!(summary.native_first_sessions, 1);
        assert_eq!(summary.native_before_first_devzymer, 1);
        assert_eq!(summary.devzymer_first_rate, Some(0.5));
    }

    #[test]
    fn a_session_with_only_native_searches_is_native_first() {
        let events = vec![
            event(kind::NATIVE_SEARCH, "a", 1, observed::HOST),
            event(kind::NATIVE_SEARCH, "a", 2, observed::HOST),
        ];
        let summary = summarize(&events, 7);
        assert_eq!(summary.native_first_sessions, 1);
        assert_eq!(summary.native_before_first_devzymer, 2);
        assert_eq!(summary.devzymer_first_rate, Some(0.0));
    }

    #[test]
    fn no_rate_is_reported_without_evidence() {
        let summary = summarize(&[], 7);
        assert_eq!(summary.devzymer_first_rate, None);
        let only_server = vec![event(kind::MCP_CALL, "a", 1, observed::SERVER)];
        assert_eq!(summarize(&only_server, 7).devzymer_first_rate, None);
    }

    #[test]
    fn host_observed_and_inferred_data_are_counted_apart() {
        let mut inferred = event(kind::MCP_CALL, "a", 1, observed::INFERRED);
        inferred.host = "unknown".into();
        let events = vec![
            event(kind::NATIVE_SEARCH, "a", 1, observed::HOST),
            event(kind::MCP_CALL, "a", 2, observed::SERVER),
            inferred,
        ];
        let summary = summarize(&events, 7);
        assert_eq!(summary.host_observed, 1);
        assert_eq!(summary.server_observed, 1);
        assert_eq!(summary.inferred, 1);
        assert_eq!(summary.by_host.len(), 2);
    }

    #[test]
    fn the_latency_comes_only_from_the_server_observations() {
        let mut slow = event(kind::MCP_CALL, "a", 1, observed::SERVER);
        slow.latency_ms = Some(40);
        let mut fast = event(kind::MCP_CALL, "a", 2, observed::SERVER);
        fast.latency_ms = Some(10);
        let mut hostside = event(kind::MCP_CALL, "a", 3, observed::HOST);
        hostside.latency_ms = Some(9_999);
        let summary = summarize(&[slow, fast, hostside], 7);
        assert_eq!(summary.latency_p50_ms, Some(40));
        assert_eq!(summary.latency_p95_ms, Some(40));
    }

    #[test]
    fn the_fallback_reasons_are_tallied() {
        let mut one = event(kind::FALLBACK_ALLOWED, "a", 1, observed::HOST);
        one.reason = "index_stale".into();
        let mut two = event(kind::FALLBACK_ALLOWED, "a", 2, observed::HOST);
        two.reason = "index_stale".into();
        let mut three = event(kind::FALLBACK_ALLOWED, "b", 3, observed::HOST);
        three.reason = "retry_after_deny".into();
        let summary = summarize(&[one, two, three], 7);
        assert_eq!(summary.fallback_reasons.get("index_stale"), Some(&2));
        assert_eq!(summary.fallback_reasons.get("retry_after_deny"), Some(&1));
    }

    #[test]
    fn the_log_never_stores_a_command_or_a_prompt() {
        let root = std::env::temp_dir().join(format!(
            "dz-telemetry-{}",
            devzymer_core::util::now_millis()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut sample = event(kind::NATIVE_SEARCH, "a", 10, observed::HOST);
        sample.category = "grep".into();
        append(&root, &RoutingConfig::default(), &sample);
        let text = std::fs::read_to_string(events_path(&root)).unwrap();
        assert!(text.contains("\"category\":\"grep\""));
        assert!(!text.contains("pattern"));
        assert!(!text.contains("command"));
        assert!(!text.contains("prompt"));
        let read = read_since(&root, 0);
        assert_eq!(read, vec![sample]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn telemetry_can_be_turned_off() {
        let root = std::env::temp_dir().join(format!(
            "dz-telemetry-off-{}",
            devzymer_core::util::now_millis()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = RoutingConfig {
            telemetry: false,
            ..RoutingConfig::default()
        };
        append(
            &root,
            &config,
            &event(kind::MCP_CALL, "a", 1, observed::HOST),
        );
        assert!(!events_path(&root).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_log_rotates_when_it_grows_past_the_limit() {
        let root = std::env::temp_dir().join(format!(
            "dz-telemetry-rot-{}",
            devzymer_core::util::now_millis()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = RoutingConfig {
            telemetry_max_bytes: 200,
            ..RoutingConfig::default()
        };
        for ts in 1..=20 {
            append(
                &root,
                &config,
                &event(kind::MCP_CALL, "a", ts, observed::HOST),
            );
        }
        assert!(events_path(&root).with_extension("1.jsonl").exists());
        assert!(std::fs::metadata(events_path(&root)).unwrap().len() < 1_000);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
