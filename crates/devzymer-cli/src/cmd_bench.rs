use std::path::PathBuf;

use anyhow::Result;
use devzymer_bench::runner::BenchmarkConfig;
use devzymer_bench::tasks::TaskKind;
use devzymer_core::paths;
use devzymer_core::store::Store;

use crate::{Ctx, OutputFormat};

#[allow(clippy::too_many_arguments)]
pub fn run(
    ctx: &Ctx,
    tasks: usize,
    format: OutputFormat,
    output: Option<PathBuf>,
    seed: u64,
    kinds: Option<String>,
    top_k: usize,
    curated: bool,
) -> Result<()> {
    let root = ctx.resolve_root();
    let db = paths::db_path(&root);
    if !db.exists() {
        anyhow::bail!(
            "índice não encontrado em {}. Execute `devzymer init` primeiro.",
            db.display()
        );
    }
    let store = Store::open(&db)?;
    let selected: Vec<TaskKind> = match kinds.as_deref() {
        Some(list) => {
            let mut parsed = Vec::new();
            for name in list.split(',') {
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                let kind = TaskKind::all()
                    .iter()
                    .find(|k| k.display() == name)
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("cenário desconhecido: {name}"))?;
                parsed.push(kind);
            }
            parsed
        }
        None => TaskKind::all().to_vec(),
    };
    let config = BenchmarkConfig {
        tasks_per_kind: tasks,
        top_k,
        candidate_window: 50,
        include_baseline: true,
        seed,
        kinds: selected,
        curated,
    };
    let report = devzymer_bench::run_benchmarks(&root, &store, &ctx.config, &config)?;

    let text = match format {
        OutputFormat::Json => report.to_json(),
        OutputFormat::Markdown => report.to_markdown(),
        OutputFormat::Text => summary(&report),
    };

    if let Some(path) = output {
        std::fs::write(&path, text.as_bytes()).map_err(|e| devzymer_core::Error::io(&path, e))?;
        println!("relatório gravado em {}", path.display());
        return Ok(());
    }

    println!("{text}");
    Ok(())
}

fn summary(report: &devzymer_bench::BenchmarkReport) -> String {
    let reduction = match report.tokens.reduction_pct {
        Some(value) => format!("{value:+.2}%"),
        None => "n/d (linha de base zero)".to_string(),
    };
    format!(
        "repositório: {} | índice: {} arquivos, {} símbolos, revisão {}\nsemente: {} | estimador: {}\ntarefas únicas: {} | execuções: {} | verificadas: {} | não verificadas: {}\nacerto no top {}: {} ({:.1}%) | recall@1 {:.1}% | recall@5 {:.1}% | MRR {:.3}\nlatência: média {:.1} ms | p50 {:.1} | p95 {:.1} | p99 {:.1}\ntokens: {} de contexto | {} de linha de base | redução {}\ncache: {} acertos, {} falhas | economia de tokens do cache: {}",
        report.repository.name,
        report.repository.files,
        report.repository.symbols,
        report.repository.index_revision,
        report.seed,
        report.token_estimator,
        report.unique_tasks,
        report.executions,
        report.verified_tasks,
        report.unverified_tasks,
        report.top_k,
        report.hits,
        report.hit_rate * 100.0,
        report.recall_at_1 * 100.0,
        report.recall_at_5 * 100.0,
        report.mrr,
        report.latency.mean_ms,
        report.latency.p50_ms,
        report.latency.p95_ms,
        report.latency.p99_ms,
        report.tokens.context_tokens,
        report.tokens.baseline_tokens,
        reduction,
        report.cache.hits,
        report.cache.misses,
        match report.cache.token_saving_from_cache_pct {
            Some(value) => format!("{value:+.2}%"),
            None => "n/d".to_string(),
        }
    )
}
