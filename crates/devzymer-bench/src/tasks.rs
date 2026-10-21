use std::path::Path;

use serde::Serialize;

/// The independent scenarios evaluated by the benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// A bare symbol name that is unique in the repository.
    ExactSymbol,
    /// A qualified name such as `Container.member`.
    QualifiedSymbol,
    /// The name of a file, without its path.
    FilenameDiscovery,
    /// A behavioural description written in prose.
    NaturalLanguage,
    /// A bare name shared by several definitions.
    AmbiguousSymbol,
    /// A file and the modules that import it.
    DependencyTraversal,
    /// The same query issued twice, to observe the retrieval cache.
    RepeatedSearch,
    /// A name that does not exist and must not resolve.
    MissingSymbol,
    /// A line range that must still match the source file.
    StaleReference,
    /// The index content compared against the files on disk.
    IndexFreshness,
}

impl TaskKind {
    pub fn display(self) -> &'static str {
        match self {
            TaskKind::ExactSymbol => "exact_symbol",
            TaskKind::QualifiedSymbol => "qualified_symbol",
            TaskKind::FilenameDiscovery => "filename_discovery",
            TaskKind::NaturalLanguage => "natural_language",
            TaskKind::AmbiguousSymbol => "ambiguous_symbol",
            TaskKind::DependencyTraversal => "dependency_traversal",
            TaskKind::RepeatedSearch => "repeated_search",
            TaskKind::MissingSymbol => "missing_symbol",
            TaskKind::StaleReference => "stale_reference",
            TaskKind::IndexFreshness => "index_freshness",
        }
    }

    pub fn all() -> &'static [TaskKind] {
        &[
            TaskKind::ExactSymbol,
            TaskKind::QualifiedSymbol,
            TaskKind::FilenameDiscovery,
            TaskKind::NaturalLanguage,
            TaskKind::AmbiguousSymbol,
            TaskKind::DependencyTraversal,
            TaskKind::RepeatedSearch,
            TaskKind::MissingSymbol,
            TaskKind::StaleReference,
            TaskKind::IndexFreshness,
        ]
    }
}

/// How the query of a task was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOrigin {
    /// Derived from the symbol identifier itself.
    Identifier,
    /// Derived from the documentation written in the source file.
    SourceDocumentation,
    /// Written by hand for the evaluation.
    Curated,
    /// Constructed by the scenario itself.
    Synthetic,
}

impl QueryOrigin {
    pub fn display(self) -> &'static str {
        match self {
            QueryOrigin::Identifier => "identifier",
            QueryOrigin::SourceDocumentation => "source_documentation",
            QueryOrigin::Curated => "curated",
            QueryOrigin::Synthetic => "synthetic",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkTask {
    pub id: String,
    pub kind: TaskKind,
    pub query: String,
    pub expected_path: Option<String>,
    pub expected_symbol: Option<String>,
    pub expected_start_line: Option<u32>,
    pub description: String,
    pub origin: QueryOrigin,
    /// For the ambiguous scenario, the number of definitions the bare name has.
    pub ambiguity_count: Option<usize>,
}

/// A hand written query, independent from the index, with the ground truth
/// declared by path and symbol. The runner validates it against the source.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CuratedTask {
    pub query: String,
    pub path: String,
    pub symbol: String,
    pub description: String,
}

/// The curated queries of a workspace, loaded from `.devzymer/curated.json`.
///
/// The corpus lives with the repository it describes, never inside the tool:
/// a published benchmark carries no path and no symbol of any project. A
/// workspace without the file simply has no curated scenario.
pub fn load_curated(root: &Path) -> Result<Vec<CuratedTask>, String> {
    let path = root.join(".devzymer").join("curated.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("nao foi possivel ler {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{} invalido: {error}", path.display()))
}

/// Deterministic pseudo random generator, so a task selection can be
/// reproduced exactly from a seed.
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.max(1) ^ 0x9E3779B97F4A7C15,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}
