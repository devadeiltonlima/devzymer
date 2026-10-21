use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use devzymer_core::config::RoutingConfig;
use serde::{Deserialize, Serialize};

use crate::signals::McpOutcome;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SessionState {
    pub created_ms: u64,
    pub updated_ms: u64,
    pub denied: BTreeMap<String, u32>,
    pub denials: u32,
    pub advisories: u32,
    pub mcp_calls: u32,
    pub last_mcp_ms: u64,
    pub last_failure_ms: u64,
    pub failure_streak: u32,
    pub ambiguous_streak: u32,
    pub bypass_until_ms: u64,
    pub first_retrieval: Option<String>,
}

impl SessionState {
    pub fn record_mcp(&mut self, now_ms: u64, outcome: McpOutcome) {
        self.mcp_calls += 1;
        self.last_mcp_ms = now_ms;
        self.first_retrieval
            .get_or_insert_with(|| "mcp".to_string());
        match outcome {
            McpOutcome::Ok => {
                self.failure_streak = 0;
                self.ambiguous_streak = 0;
            }
            McpOutcome::Ambiguous => {
                self.ambiguous_streak += 1;
                if self.ambiguous_streak >= 2 {
                    self.failure_streak += 1;
                    self.last_failure_ms = now_ms;
                }
            }
            McpOutcome::NotFound | McpOutcome::BudgetExhausted | McpOutcome::Error => {
                self.failure_streak += 1;
                self.last_failure_ms = now_ms;
            }
        }
    }

    pub fn record_native(&mut self) {
        self.first_retrieval
            .get_or_insert_with(|| "native".to_string());
    }

    pub fn recently_used(&self, now_ms: u64, window_ms: u64) -> bool {
        self.last_mcp_ms != 0 && now_ms.saturating_sub(self.last_mcp_ms) <= window_ms
    }

    pub fn recent_failure(&self, now_ms: u64, window_ms: u64) -> bool {
        self.failure_streak > 0
            && self.last_failure_ms != 0
            && now_ms.saturating_sub(self.last_failure_ms) <= window_ms
    }

    pub fn bypass_active(&self, now_ms: u64) -> bool {
        self.bypass_until_ms > now_ms
    }
}

pub fn key(raw: Option<&str>) -> String {
    match raw {
        Some(value) if !value.trim().is_empty() => {
            blake3::hash(value.as_bytes()).to_hex()[..12].to_string()
        }
        _ => "anon".to_string(),
    }
}

pub fn dir(root: &Path) -> PathBuf {
    crate::routing_dir(root).join("sessions")
}

fn path(root: &Path, key: &str) -> PathBuf {
    dir(root).join(format!("{key}.json"))
}

pub fn load(root: &Path, key: &str) -> SessionState {
    std::fs::read_to_string(path(root, key))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(
    root: &Path,
    key: &str,
    state: &mut SessionState,
    now_ms: u64,
    config: &RoutingConfig,
) -> std::io::Result<()> {
    if state.created_ms == 0 {
        state.created_ms = now_ms;
    }
    state.updated_ms = now_ms;
    let directory = dir(root);
    std::fs::create_dir_all(&directory)?;
    let bytes = serde_json::to_vec(state).map_err(std::io::Error::other)?;
    devzymer_core::config::write_atomic(&path(root, key), &bytes)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    prune(
        &directory,
        now_ms,
        config.session_ttl_secs.saturating_mul(1000),
    );
    Ok(())
}

fn prune(directory: &Path, now_ms: u64, ttl_ms: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten().take(200) {
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as u64);
        if let Some(modified) = modified {
            if now_ms.saturating_sub(modified) > ttl_ms {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_is_stored_only_as_a_short_hash() {
        let first = key(Some("session-123"));
        assert_eq!(first.len(), 12);
        assert!(!first.contains("session"));
        assert_eq!(first, key(Some("session-123")));
        assert_ne!(first, key(Some("session-124")));
        assert_eq!(key(None), "anon");
        assert_eq!(key(Some("  ")), "anon");
    }

    #[test]
    fn a_failed_lookup_opens_the_fallback_window() {
        let mut state = SessionState::default();
        assert!(!state.recent_failure(1_000, 300_000));
        state.record_mcp(1_000, McpOutcome::NotFound);
        assert!(state.recent_failure(2_000, 300_000));
        assert!(!state.recent_failure(1_000 + 300_001, 300_000));
        state.record_mcp(5_000, McpOutcome::Ok);
        assert!(!state.recent_failure(6_000, 300_000));
        assert_eq!(state.first_retrieval.as_deref(), Some("mcp"));
    }

    #[test]
    fn only_repeated_ambiguity_counts_as_a_failure() {
        let mut state = SessionState::default();
        state.record_mcp(1_000, McpOutcome::Ambiguous);
        assert!(!state.recent_failure(1_500, 300_000));
        state.record_mcp(2_000, McpOutcome::Ambiguous);
        assert!(state.recent_failure(2_500, 300_000));
    }

    #[test]
    fn an_exhausted_output_budget_and_an_error_are_failures() {
        for outcome in [McpOutcome::BudgetExhausted, McpOutcome::Error] {
            let mut state = SessionState::default();
            state.record_mcp(1_000, outcome);
            assert!(state.recent_failure(1_200, 300_000), "{outcome:?}");
        }
    }

    #[test]
    fn recent_use_is_bounded_by_the_window() {
        let mut state = SessionState::default();
        assert!(!state.recently_used(10_000, 180_000));
        state.record_mcp(10_000, McpOutcome::Ok);
        assert!(state.recently_used(100_000, 180_000));
        assert!(!state.recently_used(10_000 + 180_001, 180_000));
    }

    #[test]
    fn the_first_retrieval_keeps_the_first_source() {
        let mut state = SessionState::default();
        state.record_native();
        state.record_mcp(1_000, McpOutcome::Ok);
        assert_eq!(state.first_retrieval.as_deref(), Some("native"));
    }

    #[test]
    fn a_session_survives_a_round_trip_on_disk() {
        let root =
            std::env::temp_dir().join(format!("dz-session-{}", devzymer_core::util::now_millis()));
        std::fs::create_dir_all(&root).unwrap();
        let config = RoutingConfig::default();
        let mut state = SessionState::default();
        state.denied.insert("abc".to_string(), 1);
        state.denials = 1;
        save(&root, "k1", &mut state, 5_000, &config).unwrap();
        let loaded = load(&root, "k1");
        assert_eq!(loaded.denials, 1);
        assert_eq!(loaded.denied.get("abc"), Some(&1));
        assert_eq!(loaded.created_ms, 5_000);
        assert_eq!(load(&root, "missing"), SessionState::default());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
