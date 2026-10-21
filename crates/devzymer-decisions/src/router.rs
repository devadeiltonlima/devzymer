use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use devzymer_core::config::DecisionsConfig;
use devzymer_core::store::Store;
use devzymer_core::Result;

use crate::provider::{DecisionProvider, DeterministicProvider};
use crate::schema::{DecisionOutcome, DecisionRequest, DecisionTask};

/// Trips after a number of consecutive failures and blocks calls for a
/// cooldown period, so an unavailable provider never breaks the engine.
pub struct CircuitBreaker {
    threshold: u32,
    cooldown: Duration,
    failures: AtomicU32,
    opened_at: Mutex<Option<Instant>>,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            threshold: threshold.max(1),
            cooldown,
            failures: AtomicU32::new(0),
            opened_at: Mutex::new(None),
        }
    }

    pub fn is_open(&self) -> bool {
        let guard = match self.opened_at.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match *guard {
            Some(opened) => opened.elapsed() < self.cooldown,
            None => false,
        }
    }

    pub fn record_failure(&self) {
        let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        if failures >= self.threshold {
            if let Ok(mut guard) = self.opened_at.lock() {
                *guard = Some(Instant::now());
            }
        }
    }

    pub fn record_success(&self) {
        self.failures.store(0, Ordering::SeqCst);
        if let Ok(mut guard) = self.opened_at.lock() {
            *guard = None;
        }
    }
}

pub struct DecisionRouter {
    config: DecisionsConfig,
    deterministic: DeterministicProvider,
    remote: Option<Arc<dyn DecisionProvider>>,
    breaker: CircuitBreaker,
    store: Option<Arc<Mutex<Store>>>,
}

impl DecisionRouter {
    pub fn new(
        config: DecisionsConfig,
        remote: Option<Arc<dyn DecisionProvider>>,
        store: Option<Arc<Mutex<Store>>>,
    ) -> Self {
        let breaker = CircuitBreaker::new(
            config.circuit_breaker_threshold,
            Duration::from_secs(config.circuit_breaker_cooldown_secs),
        );
        Self {
            config,
            deterministic: DeterministicProvider,
            remote,
            breaker,
            store,
        }
    }

    pub fn provider_available(&self) -> bool {
        self.config.enabled && self.remote.is_some()
    }

    /// Resolves a decision following the deterministic first hierarchy.
    pub async fn route(&self, request: &DecisionRequest) -> Result<DecisionOutcome> {
        let local = self
            .deterministic
            .decide(request)
            .await
            .map_err(|e| devzymer_core::Error::other(e.to_string()))?;
        let confidence = local.confidence.unwrap_or(0.0);

        if !self.config.enabled || self.remote.is_none() {
            return Ok(self.apply_escalation(local, request));
        }
        if confidence >= 0.7 {
            return Ok(self.apply_escalation(local, request));
        }
        if self.breaker.is_open() {
            return Ok(self.apply_escalation(local, request));
        }

        if let Some(store) = &self.store {
            let cached = store
                .lock()
                .ok()
                .and_then(|guard| {
                    guard
                        .decision_cache_get(&request.cache_key(), now_ms())
                        .ok()
                })
                .flatten();
            if let Some(payload) = cached {
                if let Ok(mut cached) = serde_json::from_str::<DecisionOutcome>(&payload) {
                    cached.cached = true;
                    return Ok(self.apply_escalation(cached, request));
                }
            }
        }

        let provider = self.remote.as_ref().expect("provider checked above");
        match provider.decide(request).await {
            Ok(outcome) => {
                self.breaker.record_success();
                if let Some(store) = &self.store {
                    let expires = now_ms() + self.config.cache_ttl_secs * 1000;
                    if let Ok(payload) = serde_json::to_string(&outcome) {
                        if let Ok(guard) = store.lock() {
                            let _ =
                                guard.decision_cache_put(&request.cache_key(), &payload, expires);
                        }
                    }
                }
                Ok(self.apply_escalation(outcome, request))
            }
            Err(err) => {
                self.breaker.record_failure();
                tracing::warn!(error = %err, "decision provider failed, using deterministic fallback");
                Ok(self.apply_escalation(local, request))
            }
        }
    }

    fn apply_escalation(
        &self,
        mut outcome: DecisionOutcome,
        request: &DecisionRequest,
    ) -> DecisionOutcome {
        if request.task == DecisionTask::EscalateToClaude && outcome.is_yes() {
            outcome.escalated = true;
            outcome.rationale = Some(
                outcome
                    .rationale
                    .unwrap_or_else(|| "escalated by decision policy".to_string()),
            );
        }
        if !outcome.escalated
            && request.task == DecisionTask::EscalateToClaude
            && outcome.choice.is_none()
        {
            outcome.escalated = true;
        }
        outcome
    }
}

fn now_ms() -> u64 {
    devzymer_core::util::now_millis()
}

/// Builds the remote provider described by the configuration, when a key is
/// available in the environment.
pub fn build_remote_provider(
    config: &DecisionsConfig,
) -> Result<Option<Arc<dyn DecisionProvider>>> {
    if !config.enabled {
        return Ok(None);
    }
    let key_env = if config.api_key_env.is_empty() {
        "TYPESAFE_API_KEY"
    } else {
        &config.api_key_env
    };
    let Ok(api_key) = std::env::var(key_env) else {
        return Ok(None);
    };
    if api_key.trim().is_empty() {
        return Ok(None);
    }
    let model = model_name(&config.provider, &config.model);
    let provider: Arc<dyn crate::provider::DecisionProvider> = match config.provider.as_str() {
        "openrouter" => Arc::new(
            crate::provider::OpenRouterProvider::new(
                config.base_url.clone(),
                api_key,
                model,
                Duration::from_secs(config.timeout_secs),
                config.max_retries,
            )
            .map_err(|e| devzymer_core::Error::other(e.to_string()))?,
        ),
        _ => Arc::new(
            crate::provider::JevProvider::new(
                config.base_url.clone(),
                api_key,
                model,
                Duration::from_secs(config.timeout_secs),
                config.max_retries,
            )
            .map_err(|e| devzymer_core::Error::other(e.to_string()))?,
        ),
    };
    Ok(Some(provider))
}

/// The System One API expects a bare model identifier, without a vendor prefix.
pub fn model_name(provider: &str, model: &str) -> String {
    match provider {
        "typesafe" => model.rsplit('/').next().unwrap_or(model).to_string(),
        _ => model.to_string(),
    }
}
