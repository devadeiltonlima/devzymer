use std::path::{Path, PathBuf};

use devzymer_core::config::Config;
use devzymer_core::store::Store;
use devzymer_indexer::index::{IndexOptions, Indexer};
use devzymer_indexer::scan::ScanOptions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Health {
    pub checked_ms: u64,
    pub index_present: bool,
    pub index_ok: bool,
    pub workspace_match: bool,
    pub schema_current: bool,
    pub fts_ok: bool,
    pub freshness: Freshness,
    pub stale_files: usize,
    pub mcp_alive: bool,
}

impl Health {
    pub fn blocker(&self) -> Option<&'static str> {
        if !self.index_present {
            return Some("index_missing");
        }
        if !self.workspace_match {
            return Some("workspace_mismatch");
        }
        if !self.index_ok || !self.fts_ok {
            return Some("index_unhealthy");
        }
        if !self.schema_current {
            return Some("index_schema_outdated");
        }
        match self.freshness {
            Freshness::Stale => return Some("index_stale"),
            Freshness::Unknown => return Some("index_freshness_unknown"),
            Freshness::Fresh => {}
        }
        if !self.mcp_alive {
            return Some("mcp_unavailable");
        }
        None
    }

    pub fn ready(&self) -> bool {
        self.blocker().is_none()
    }

    fn missing(now_ms: u64) -> Health {
        Health {
            checked_ms: now_ms,
            index_present: false,
            index_ok: false,
            workspace_match: true,
            schema_current: false,
            fts_ok: false,
            freshness: Freshness::Unknown,
            stale_files: 0,
            mcp_alive: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    pub pid: u32,
    pub started_ms: u64,
    pub updated_ms: u64,
    pub host: String,
}

pub fn heartbeat_path(root: &Path) -> PathBuf {
    crate::routing_dir(root).join("heartbeat.json")
}

pub fn write_heartbeat(root: &Path, heartbeat: &Heartbeat) {
    let path = heartbeat_path(root);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(bytes) = serde_json::to_vec(heartbeat) {
        let _ = devzymer_core::config::write_atomic(&path, &bytes);
    }
}

pub fn read_heartbeat(root: &Path) -> Option<Heartbeat> {
    std::fs::read_to_string(heartbeat_path(root))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

pub fn clear_heartbeat(root: &Path, pid: u32) {
    if read_heartbeat(root).is_some_and(|beat| beat.pid == pid) {
        let _ = std::fs::remove_file(heartbeat_path(root));
    }
}

pub fn mcp_alive(root: &Path, max_age_ms: u64, now_ms: u64) -> bool {
    read_heartbeat(root).is_some_and(|beat| now_ms.saturating_sub(beat.updated_ms) <= max_age_ms)
}

fn cache_path(root: &Path) -> PathBuf {
    crate::routing_dir(root).join("health.json")
}

pub fn probe(root: &Path, config: &Config, now_ms: u64) -> Health {
    let ttl_ms = config.routing.health_cache_secs.saturating_mul(1000);
    let heartbeat_ms = config.routing.heartbeat_max_age_secs.saturating_mul(1000);
    if ttl_ms > 0 {
        if let Some(mut cached) = std::fs::read_to_string(cache_path(root))
            .ok()
            .and_then(|text| serde_json::from_str::<Health>(&text).ok())
        {
            if now_ms.saturating_sub(cached.checked_ms) < ttl_ms && cached.index_present {
                cached.mcp_alive = mcp_alive(root, heartbeat_ms, now_ms);
                return cached;
            }
        }
    }
    let health = probe_uncached(root, config, now_ms);
    if let Some(parent) = cache_path(root).parent() {
        if std::fs::create_dir_all(parent).is_ok() {
            if let Ok(bytes) = serde_json::to_vec(&health) {
                let _ = devzymer_core::config::write_atomic(&cache_path(root), &bytes);
            }
        }
    }
    health
}

pub fn probe_uncached(root: &Path, config: &Config, now_ms: u64) -> Health {
    let heartbeat_ms = config.routing.heartbeat_max_age_secs.saturating_mul(1000);
    let db = devzymer_core::paths::db_path(root);
    if !db.exists() {
        return Health::missing(now_ms);
    }
    let alive = mcp_alive(root, heartbeat_ms, now_ms);
    let store = match Store::open_read_only(&db) {
        Ok(store) => store,
        Err(devzymer_core::Error::WorkspaceMismatch(_)) => {
            return Health {
                workspace_match: false,
                mcp_alive: alive,
                index_present: true,
                ..Health::missing(now_ms)
            };
        }
        Err(_) => {
            return Health {
                index_present: true,
                mcp_alive: alive,
                ..Health::missing(now_ms)
            };
        }
    };

    let structure_ok = store
        .quick_check()
        .map(|value| value == "ok")
        .unwrap_or(false);
    let fts_ok = store.fts_available()
        && store
            .fts_search_terms(&["probe".to_string()], 1, [1.0; 6])
            .is_ok();
    let schema_current = store.schema_is_current() && !store.parser_version_changed();

    let (freshness, stale_files) = if structure_ok && fts_ok {
        let indexer = Indexer::new(&store, root);
        match indexer.changed_files(&IndexOptions {
            root: root.to_path_buf(),
            scan: ScanOptions {
                exclude: config.index.exclude.clone(),
                exclude_secret_files: config.privacy.exclude_secret_files,
                max_file_size: config.index.max_file_size_bytes,
                follow_symlinks: config.index.follow_symlinks,
                respect_gitignore: config.index.respect_gitignore,
                include_hidden: config.index.include_hidden,
                max_duration_ms: None,
                ..ScanOptions::default()
            },
            force_full: false,
            chunk_size: 256,
        }) {
            Ok(changed) if changed.is_empty() => (Freshness::Fresh, 0),
            Ok(changed) => (Freshness::Stale, changed.len()),
            Err(_) => (Freshness::Unknown, 0),
        }
    } else {
        (Freshness::Unknown, 0)
    };

    Health {
        checked_ms: now_ms,
        index_present: true,
        index_ok: structure_ok,
        workspace_match: true,
        schema_current,
        fts_ok,
        freshness,
        stale_files,
        mcp_alive: alive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_healthy_index_with_a_live_server_is_ready() {
        assert!(healthy().ready());
        assert_eq!(healthy().blocker(), None);
    }

    #[test]
    fn every_failure_has_its_own_reason() {
        let cases: Vec<(Health, &str)> = vec![
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
                    stale_files: 2,
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
            assert_eq!(health.blocker(), Some(reason));
            assert!(!health.ready());
        }
    }

    #[test]
    fn a_missing_index_is_reported_without_creating_one() {
        let root =
            std::env::temp_dir().join(format!("dz-health-{}", devzymer_core::util::now_millis()));
        std::fs::create_dir_all(&root).unwrap();
        let health = probe_uncached(&root, &Config::default(), 10);
        assert_eq!(health.blocker(), Some("index_missing"));
        assert!(!devzymer_core::paths::db_path(&root).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_stale_heartbeat_means_the_server_is_gone() {
        let root =
            std::env::temp_dir().join(format!("dz-beat-{}", devzymer_core::util::now_millis()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(!mcp_alive(&root, 90_000, 1_000));
        let beat = Heartbeat {
            pid: 42,
            started_ms: 1_000,
            updated_ms: 1_000,
            host: "claude".into(),
        };
        write_heartbeat(&root, &beat);
        assert!(mcp_alive(&root, 90_000, 50_000));
        assert!(!mcp_alive(&root, 90_000, 1_000 + 90_001));
        clear_heartbeat(&root, 7);
        assert!(read_heartbeat(&root).is_some());
        clear_heartbeat(&root, 42);
        assert!(read_heartbeat(&root).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
