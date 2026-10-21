use std::collections::{HashMap, HashSet};
use std::path::Path;

use devzymer_core::config::Config;
use devzymer_core::model::Symbol;
use devzymer_core::store::Store;
use devzymer_core::Result;
use devzymer_search::read::read_whole;
use devzymer_search::resolve::{CacheStatus, ResolveQuery};
use devzymer_search::Resolver;
use serde::Serialize;

use crate::report::{BenchmarkReport, TokenEstimator};
use crate::source;
use crate::tasks::{BenchmarkTask, CuratedTask, QueryOrigin, SeededRng, TaskKind};

#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub tasks_per_kind: usize,
    pub top_k: usize,
    /// How many candidates are inspected to determine the true rank.
    pub candidate_window: usize,
    pub include_baseline: bool,
    pub seed: u64,
    pub kinds: Vec<TaskKind>,
    pub curated: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            tasks_per_kind: 10,
            top_k: 5,
            candidate_window: 50,
            include_baseline: true,
            seed: 20_260_319,
            kinds: TaskKind::all().to_vec(),
            curated: true,
        }
    }
}

/// Whether the expected answer was confirmed against the source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum GroundTruth {
    Verified,
    Unverified(String),
    NotApplicable,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateInfo {
    pub qualified_name: String,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub kind: String,
    pub source: String,
    pub confidence: String,
    pub score: f64,
    pub matched_terms: usize,
    pub identity_matches: usize,
    pub evidence: f64,
    pub path_hint: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskResult {
    pub task: BenchmarkTask,
    pub ground_truth: GroundTruth,
    /// The expected symbol was retrieved inside the candidate window.
    pub found: bool,
    /// The expected symbol is within the top K.
    pub hit: bool,
    pub rank: Option<usize>,
    pub latency_ms: u64,
    pub context_tokens: usize,
    pub baseline_tokens: usize,
    /// Signed reduction. Negative when the returned context is larger than the
    /// baseline, which is a legitimate outcome.
    pub token_reduction_pct: Option<f64>,
    pub candidates: usize,
    pub candidate_list: Vec<CandidateInfo>,
    pub cache: CacheStatus,
    /// Latency of the first retrieval, with an empty cache.
    pub cache_cold_latency_ms: Option<u64>,
    /// Latency of the second retrieval, served from the cache.
    pub cache_warm_latency_ms: Option<u64>,
    pub ambiguity: Option<AmbiguityInfo>,
    pub failure_reason: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AmbiguityInfo {
    pub name: String,
    pub count: usize,
    pub candidates: Vec<String>,
}

/// Runs the retrieval benchmark suite against the live index.
pub fn run_benchmarks(
    root: &Path,
    store: &Store,
    config: &Config,
    benchmark: &BenchmarkConfig,
) -> Result<BenchmarkReport> {
    let tasks = build_tasks(store, root, benchmark)?;
    let mut cold_config = config.clone();
    cold_config.context.cache_enabled = false;
    let resolver = Resolver::new(store, &cold_config);
    let mut warm_config = config.clone();
    warm_config.context.cache_enabled = true;
    let warm_resolver = Resolver::new(store, &warm_config);
    let graph = devzymer_graph::GraphQuery::new(store);
    let mut results: Vec<TaskResult> = Vec::new();

    for task in tasks {
        results.push(run_task(
            root,
            store,
            &resolver,
            &warm_resolver,
            &graph,
            &task,
            benchmark,
        ));
    }

    Ok(BenchmarkReport::from_results(
        root, store, results, benchmark,
    ))
}

fn run_task(
    root: &Path,
    store: &Store,
    resolver: &Resolver,
    warm_resolver: &Resolver,
    graph: &devzymer_graph::GraphQuery,
    task: &BenchmarkTask,
    benchmark: &BenchmarkConfig,
) -> TaskResult {
    let started = std::time::Instant::now();
    let query = ResolveQuery {
        query: task.query.clone(),
        path: None,
        language: None,
        limit: benchmark.candidate_window,
        candidate_limit: benchmark.candidate_window,
        diagnostics: false,
    };
    let mut cache_cold_latency_ms = None;
    let mut cache_warm_latency_ms = None;
    let mut warm_cache = CacheStatus::Disabled;
    let resolved = if task.kind == TaskKind::RepeatedSearch {
        // The cache is emptied so the cold and the warm retrieval can be
        // measured in the same run.
        let _ = store.cache_clear();
        let cold_started = std::time::Instant::now();
        let cold = warm_resolver.resolve(&query);
        cache_cold_latency_ms = Some(cold_started.elapsed().as_micros() as u64 / 1000);
        let warm_started = std::time::Instant::now();
        let warm = warm_resolver.resolve(&query);
        cache_warm_latency_ms = Some(warm_started.elapsed().as_micros() as u64 / 1000);
        warm_cache = warm.map(|set| set.cache).unwrap_or(CacheStatus::Disabled);
        cold
    } else {
        resolver.resolve(&query)
    };
    let latency_ms = started.elapsed().as_micros() as u64 / 1000;

    match resolved {
        Ok(set) => {
            let candidates = &set.results;
            let mut rank = None;
            for (index, candidate) in candidates.iter().enumerate() {
                let same_path = task
                    .expected_path
                    .as_ref()
                    .map(|p| p == &candidate.symbol.path)
                    .unwrap_or(true);
                let same_symbol = match &task.expected_symbol {
                    Some(expected) => &candidate.symbol.qualified_name == expected,
                    None => true,
                };
                if same_path && same_symbol {
                    rank = Some(index + 1);
                    break;
                }
            }

            // A filename task is answered by the file itself, and the
            // resolver presents file matches before the symbol candidates,
            // so the file position is the position the client sees.
            if task.kind == TaskKind::FilenameDiscovery {
                if let Some(position) = set
                    .files
                    .iter()
                    .position(|f| task.expected_path.as_ref() == Some(&f.path))
                {
                    rank = Some(position + 1);
                }
            }

            let context_tokens = candidates
                .iter()
                .take(3)
                .map(|c| {
                    devzymer_core::util::estimate_tokens(c.symbol.body.as_deref().unwrap_or(""))
                })
                .sum();
            let baseline_tokens = match &task.expected_path {
                Some(path) if benchmark.include_baseline => read_whole(root, path)
                    .map(|(text, _)| devzymer_core::util::estimate_tokens(&text))
                    .unwrap_or(0),
                _ => 0,
            };

            let ground_truth = validate_ground_truth(root, task);

            // The index must reflect the file that is on disk right now.
            let mut fresh = true;
            let mut fresh_reason: Option<String> = None;
            if task.kind == TaskKind::IndexFreshness {
                if let Some(path) = &task.expected_path {
                    let current = current_hash(root, path);
                    let indexed = store
                        .file_by_path(path)
                        .ok()
                        .flatten()
                        .map(|f| f.content_hash);
                    match (current, indexed) {
                        (Some(current), Some(indexed)) if current != indexed => {
                            fresh = false;
                            fresh_reason =
                                Some("o hash do índice difere do arquivo no disco".to_string());
                        }
                        (None, _) => {
                            fresh = false;
                            fresh_reason = Some("o arquivo não pôde ser lido".to_string());
                        }
                        _ => {}
                    }
                }
            }

            // The scenarios that do not look for a specific target have their
            // own success criterion.
            let hit = match task.kind {
                TaskKind::MissingSymbol => rank.is_none(),
                TaskKind::AmbiguousSymbol => set.ambiguity.is_some() && set.results.len() > 1,
                TaskKind::DependencyTraversal => {
                    // The relationship lives in the graph, not in the resolution: the
                    // expected file must import the queried one.
                    match (&task.expected_path, &task.query) {
                        (Some(expected), queried) => graph
                            .dependents(queried)
                            .map(|relations| {
                                relations.iter().any(|r| match &r.src {
                                    devzymer_core::model::Entity::File(f) => f == expected,
                                    _ => false,
                                })
                            })
                            .unwrap_or(false),
                        _ => false,
                    }
                }
                TaskKind::StaleReference => {
                    // The line range must still point at the symbol.
                    let located = candidates.iter().find(|c| {
                        Some(&c.symbol.qualified_name) == task.expected_symbol.as_ref()
                            && Some(&c.symbol.path) == task.expected_path.as_ref()
                    });
                    match (located, task.expected_start_line) {
                        (Some(candidate), Some(line)) => {
                            candidate.symbol.span.start_line == line
                                && candidate.symbol.span.end_line >= line
                        }
                        (Some(_), None) => true,
                        _ => false,
                    }
                }
                _ => rank.map(|r| r <= benchmark.top_k).unwrap_or(false),
            };
            let hit = if task.kind == TaskKind::IndexFreshness {
                fresh
            } else {
                hit
            };
            let mut failure_reason =
                failure_reason(task, &ground_truth, rank, benchmark, &set, hit);
            if let Some(reason) = fresh_reason {
                failure_reason = Some(reason);
            }

            TaskResult {
                task: task.clone(),
                ground_truth,
                found: rank.is_some(),
                hit,
                rank,
                latency_ms,
                context_tokens,
                baseline_tokens,
                token_reduction_pct: crate::report::signed_reduction_pct(
                    context_tokens,
                    baseline_tokens,
                ),
                candidates: candidates.len(),
                candidate_list: candidates
                    .iter()
                    .take(benchmark.candidate_window)
                    .map(|c| CandidateInfo {
                        qualified_name: c.symbol.qualified_name.clone(),
                        path: c.symbol.path.clone(),
                        start_line: c.symbol.span.start_line,
                        end_line: c.symbol.span.end_line,
                        kind: c.symbol.kind.display().to_string(),
                        source: c.source.display().to_string(),
                        confidence: c.confidence.display().to_string(),
                        score: c.score,
                        matched_terms: c.matches,
                        identity_matches: c.identity_matches,
                        evidence: c.evidence,
                        path_hint: c.path_hint,
                    })
                    .collect(),
                cache: if task.kind == TaskKind::RepeatedSearch {
                    warm_cache
                } else {
                    set.cache
                },
                cache_cold_latency_ms,
                cache_warm_latency_ms,
                ambiguity: set.ambiguity.as_ref().map(|a| AmbiguityInfo {
                    name: a.name.clone(),
                    count: a.count,
                    candidates: a.candidates.clone(),
                }),
                failure_reason,
                error: None,
            }
        }
        Err(err) => TaskResult {
            task: task.clone(),
            ground_truth: GroundTruth::NotApplicable,
            found: false,
            hit: false,
            rank: None,
            latency_ms,
            context_tokens: 0,
            baseline_tokens: 0,
            token_reduction_pct: None,
            candidates: 0,
            candidate_list: Vec::new(),
            cache: CacheStatus::Disabled,
            cache_cold_latency_ms: None,
            cache_warm_latency_ms: None,
            ambiguity: None,
            failure_reason: Some(format!("erro de recuperação: {err}")),
            error: Some(err.to_string()),
        },
    }
}

/// The hash of the file as it is on disk right now.
fn current_hash(root: &Path, relative: &str) -> Option<String> {
    let bytes =
        std::fs::read(devzymer_core::util::resolve_inside(root, Path::new(relative)).ok()?).ok()?;
    Some(blake3::hash(&bytes).to_hex()[..32].to_string())
}

/// Confirms the expected answer against the file on disk, so the ground truth
/// does not come from the same index under evaluation.
fn validate_ground_truth(root: &Path, task: &BenchmarkTask) -> GroundTruth {
    let Some(expected_path) = &task.expected_path else {
        return GroundTruth::NotApplicable;
    };
    let Some(text) = source::read_source(root, expected_path) else {
        return GroundTruth::Unverified(format!("arquivo ausente: {expected_path}"));
    };
    let lines: Vec<&str> = text.lines().collect();

    if let Some(expected_symbol) = &task.expected_symbol {
        let identifier = source::identifier_of(expected_symbol);
        if !source::contains_identifier(&text, identifier) {
            return GroundTruth::Unverified(format!(
                "o identificador '{identifier}' não aparece em {expected_path}"
            ));
        }
        if let Some(line) = task.expected_start_line {
            let near = (line.saturating_sub(3)..=line + 3)
                .any(|candidate| source::line_contains(&lines, candidate, identifier));
            if !near {
                return GroundTruth::Unverified(format!(
                    "a linha {line} de {expected_path} não contém '{identifier}'"
                ));
            }
        }
    }
    GroundTruth::Verified
}

/// Explains why a task did not reach the expected answer.
fn failure_reason(
    task: &BenchmarkTask,
    ground_truth: &GroundTruth,
    rank: Option<usize>,
    benchmark: &BenchmarkConfig,
    set: &devzymer_search::resolve::ResolutionSet,
    hit: bool,
) -> Option<String> {
    if let GroundTruth::Unverified(reason) = ground_truth {
        return Some(format!("verdade de base não verificada: {reason}"));
    }
    if hit {
        return None;
    }
    match task.kind {
        TaskKind::MissingSymbol => {
            if rank.is_some() {
                return Some("um símbolo inexistente foi retornado".to_string());
            }
            return None;
        }
        TaskKind::AmbiguousSymbol => {
            let reported = set.ambiguity.is_some();
            let multiple = set.results.len() > 1;
            if reported && multiple {
                return None;
            }
            return Some(format!(
                "a ambiguidade não foi reportada (candidatos={}, reportado={})",
                set.results.len(),
                reported
            ));
        }
        TaskKind::IndexFreshness => {
            // The verdict comes from the hash comparison, not from the
            // retrieval.
            return None;
        }
        TaskKind::StaleReference => {
            if rank.is_some() {
                return None;
            }
            return Some("o conteúdo verificado não foi localizado".to_string());
        }
        _ => {}
    }
    match rank {
        Some(rank) if rank > benchmark.top_k => Some(format!(
            "o símbolo esperado foi recuperado na posição {rank}, fora do top {}",
            benchmark.top_k
        )),
        Some(_rank) => None,
        None => {
            let alternatives: Vec<String> = set
                .results
                .iter()
                .take(5)
                .map(|c| c.symbol.qualified_name.clone())
                .collect();
            Some(format!(
                "o símbolo esperado não foi recuperado; candidatos: {}",
                alternatives.join(", ")
            ))
        }
    }
}

/// Selects the benchmark tasks. The selection is reproducible from the seed.
fn build_tasks(
    store: &Store,
    root: &Path,
    benchmark: &BenchmarkConfig,
) -> Result<Vec<BenchmarkTask>> {
    let symbols = store.all_symbols()?;
    let mut rng = SeededRng::new(benchmark.seed);
    let mut tasks: Vec<BenchmarkTask> = Vec::new();

    let name_counts: HashMap<String, usize> = symbols
        .iter()
        .filter(|s| !matches!(s.kind, devzymer_core::model::SymbolKind::Import))
        .fold(HashMap::new(), |mut acc, s| {
            *acc.entry(s.name.clone()).or_insert(0) += 1;
            acc
        });

    let callable: Vec<&Symbol> = symbols
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                devzymer_core::model::SymbolKind::Function
                    | devzymer_core::model::SymbolKind::Method
                    | devzymer_core::model::SymbolKind::Class
            ) && s.name.len() >= 5
                && s.name.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
        .collect();

    let wanted = |kind: TaskKind| benchmark.kinds.contains(&kind);

    if wanted(TaskKind::ExactSymbol) {
        let unique: Vec<&&Symbol> = callable
            .iter()
            .filter(|s| name_counts.get(&s.name).copied().unwrap_or(0) == 1)
            .collect();
        let picked = sample(&mut rng, &unique, benchmark.tasks_per_kind);
        for (index, symbol) in picked.into_iter().enumerate() {
            tasks.push(BenchmarkTask {
                id: format!("exact-{index}"),
                kind: TaskKind::ExactSymbol,
                query: symbol.name.clone(),
                expected_path: Some(symbol.path.clone()),
                expected_symbol: Some(symbol.qualified_name.clone()),
                expected_start_line: Some(symbol.span.start_line),
                description: format!("definição única de {}", symbol.name),
                origin: QueryOrigin::Identifier,
                ambiguity_count: Some(1),
            });
        }
    }

    if wanted(TaskKind::QualifiedSymbol) {
        let qualified: Vec<&&Symbol> = callable
            .iter()
            .filter(|s| s.qualified_name.contains('.'))
            .collect();
        let picked = sample(&mut rng, &qualified, benchmark.tasks_per_kind);
        for (index, symbol) in picked.into_iter().enumerate() {
            tasks.push(BenchmarkTask {
                id: format!("qualified-{index}"),
                kind: TaskKind::QualifiedSymbol,
                query: symbol.qualified_name.clone(),
                expected_path: Some(symbol.path.clone()),
                expected_symbol: Some(symbol.qualified_name.clone()),
                expected_start_line: Some(symbol.span.start_line),
                description: format!("símbolo qualificado {}", symbol.qualified_name),
                origin: QueryOrigin::Identifier,
                ambiguity_count: None,
            });
        }
    }

    if wanted(TaskKind::FilenameDiscovery) {
        let files = store.all_files()?;
        let mut picked = 0usize;
        for file in files.iter() {
            if picked >= benchmark.tasks_per_kind {
                break;
            }
            let stem = devzymer_core::store::stem_of(&file.path);
            if stem.len() < 4 {
                continue;
            }
            picked += 1;
            tasks.push(BenchmarkTask {
                id: format!("filename-{picked}"),
                kind: TaskKind::FilenameDiscovery,
                query: stem,
                expected_path: Some(file.path.clone()),
                expected_symbol: None,
                expected_start_line: None,
                description: format!("descoberta pelo nome do arquivo {}", file.path),
                origin: QueryOrigin::Identifier,
                ambiguity_count: None,
            });
        }
    }

    if wanted(TaskKind::NaturalLanguage) {
        for (index, symbol) in callable.iter().enumerate() {
            if index >= benchmark.tasks_per_kind * 4 {
                break;
            }
            let Some(text) = source::read_source(root, &symbol.path) else {
                continue;
            };
            let lines: Vec<&str> = text.lines().collect();
            let Some(doc) = source::doc_comment_above(&lines, symbol.span.start_line) else {
                continue;
            };
            let identifier = source::identifier_of(&symbol.qualified_name);
            if source::contains_identifier(&doc, identifier) {
                continue;
            }
            let mut query = source::first_sentence(&doc, 220);
            query = source::redact_identifier(&query, identifier);
            if query.len() < 25 {
                continue;
            }
            tasks.push(BenchmarkTask {
                id: format!("nl-{index}"),
                kind: TaskKind::NaturalLanguage,
                query,
                expected_path: Some(symbol.path.clone()),
                expected_symbol: Some(symbol.qualified_name.clone()),
                expected_start_line: Some(symbol.span.start_line),
                description: "descrição comportamental escrita na documentação do código"
                    .to_string(),
                origin: QueryOrigin::SourceDocumentation,
                ambiguity_count: name_counts.get(&symbol.name).copied(),
            });
        }
        // The cut applies to the natural language block only. Truncating the
        // whole list would hide the scenarios that measure the worst case.
        let natural_language_start = tasks
            .iter()
            .position(|task| task.kind == TaskKind::NaturalLanguage)
            .unwrap_or(tasks.len());
        let keep = benchmark.tasks_per_kind * 2;
        if tasks.len() - natural_language_start > keep {
            tasks.truncate(natural_language_start + keep);
        }
    }

    if wanted(TaskKind::AmbiguousSymbol) {
        let mut ambiguous: HashMap<String, Vec<&Symbol>> = HashMap::new();
        for symbol in &callable {
            ambiguous
                .entry(symbol.name.clone())
                .or_default()
                .push(symbol);
        }
        let mut groups: Vec<(String, Vec<&Symbol>)> = ambiguous
            .into_iter()
            .filter(|(_, group)| group.len() > 1)
            .collect();
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        for (index, (name, group)) in groups.iter().take(benchmark.tasks_per_kind).enumerate() {
            tasks.push(BenchmarkTask {
                id: format!("ambiguous-{index}"),
                kind: TaskKind::AmbiguousSymbol,
                query: name.clone(),
                expected_path: None,
                expected_symbol: None,
                expected_start_line: None,
                description: format!("nome ambíguo com {} definições", group.len()),
                origin: QueryOrigin::Identifier,
                ambiguity_count: Some(group.len()),
            });
        }
    }

    if wanted(TaskKind::DependencyTraversal) {
        let files = store.all_files()?;
        let graph = devzymer_graph::GraphQuery::new(store);
        let mut picked = 0usize;
        for file in files.iter() {
            if picked >= benchmark.tasks_per_kind {
                break;
            }
            let dependents = graph.dependents(&file.path)?;
            let Some(dependent_file) = dependents.iter().find_map(|r| {
                if let devzymer_core::model::Entity::File(f) = &r.src {
                    Some(f.clone())
                } else {
                    None
                }
            }) else {
                continue;
            };
            let Some(dependent_source) = source::read_source(root, &dependent_file) else {
                continue;
            };
            let stem = devzymer_core::store::stem_of(&file.path);
            if !source::contains_identifier(&dependent_source, &stem) {
                continue;
            }
            picked += 1;
            tasks.push(BenchmarkTask {
                id: format!("dependency-{picked}"),
                kind: TaskKind::DependencyTraversal,
                query: file.path.clone(),
                expected_path: Some(dependent_file),
                expected_symbol: None,
                expected_start_line: None,
                description: format!("módulo que importa {}", file.path),
                origin: QueryOrigin::Synthetic,
                ambiguity_count: None,
            });
        }
    }

    if wanted(TaskKind::MissingSymbol) {
        for index in 0..benchmark.tasks_per_kind.min(5) {
            tasks.push(BenchmarkTask {
                id: format!("missing-{index}"),
                kind: TaskKind::MissingSymbol,
                query: format!("simboloInexistenteBenchmark{index}Zzz"),
                expected_path: None,
                expected_symbol: None,
                expected_start_line: None,
                description: "nome que não existe e não deve resolver".to_string(),
                origin: QueryOrigin::Synthetic,
                ambiguity_count: None,
            });
        }
    }

    if wanted(TaskKind::StaleReference) {
        let picked = sample(&mut rng, &callable, benchmark.tasks_per_kind);
        for (index, symbol) in picked.into_iter().enumerate() {
            tasks.push(BenchmarkTask {
                id: format!("stale-{index}"),
                kind: TaskKind::StaleReference,
                query: symbol.qualified_name.clone(),
                expected_path: Some(symbol.path.clone()),
                expected_symbol: Some(symbol.qualified_name.clone()),
                expected_start_line: Some(symbol.span.start_line),
                description: "a faixa de linhas precisa continuar apontando para o símbolo"
                    .to_string(),
                origin: QueryOrigin::Identifier,
                ambiguity_count: None,
            });
        }
    }

    if wanted(TaskKind::IndexFreshness) {
        let files = store.all_files()?;
        for (index, file) in files
            .iter()
            .step_by(7)
            .take(benchmark.tasks_per_kind)
            .enumerate()
        {
            tasks.push(BenchmarkTask {
                id: format!("freshness-{index}"),
                kind: TaskKind::IndexFreshness,
                query: file.path.clone(),
                expected_path: Some(file.path.clone()),
                expected_symbol: None,
                expected_start_line: None,
                description: "o índice precisa refletir o arquivo no disco".to_string(),
                origin: QueryOrigin::Synthetic,
                ambiguity_count: None,
            });
        }
    }

    if wanted(TaskKind::RepeatedSearch) {
        if let Some(first) = tasks
            .iter()
            .find(|t| t.kind == TaskKind::ExactSymbol)
            .cloned()
        {
            tasks.push(BenchmarkTask {
                id: "repeated-0".into(),
                kind: TaskKind::RepeatedSearch,
                query: first.query.clone(),
                expected_path: first.expected_path.clone(),
                expected_symbol: first.expected_symbol.clone(),
                expected_start_line: first.expected_start_line,
                description: "mesma consulta, para observar o cache de recuperação".to_string(),
                origin: first.origin,
                ambiguity_count: first.ambiguity_count,
            });
        }
    }

    if benchmark.curated {
        for (index, task) in curated_tasks(root, store).into_iter().enumerate() {
            tasks.push(BenchmarkTask {
                id: format!("curated-{index}"),
                kind: TaskKind::NaturalLanguage,
                query: task.query.to_string(),
                expected_path: Some(task.path.to_string()),
                expected_symbol: Some(task.symbol.to_string()),
                expected_start_line: None,
                description: task.description.to_string(),
                origin: QueryOrigin::Curated,
                ambiguity_count: None,
            });
        }
    }

    // The seed makes the selection reproducible. The rotation keeps the order
    // stable, while the sampling above gives every symbol the same chance.
    if benchmark.seed != 0 && !tasks.is_empty() {
        let rotation = rng.below(tasks.len());
        tasks.rotate_left(rotation);
    }

    Ok(tasks)
}

/// Selects `count` elements from a list, reproducibly for a given seed.
///
/// Taking the first elements of a sorted list always selects the same
/// symbols, whatever the seed says, so the selection is a real sample.
fn sample<'a, T>(rng: &mut SeededRng, items: &'a [T], count: usize) -> Vec<&'a T> {
    let mut pool: Vec<usize> = (0..items.len()).collect();
    for i in (1..pool.len()).rev() {
        let j = rng.below(i + 1);
        pool.swap(i, j);
    }
    pool.into_iter()
        .take(count)
        .filter_map(|index| items.get(index))
        .collect()
}

/// The curated queries that match this repository.
fn curated_tasks(root: &Path, store: &Store) -> Vec<CuratedTask> {
    let owned = store.all_files().unwrap_or_default();
    let files: HashSet<&str> = owned.iter().map(|f| f.path.as_str()).collect();
    let corpus = match crate::tasks::load_curated(root) {
        Ok(tasks) => tasks,
        Err(error) => {
            eprintln!("[aviso] corpus curado ignorado: {error}");
            Vec::new()
        }
    };
    corpus
        .into_iter()
        .filter(|task| files.contains(task.path.as_str()))
        .filter(|task| source::read_source(root, &task.path).is_some())
        .collect()
}

/// The identity of the repository under evaluation, without leaking absolute
/// paths beyond what the operator already knows.
pub fn repository_identity(root: &Path, store: &Store) -> crate::report::RepositoryIdentity {
    let stats = store.stats().unwrap_or_default();
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());
    crate::report::RepositoryIdentity {
        name,
        files: stats.files,
        symbols: stats.symbols,
        relations: stats.relations,
        index_revision: stats.revision,
        parser_version: stats.parser_version,
        schema_version: stats.schema_version,
    }
}

/// The identifier of the token estimator used by the benchmark.
pub const TOKEN_ESTIMATOR: TokenEstimator = TokenEstimator::HeuristicChars;
