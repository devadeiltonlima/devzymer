use std::path::Path;

use serde::Serialize;

use crate::runner::{BenchmarkConfig, GroundTruth, TaskResult};
use crate::tasks::TaskKind;

pub const BENCHMARK_VERSION: &str = "devzymer-bench-2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Json,
    Markdown,
}

/// The estimator used to count tokens. It is an estimate, not the tokeniser of
/// the model that will consume the context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenEstimator {
    /// Four characters per token, with a word based correction.
    HeuristicChars,
}

impl TokenEstimator {
    pub fn id(self) -> &'static str {
        "heuristic-chars-v1"
    }
}

/// Signed reduction between the context returned and the baseline.
///
/// A negative value is legitimate: the retrieval may return more context than
/// reading the whole file would have cost.
pub fn signed_reduction_pct(context_tokens: usize, baseline_tokens: usize) -> Option<f64> {
    if baseline_tokens == 0 {
        return None;
    }
    let value = (baseline_tokens as f64 - context_tokens as f64) / baseline_tokens as f64 * 100.0;
    if !value.is_finite() {
        return None;
    }
    Some(value)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RepositoryIdentity {
    pub name: String,
    pub files: u64,
    pub symbols: u64,
    pub relations: u64,
    pub index_revision: u64,
    pub parser_version: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LatencySummary {
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TokenSummary {
    pub context_tokens: u64,
    pub baseline_tokens: u64,
    /// Signed. Negative when the context is larger than the baseline.
    pub reduction_pct: Option<f64>,
    /// Tasks with no baseline, excluded from the percentage.
    pub tasks_without_baseline: usize,
    pub tasks_with_negative_reduction: usize,
    pub worst_reduction_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheSummary {
    pub hits: usize,
    pub misses: usize,
    pub disabled: usize,
    pub hit_rate: f64,
    pub mean_latency_hit_ms: f64,
    pub mean_latency_miss_ms: f64,
    /// The context returned on a hit is the same as on a miss, so a cache
    /// saves work and not tokens.
    pub context_identical_on_hit: bool,
    pub token_saving_from_cache_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct KindSummary {
    pub kind: String,
    pub tasks: usize,
    pub verified_tasks: usize,
    pub hits: usize,
    pub hit_rate: f64,
    /// Not applicable to the scenarios without a ranked target.
    pub recall_at_1: Option<f64>,
    pub recall_at_5: Option<f64>,
    pub mrr: Option<f64>,
    pub mean_latency_ms: f64,
    pub context_tokens: u64,
    pub baseline_tokens: u64,
    pub reduction_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BenchmarkReport {
    pub benchmark_version: String,
    pub generated_at_ms: u64,
    pub repository: RepositoryIdentity,
    pub seed: u64,
    pub token_estimator: String,
    pub top_k: usize,
    pub candidate_window: usize,
    /// Distinct tasks, without counting the repeated executions.
    pub unique_tasks: usize,
    /// Distinct query strings, which is smaller when a query is repeated.
    pub unique_queries: usize,
    /// Every execution, including the repeated ones.
    pub executions: usize,
    pub verified_tasks: usize,
    pub unverified_tasks: usize,
    pub hits: usize,
    pub hit_rate: f64,
    pub recall_at_1: f64,
    pub recall_at_5: f64,
    pub mrr: f64,
    pub latency: LatencySummary,
    pub tokens: TokenSummary,
    pub cache: CacheSummary,
    pub by_kind: Vec<KindSummary>,
    pub results: Vec<TaskResult>,
    pub notes: Vec<String>,
}

impl BenchmarkReport {
    pub fn from_results(
        root: &Path,
        store: &devzymer_core::store::Store,
        results: Vec<TaskResult>,
        config: &BenchmarkConfig,
    ) -> Self {
        let mut report = BenchmarkReport {
            benchmark_version: BENCHMARK_VERSION.to_string(),
            generated_at_ms: devzymer_core::util::now_millis(),
            repository: crate::runner::repository_identity(root, store),
            seed: config.seed,
            token_estimator: crate::runner::TOKEN_ESTIMATOR.id().to_string(),
            top_k: config.top_k,
            candidate_window: config.candidate_window,
            executions: results.len(),
            results,
            notes: vec![
                "as contagens de tokens são estimativas locais e não o tokenizador do Claude".into(),
                "a linha de base é a leitura do arquivo inteiro que contém a resposta".into(),
                "a verdade de base de cada tarefa é validada contra o arquivo no disco".into(),
                "a redução é com sinal: valores negativos significam contexto maior que a linha de base"
                    .into(),
                "o cache de recuperação economiza trabalho de banco, não tokens devolvidos".into(),
            ],
            ..Default::default()
        };

        report.unique_tasks = report
            .results
            .iter()
            .map(|r| r.task.id.clone())
            .collect::<std::collections::HashSet<_>>()
            .len();
        report.unique_queries = report
            .results
            .iter()
            .map(|r| r.task.query.clone())
            .collect::<std::collections::HashSet<_>>()
            .len();
        report.verified_tasks = report
            .results
            .iter()
            .filter(|r| matches!(r.ground_truth, GroundTruth::Verified))
            .count();
        report.unverified_tasks = report
            .results
            .iter()
            .filter(|r| matches!(r.ground_truth, GroundTruth::Unverified(_)))
            .count();

        let scored: Vec<&TaskResult> = report
            .results
            .iter()
            .filter(|r| {
                !matches!(r.ground_truth, GroundTruth::Unverified(_))
                    && !matches!(
                        r.task.kind,
                        TaskKind::MissingSymbol | TaskKind::AmbiguousSymbol
                    )
            })
            .collect();
        let denominator = scored.len().max(1) as f64;

        report.hits = scored.iter().filter(|r| r.hit).count();
        report.hit_rate = report.hits as f64 / denominator;
        report.recall_at_1 =
            scored.iter().filter(|r| r.rank == Some(1)).count() as f64 / denominator;
        report.recall_at_5 = scored
            .iter()
            .filter(|r| r.rank.map(|rank| rank <= 5).unwrap_or(false))
            .count() as f64
            / denominator;
        report.mrr = scored
            .iter()
            .filter_map(|r| r.rank)
            .map(|rank| 1.0 / rank as f64)
            .sum::<f64>()
            / denominator;

        let mut latencies: Vec<u64> = report.results.iter().map(|r| r.latency_ms).collect();
        latencies.sort_unstable();
        report.latency = LatencySummary {
            mean_ms: if latencies.is_empty() {
                0.0
            } else {
                latencies.iter().sum::<u64>() as f64 / latencies.len() as f64
            },
            p50_ms: percentile(&latencies, 0.50),
            p95_ms: percentile(&latencies, 0.95),
            p99_ms: percentile(&latencies, 0.99),
            max_ms: latencies.last().copied().unwrap_or(0) as f64,
        };

        report.tokens = token_summary(&report.results);
        report.cache = cache_summary(&report.results);

        let mut kinds: Vec<TaskKind> = report.results.iter().map(|r| r.task.kind).collect();
        kinds.sort();
        kinds.dedup();
        for kind in kinds {
            let group: Vec<&TaskResult> = report
                .results
                .iter()
                .filter(|r| r.task.kind == kind)
                .collect();
            let verifiable: Vec<&TaskResult> = group
                .iter()
                .filter(|r| !matches!(r.ground_truth, GroundTruth::Unverified(_)))
                .copied()
                .collect();
            let denominator = verifiable.len().max(1) as f64;
            let ranked = !matches!(
                kind,
                TaskKind::MissingSymbol | TaskKind::AmbiguousSymbol | TaskKind::DependencyTraversal
            );
            let context_tokens: u64 = group.iter().map(|r| r.context_tokens as u64).sum();
            let baseline_tokens: u64 = group.iter().map(|r| r.baseline_tokens as u64).sum();
            report.by_kind.push(KindSummary {
                kind: kind.display().to_string(),
                tasks: group.len(),
                verified_tasks: verifiable.len(),
                hits: verifiable.iter().filter(|r| r.hit).count(),
                hit_rate: verifiable.iter().filter(|r| r.hit).count() as f64 / denominator,
                recall_at_1: ranked_recall(&verifiable, denominator, |r| r.rank == Some(1)),
                recall_at_5: ranked_recall(&verifiable, denominator, |r| {
                    r.rank.map(|rank| rank <= 5).unwrap_or(false)
                }),
                mrr: if ranked {
                    Some(
                        verifiable
                            .iter()
                            .filter_map(|r| r.rank)
                            .map(|rank| 1.0 / rank as f64)
                            .sum::<f64>()
                            / denominator,
                    )
                } else {
                    None
                },
                mean_latency_ms: if group.is_empty() {
                    0.0
                } else {
                    group.iter().map(|r| r.latency_ms).sum::<u64>() as f64 / group.len() as f64
                },
                context_tokens,
                baseline_tokens,
                reduction_pct: signed_reduction_pct(
                    context_tokens as usize,
                    baseline_tokens as usize,
                ),
            });
        }

        report
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# devzymer benchmark\n\n");
        out.push_str(&format!("- versão: {}\n", self.benchmark_version));
        out.push_str(&format!("- repositório: {}\n", self.repository.name));
        out.push_str(&format!(
            "- índice: {} arquivos, {} símbolos, revisão {}, parser {}\n",
            self.repository.files,
            self.repository.symbols,
            self.repository.index_revision,
            self.repository.parser_version
        ));
        out.push_str(&format!("- semente: {}\n", self.seed));
        out.push_str(&format!(
            "- estimador de tokens: {}\n",
            self.token_estimator
        ));
        out.push_str(&format!(
            "- tarefas únicas: {} | consultas únicas: {} | execuções: {} | verificadas: {} | não verificadas: {}\n",
            self.unique_tasks,
            self.unique_queries,
            self.executions,
            self.verified_tasks,
            self.unverified_tasks
        ));
        out.push_str(&format!(
            "- acerto no top {}: {} ({:.1}%)\n",
            self.top_k,
            self.hits,
            self.hit_rate * 100.0
        ));
        out.push_str(&format!("- recall@1: {:.1}%\n", self.recall_at_1 * 100.0));
        out.push_str(&format!("- recall@5: {:.1}%\n", self.recall_at_5 * 100.0));
        out.push_str(&format!("- MRR: {:.3}\n", self.mrr));
        out.push_str(&format!(
            "- latência: média {:.1} ms | p50 {:.1} | p95 {:.1} | p99 {:.1} | máx {:.1}\n",
            self.latency.mean_ms,
            self.latency.p50_ms,
            self.latency.p95_ms,
            self.latency.p99_ms,
            self.latency.max_ms
        ));
        match self.tokens.reduction_pct {
            Some(value) => out.push_str(&format!(
                "- tokens: {} de contexto | {} de linha de base | redução {:+.2}%\n",
                self.tokens.context_tokens, self.tokens.baseline_tokens, value
            )),
            None => out.push_str("- tokens: redução não calculável (linha de base zero)\n"),
        }
        if self.tokens.tasks_with_negative_reduction > 0 {
            out.push_str(&format!(
                "- tarefas com redução negativa: {} (o contexto devolvido é maior que a linha de base)\n",
                self.tokens.tasks_with_negative_reduction
            ));
        }
        out.push_str(&format!(
            "- cache: {} acertos, {} falhas, {} desativado | latência acerto {:.2} ms | falha {:.2} ms\n",
            self.cache.hits,
            self.cache.misses,
            self.cache.disabled,
            self.cache.mean_latency_hit_ms,
            self.cache.mean_latency_miss_ms
        ));
        out.push_str(&format!(
            "- economia de tokens vinda do cache: {}\n\n",
            match self.cache.token_saving_from_cache_pct {
                Some(value) => format!("{value:+.2}%"),
                None => "não aplicável".to_string(),
            }
        ));

        out.push_str("| cenário | tarefas | verificadas | acertos | recall@1 | recall@5 | MRR | latência ms | redução |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for kind in &self.by_kind {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {:.1}% | {:.1}% | {:.3} | {:.1} | {} |\n",
                kind.kind,
                kind.tasks,
                kind.verified_tasks,
                kind.hits,
                kind.recall_at_1.map(|v| v * 100.0).unwrap_or(0.0),
                kind.recall_at_5.map(|v| v * 100.0).unwrap_or(0.0),
                kind.mrr
                    .map(|v| format!("{v:.3}"))
                    .unwrap_or_else(|| "n/d".into()),
                kind.mean_latency_ms,
                match kind.reduction_pct {
                    Some(value) => format!("{value:+.2}%"),
                    None => "n/d".to_string(),
                }
            ));
        }

        let failures: Vec<&TaskResult> = report_failures(&self.results);
        if !failures.is_empty() {
            out.push_str("\n## falhas\n\n");
            for failure in failures {
                out.push_str(&format!(
                    "- [{}] `{}` — {}\n",
                    failure.task.kind.display(),
                    failure.task.query,
                    failure
                        .failure_reason
                        .clone()
                        .unwrap_or_else(|| "sem motivo registrado".to_string())
                ));
            }
        }

        if !self.notes.is_empty() {
            out.push_str("\n## notas\n\n");
            for note in &self.notes {
                out.push_str(&format!("- {note}\n"));
            }
        }
        out
    }
}

fn report_failures(results: &[TaskResult]) -> Vec<&TaskResult> {
    results
        .iter()
        .filter(|r| r.failure_reason.is_some())
        .collect()
}

fn percentile(sorted: &[u64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64 * quantile).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index] as f64
}

fn token_summary(results: &[TaskResult]) -> TokenSummary {
    let context_tokens: u64 = results.iter().map(|r| r.context_tokens as u64).sum();
    let baseline_tokens: u64 = results.iter().map(|r| r.baseline_tokens as u64).sum();
    let without_baseline = results
        .iter()
        .filter(|r| r.baseline_tokens == 0 && r.context_tokens > 0)
        .count();
    let negative: Vec<f64> = results
        .iter()
        .filter_map(|r| r.token_reduction_pct)
        .filter(|value| *value < 0.0)
        .collect();
    TokenSummary {
        context_tokens,
        baseline_tokens,
        reduction_pct: signed_reduction_pct(context_tokens as usize, baseline_tokens as usize),
        tasks_without_baseline: without_baseline,
        tasks_with_negative_reduction: negative.len(),
        worst_reduction_pct: negative.iter().cloned().fold(None, |worst, value| {
            Some(match worst {
                Some(current) if current <= value => current,
                _ => value,
            })
        }),
    }
}

fn cache_summary(results: &[TaskResult]) -> CacheSummary {
    // The main measurements run with the cache disabled, so the reported
    // latency is the real retrieval cost. The cache is measured by the
    // repeated scenario, which runs the same query with an empty cache and
    // then again.
    let repeated: Vec<&TaskResult> = results
        .iter()
        .filter(|r| r.cache_cold_latency_ms.is_some())
        .collect();
    let mean = |values: Vec<u64>| -> f64 {
        if values.is_empty() {
            0.0
        } else {
            values.iter().sum::<u64>() as f64 / values.len() as f64
        }
    };
    let cold: Vec<u64> = repeated
        .iter()
        .filter_map(|r| r.cache_cold_latency_ms)
        .collect();
    let warm: Vec<u64> = repeated
        .iter()
        .filter_map(|r| r.cache_warm_latency_ms)
        .collect();
    let hits = repeated
        .iter()
        .filter(|r| r.cache == devzymer_search::resolve::CacheStatus::Hit)
        .count();
    let total = repeated.len();
    CacheSummary {
        hits,
        misses: total.saturating_sub(hits),
        disabled: results
            .iter()
            .filter(|r| r.cache == devzymer_search::resolve::CacheStatus::Disabled)
            .count(),
        hit_rate: if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        },
        mean_latency_hit_ms: mean(warm),
        mean_latency_miss_ms: mean(cold),
        context_identical_on_hit: true,
        token_saving_from_cache_pct: Some(0.0),
    }
}

/// The recall of a scenario, when the scenario has a ranked target.
fn ranked_recall<F>(results: &[&TaskResult], denominator: f64, predicate: F) -> Option<f64>
where
    F: Fn(&&TaskResult) -> bool,
{
    let count = results.iter().filter(|r| predicate(r)).count();
    Some(count as f64 / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_reduction_is_reported_with_its_sign() {
        let value = signed_reduction_pct(170, 139).expect("value");
        assert!((value + 22.30).abs() < 0.01, "unexpected value {value}");
        assert!(value < 0.0);
    }

    #[test]
    fn a_positive_reduction_is_reported_with_its_sign() {
        let value = signed_reduction_pct(100, 400).expect("value");
        assert!((value - 75.0).abs() < 0.001, "unexpected value {value}");
    }

    #[test]
    fn a_zero_baseline_has_no_percentage() {
        assert!(signed_reduction_pct(120, 0).is_none());
    }

    #[test]
    fn an_impossible_percentage_is_never_produced() {
        for (context, baseline) in [(0usize, 0usize), (10, 0), (0, 10), (usize::MAX, 1)] {
            if let Some(value) = signed_reduction_pct(context, baseline) {
                assert!(
                    value.is_finite(),
                    "non finite value for {context}/{baseline}"
                );
            }
        }
    }
}
