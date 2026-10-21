use std::path::Path;

use devzymer_core::config::{RoutingConfig, RoutingMode};

use crate::classify::{Intent, IntentKind};
use crate::health::Health;
use crate::host::Host;
use crate::session::SessionState;

pub trait IndexLookup {
    fn knows_symbol(&self, name: &str) -> bool;
    fn knows_file_stem(&self, stem: &str) -> bool;
    fn indexed_size(&self, relative: &str) -> Option<u64>;
}

impl IndexLookup for devzymer_core::store::Store {
    fn knows_symbol(&self, name: &str) -> bool {
        self.symbols_by_name(name)
            .map(|symbols| !symbols.is_empty())
            .unwrap_or(false)
    }

    fn knows_file_stem(&self, stem: &str) -> bool {
        self.files_by_stem(&devzymer_core::store::stem_of(stem))
            .map(|files| !files.is_empty())
            .unwrap_or(false)
    }

    fn indexed_size(&self, relative: &str) -> Option<u64> {
        self.file_by_path(relative)
            .ok()
            .flatten()
            .map(|file| file.size_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Advise(String),
    Deny(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub verdict: Verdict,
    pub reason: &'static str,
    pub fingerprint: Option<String>,
    pub category: &'static str,
    pub native_search: bool,
}

pub struct Request<'a> {
    pub host: Host,
    pub intent: &'a Intent,
    pub root: &'a Path,
}

pub struct Env<'a> {
    pub mode: RoutingMode,
    pub config: &'a RoutingConfig,
    pub now_ms: u64,
    pub index: Option<&'a dyn IndexLookup>,
    pub health: &'a dyn Fn() -> Health,
    pub session: &'a SessionState,
}

const UNCOVERED_DIRECTORIES: [&str; 12] = [
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "out",
    "coverage",
];

const LARGE_READ_BYTES: u64 = 24_000;

pub fn fingerprint(category: &str, term: &str) -> String {
    let key = format!("{category}\0{}", term.to_ascii_lowercase());
    blake3::hash(key.as_bytes()).to_hex()[..12].to_string()
}

fn is_absolute_like(text: &str) -> bool {
    text.starts_with('/')
        || (text.len() >= 3
            && text.as_bytes()[1] == b':'
            && text.as_bytes()[2] == b'/'
            && text.as_bytes()[0].is_ascii_alphabetic())
}

fn relative_to_root(scope: &str, root: &Path) -> Option<String> {
    let normalized = scope.replace('\\', "/");
    if is_absolute_like(&normalized) {
        let root_text = root.to_string_lossy().replace('\\', "/");
        let (haystack, needle) = if cfg!(windows) {
            (
                normalized.to_ascii_lowercase(),
                root_text.to_ascii_lowercase(),
            )
        } else {
            (normalized.clone(), root_text.clone())
        };
        let rest = haystack.strip_prefix(needle.as_str())?;
        let offset = normalized.len() - rest.len();
        return Some(normalized[offset..].trim_start_matches('/').to_string());
    }
    Some(normalized.trim_start_matches("./").to_string())
}

fn outside_coverage(scope: &str, root: &Path) -> bool {
    let Some(relative) = relative_to_root(scope, root) else {
        return true;
    };
    relative
        .split('/')
        .next()
        .is_some_and(|first| UNCOVERED_DIRECTORIES.contains(&first))
}

fn answer(intent: &Intent, index: &dyn IndexLookup) -> Option<String> {
    intent
        .terms
        .iter()
        .find(|term| match intent.kind {
            IntentKind::CodeSearch => index.knows_symbol(term) || index.knows_file_stem(term),
            IntentKind::FileDiscovery => index.knows_file_stem(term) || index.knows_symbol(term),
            IntentKind::FileRead => false,
        })
        .cloned()
}

fn allow(reason: &'static str, category: &'static str, native_search: bool) -> Decision {
    Decision {
        verdict: Verdict::Allow,
        reason,
        fingerprint: None,
        category,
        native_search,
    }
}

pub fn decide(request: &Request, env: &Env) -> Decision {
    let intent = request.intent;
    let category = intent.category;
    let prefix = request.host.mcp_tool_prefix();

    if env.mode == RoutingMode::Off {
        return allow("mode_off", category, intent.kind != IntentKind::FileRead);
    }

    if intent.kind == IntentKind::FileRead {
        return decide_read(request, env, prefix);
    }

    let window = env.config.recent_use_window_secs.saturating_mul(1000);
    let failure_window = env.config.failure_window_secs.saturating_mul(1000);
    if env.session.bypass_active(env.now_ms) {
        return allow("explicit_request", category, true);
    }
    if env.session.recent_failure(env.now_ms, failure_window) {
        return allow("recent_devzymer_failure", category, true);
    }
    if env.session.recently_used(env.now_ms, window) {
        return allow("complementary_evidence", category, true);
    }
    if let Some(scope) = intent.scope.as_deref() {
        if outside_coverage(scope, request.root) {
            return allow("outside_coverage", category, true);
        }
    }
    let Some(index) = env.index else {
        return allow("index_unavailable", category, true);
    };
    let Some(term) = answer(intent, index) else {
        return allow("not_answerable", category, true);
    };

    match env.mode {
        RoutingMode::Off => allow("mode_off", category, true),
        RoutingMode::Prefer => {
            if env.session.advisories >= env.config.max_advisories_per_session {
                return allow("advice_exhausted", category, true);
            }
            Decision {
                verdict: Verdict::Advise(format!(
                    "devzymer tip: {prefix}context_resolve with query \"{term}\" locates this directly \
                     (path, symbol and line range, no repository scan). Use native search if it \
                     returns nothing useful."
                )),
                reason: "advised",
                fingerprint: None,
                category,
                native_search: true,
            }
        }
        RoutingMode::Guard => {
            let health = (env.health)();
            if let Some(blocker) = health.blocker() {
                return allow(blocker, category, true);
            }
            let print = fingerprint(category, &term);
            if env.session.denied.get(&print).copied().unwrap_or(0) > 0 {
                return allow("retry_after_deny", category, true);
            }
            if env.session.denials >= env.config.max_denials_per_session {
                return allow("deny_budget_exhausted", category, true);
            }
            Decision {
                verdict: Verdict::Deny(format!(
                    "devzymer can answer this lookup. Call {prefix}context_resolve with query \
                     \"{term}\": it returns the path, symbol and line range without scanning the \
                     repository; then {prefix}context_read for just that range. This search is \
                     redirected once. If devzymer is unavailable or returns nothing useful, repeat \
                     the same search and it will be allowed."
                )),
                reason: "redirected",
                fingerprint: Some(print),
                category,
                native_search: true,
            }
        }
    }
}

fn decide_read(request: &Request, env: &Env, prefix: &str) -> Decision {
    let category = request.intent.category;
    let quiet = allow("read_allowed", category, false);
    if env.mode == RoutingMode::Off {
        return allow("mode_off", category, false);
    }
    if env.session.advisories >= env.config.max_advisories_per_session
        || env.session.recently_used(
            env.now_ms,
            env.config.recent_use_window_secs.saturating_mul(1000),
        )
    {
        return quiet;
    }
    let (Some(index), Some(scope)) = (env.index, request.intent.scope.as_deref()) else {
        return quiet;
    };
    let Some(relative) = relative_to_root(scope, request.root) else {
        return quiet;
    };
    match index.indexed_size(&relative) {
        Some(size) if size >= LARGE_READ_BYTES => Decision {
            verdict: Verdict::Advise(format!(
                "devzymer tip: {relative} is indexed and large. To explore it, {prefix}context_read \
                 with a symbol or line range returns only the needed excerpt. A whole-file Read is \
                 fine when you need to edit it."
            )),
            reason: "read_advice",
            fingerprint: None,
            category,
            native_search: false,
        },
        _ => quiet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Intent;
    use crate::health::Freshness;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Default)]
    struct FakeIndex {
        symbols: BTreeSet<String>,
        stems: BTreeSet<String>,
        sizes: BTreeMap<String, u64>,
    }

    impl IndexLookup for FakeIndex {
        fn knows_symbol(&self, name: &str) -> bool {
            self.symbols.contains(name)
        }
        fn knows_file_stem(&self, stem: &str) -> bool {
            self.stems.contains(&stem.to_ascii_lowercase())
        }
        fn indexed_size(&self, relative: &str) -> Option<u64> {
            self.sizes.get(relative).copied()
        }
    }

    fn index() -> FakeIndex {
        let mut index = FakeIndex::default();
        index.symbols.insert("useVoiceCall".into());
        index.stems.insert("usevoicecall".into());
        index.sizes.insert("src/big.ts".into(), 90_000);
        index.sizes.insert("src/small.ts".into(), 900);
        index
    }

    fn healthy() -> Health {
        Health {
            checked_ms: 1,
            index_present: true,
            index_ok: true,
            workspace_match: true,
            schema_current: true,
            fts_ok: true,
            freshness: Freshness::Fresh,
            stale_files: 0,
            mcp_alive: true,
        }
    }

    fn search(term: &str) -> Intent {
        Intent {
            kind: IntentKind::CodeSearch,
            category: "grep",
            terms: vec![term.to_string()],
            scope: None,
        }
    }

    fn run(
        mode: RoutingMode,
        intent: &Intent,
        session: &SessionState,
        health: Health,
        lookup: Option<&dyn IndexLookup>,
    ) -> Decision {
        let config = RoutingConfig::default();
        let probe = move || health.clone();
        decide(
            &Request {
                host: Host::Claude,
                intent,
                root: Path::new("/repo"),
            },
            &Env {
                mode,
                config: &config,
                now_ms: 1_000_000,
                index: lookup,
                health: &probe,
                session,
            },
        )
    }

    #[test]
    fn guard_redirects_a_lookup_the_index_can_answer() {
        let index = index();
        let decision = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &SessionState::default(),
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "redirected");
        let Verdict::Deny(message) = decision.verdict else {
            panic!("expected a denial");
        };
        assert!(message.contains("mcp__devzymer__context_resolve"));
        assert!(message.contains("useVoiceCall"));
        assert!(message.contains("repeat the same search"));
        assert!(decision.fingerprint.is_some());
    }

    #[test]
    fn prefer_never_blocks_it_only_advises() {
        let index = index();
        let decision = run(
            RoutingMode::Prefer,
            &search("useVoiceCall"),
            &SessionState::default(),
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "advised");
        assert!(matches!(decision.verdict, Verdict::Advise(_)));
    }

    #[test]
    fn off_changes_nothing() {
        let index = index();
        let decision = run(
            RoutingMode::Off,
            &search("useVoiceCall"),
            &SessionState::default(),
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "mode_off");
        assert_eq!(decision.verdict, Verdict::Allow);
    }

    #[test]
    fn a_lookup_the_index_cannot_answer_is_left_alone() {
        let index = index();
        for mode in [RoutingMode::Guard, RoutingMode::Prefer] {
            let decision = run(
                mode,
                &search("completelyUnknownName"),
                &SessionState::default(),
                healthy(),
                Some(&index),
            );
            assert_eq!(decision.reason, "not_answerable");
            assert_eq!(decision.verdict, Verdict::Allow);
        }
        let plain = Intent {
            terms: Vec::new(),
            ..search("x")
        };
        assert_eq!(
            run(
                RoutingMode::Guard,
                &plain,
                &SessionState::default(),
                healthy(),
                Some(&index)
            )
            .reason,
            "not_answerable"
        );
    }

    #[test]
    fn the_same_search_is_redirected_only_once() {
        let index = index();
        let mut session = SessionState::default();
        let first = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &session,
            healthy(),
            Some(&index),
        );
        assert_eq!(first.reason, "redirected");
        session.denied.insert(first.fingerprint.unwrap(), 1);
        session.denials = 1;
        let second = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &session,
            healthy(),
            Some(&index),
        );
        assert_eq!(second.reason, "retry_after_deny");
        assert_eq!(second.verdict, Verdict::Allow);
    }

    #[test]
    fn the_denial_budget_of_a_session_is_bounded() {
        let index = index();
        let session = SessionState {
            denials: RoutingConfig::default().max_denials_per_session,
            ..SessionState::default()
        };
        let decision = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &session,
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "deny_budget_exhausted");
        assert_eq!(decision.verdict, Verdict::Allow);
    }

    #[test]
    fn every_unhealthy_index_state_allows_the_native_search() {
        let index = index();
        let cases = [
            (
                Health {
                    index_present: false,
                    ..healthy()
                },
                "index_missing",
            ),
            (
                Health {
                    workspace_match: false,
                    ..healthy()
                },
                "workspace_mismatch",
            ),
            (
                Health {
                    index_ok: false,
                    ..healthy()
                },
                "index_unhealthy",
            ),
            (
                Health {
                    fts_ok: false,
                    ..healthy()
                },
                "index_unhealthy",
            ),
            (
                Health {
                    schema_current: false,
                    ..healthy()
                },
                "index_schema_outdated",
            ),
            (
                Health {
                    freshness: Freshness::Stale,
                    stale_files: 3,
                    ..healthy()
                },
                "index_stale",
            ),
            (
                Health {
                    freshness: Freshness::Unknown,
                    ..healthy()
                },
                "index_freshness_unknown",
            ),
            (
                Health {
                    mcp_alive: false,
                    ..healthy()
                },
                "mcp_unavailable",
            ),
        ];
        for (health, reason) in cases {
            let decision = run(
                RoutingMode::Guard,
                &search("useVoiceCall"),
                &SessionState::default(),
                health,
                Some(&index),
            );
            assert_eq!(decision.reason, reason);
            assert_eq!(decision.verdict, Verdict::Allow, "{reason}");
        }
    }

    #[test]
    fn a_failed_devzymer_lookup_unlocks_native_search() {
        let index = index();
        let mut session = SessionState::default();
        session.record_mcp(1_000_000 - 5_000, crate::signals::McpOutcome::NotFound);
        let decision = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &session,
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "recent_devzymer_failure");
        assert_eq!(decision.verdict, Verdict::Allow);
    }

    #[test]
    fn a_session_that_already_used_devzymer_may_search_natively() {
        let index = index();
        let mut session = SessionState::default();
        session.record_mcp(1_000_000 - 10_000, crate::signals::McpOutcome::Ok);
        let decision = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &session,
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "complementary_evidence");
        assert_eq!(decision.verdict, Verdict::Allow);
    }

    #[test]
    fn an_explicit_user_request_for_native_search_is_honored() {
        let index = index();
        let session = SessionState {
            bypass_until_ms: 2_000_000,
            ..SessionState::default()
        };
        let decision = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &session,
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "explicit_request");
        assert_eq!(decision.verdict, Verdict::Allow);
    }

    #[test]
    fn a_missing_lookup_backend_allows_the_search() {
        let decision = run(
            RoutingMode::Guard,
            &search("useVoiceCall"),
            &SessionState::default(),
            healthy(),
            None,
        );
        assert_eq!(decision.reason, "index_unavailable");
    }

    #[test]
    fn searches_outside_the_indexed_tree_are_never_redirected() {
        let index = index();
        for scope in [
            "node_modules/react",
            "target/debug",
            "/elsewhere/project",
            ".git",
        ] {
            let intent = Intent {
                scope: Some(scope.to_string()),
                ..search("useVoiceCall")
            };
            let decision = run(
                RoutingMode::Guard,
                &intent,
                &SessionState::default(),
                healthy(),
                Some(&index),
            );
            assert_eq!(decision.reason, "outside_coverage", "{scope}");
        }
        let inside = Intent {
            scope: Some("src".to_string()),
            ..search("useVoiceCall")
        };
        assert_eq!(
            run(
                RoutingMode::Guard,
                &inside,
                &SessionState::default(),
                healthy(),
                Some(&index)
            )
            .reason,
            "redirected"
        );
    }

    #[test]
    fn a_file_name_lookup_is_redirected_by_stem() {
        let index = index();
        let intent = Intent {
            kind: IntentKind::FileDiscovery,
            category: "find",
            terms: vec!["useVoiceCall".into()],
            scope: None,
        };
        let decision = run(
            RoutingMode::Guard,
            &intent,
            &SessionState::default(),
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "redirected");
    }

    #[test]
    fn reading_a_file_is_never_blocked_because_edits_need_it() {
        let index = index();
        let intent = Intent {
            kind: IntentKind::FileRead,
            category: "read-tool",
            terms: Vec::new(),
            scope: Some("src/big.ts".into()),
        };
        let guard = run(
            RoutingMode::Guard,
            &intent,
            &SessionState::default(),
            healthy(),
            Some(&index),
        );
        assert!(matches!(guard.verdict, Verdict::Advise(_)));
        assert!(!guard.native_search);
        let small = Intent {
            scope: Some("src/small.ts".into()),
            ..intent.clone()
        };
        let quiet = run(
            RoutingMode::Guard,
            &small,
            &SessionState::default(),
            healthy(),
            Some(&index),
        );
        assert_eq!(quiet.verdict, Verdict::Allow);
        let unknown = Intent {
            scope: Some("src/other.ts".into()),
            ..intent
        };
        assert_eq!(
            run(
                RoutingMode::Prefer,
                &unknown,
                &SessionState::default(),
                healthy(),
                Some(&index)
            )
            .verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn advice_is_rate_limited_per_session() {
        let index = index();
        let session = SessionState {
            advisories: RoutingConfig::default().max_advisories_per_session,
            ..SessionState::default()
        };
        let decision = run(
            RoutingMode::Prefer,
            &search("useVoiceCall"),
            &session,
            healthy(),
            Some(&index),
        );
        assert_eq!(decision.reason, "advice_exhausted");
        assert_eq!(decision.verdict, Verdict::Allow);
    }

    #[test]
    fn the_tool_name_follows_the_host() {
        let index = index();
        let config = RoutingConfig::default();
        let session = SessionState::default();
        let probe = healthy;
        let decision = decide(
            &Request {
                host: Host::OpenCode,
                intent: &search("useVoiceCall"),
                root: Path::new("/repo"),
            },
            &Env {
                mode: RoutingMode::Guard,
                config: &config,
                now_ms: 1_000_000,
                index: Some(&index),
                health: &probe,
                session: &session,
            },
        );
        let Verdict::Deny(message) = decision.verdict else {
            panic!("expected a denial");
        };
        assert!(message.contains("devzymer_context_resolve"));
        assert!(!message.contains("mcp__devzymer__"));
    }

    #[test]
    fn the_fingerprint_hides_the_term_and_ignores_case() {
        let print = fingerprint("grep", "UseVoiceCall");
        assert_eq!(print.len(), 12);
        assert!(!print.contains("voice"));
        assert_eq!(print, fingerprint("grep", "usevoicecall"));
        assert_ne!(print, fingerprint("rg", "usevoicecall"));
    }
}
