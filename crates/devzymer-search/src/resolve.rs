use serde::{Deserialize, Serialize};

use devzymer_core::model::{Confidence, Language, Symbol};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionSource {
    ExactPath,
    FileMatch,
    /// A file whose name appears in the query, but only as a hint.
    FileHint,
    ExactSymbol,
    QualifiedSymbol,
    TokenMatch,
    Prefix,
    Lexical,
    GraphExpansion,
}

impl ResolutionSource {
    pub fn base_score(self) -> f64 {
        match self {
            ResolutionSource::ExactPath => 1.0,
            ResolutionSource::FileMatch => 0.93,
            ResolutionSource::FileHint => 0.6,
            ResolutionSource::ExactSymbol => 0.95,
            ResolutionSource::QualifiedSymbol => 0.9,
            ResolutionSource::TokenMatch => 0.82,
            ResolutionSource::Prefix => 0.68,
            ResolutionSource::Lexical => 0.5,
            ResolutionSource::GraphExpansion => 0.45,
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            ResolutionSource::ExactPath => "exact_path",
            ResolutionSource::FileMatch => "file_match",
            ResolutionSource::FileHint => "file_hint",
            ResolutionSource::ExactSymbol => "exact_symbol",
            ResolutionSource::QualifiedSymbol => "qualified_symbol",
            ResolutionSource::TokenMatch => "token_match",
            ResolutionSource::Prefix => "prefix",
            ResolutionSource::Lexical => "lexical",
            ResolutionSource::GraphExpansion => "graph",
        }
    }
}

/// A file matched by name, when the query names a file instead of a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMatch {
    pub path: String,
    pub symbols: usize,
    pub score: f64,
    pub reason: String,
    pub stale: bool,
}

/// Reports that a bare name resolves to more than one definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ambiguity {
    pub name: String,
    pub count: usize,
    pub candidates: Vec<String>,
}

/// Whether the retrieval was served from the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    Hit,
    #[default]
    Miss,
    Disabled,
}

impl CacheStatus {
    pub fn display(self) -> &'static str {
        match self {
            CacheStatus::Hit => "hit",
            CacheStatus::Miss => "miss",
            CacheStatus::Disabled => "disabled",
        }
    }
}

/// The version of the retrieval algorithm. It is part of the cache key, so a
/// change in the algorithm never serves a result produced by the previous one.
pub const RESOLVER_VERSION: &str = "devzymer-resolver-22";

/// The complete outcome of a retrieval.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolutionSet {
    pub results: Vec<Resolution>,
    pub files: Vec<FileMatch>,
    pub ambiguity: Option<Ambiguity>,
    pub cache: CacheStatus,
    /// Why each candidate matched, collected only when the caller asks for
    /// the diagnostics.
    pub evidence: Vec<CandidateEvidence>,
}

/// The terms a candidate covers, and where they were found.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub qualified_name: String,
    pub terms: Vec<EvidenceDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceDetail {
    /// The term of the question.
    pub term: String,
    /// The form of the term that was found in the repository.
    pub form: String,
    /// Where it was found: the name, the documentation or the body.
    pub field: String,
    pub weight: f64,
}

/// The payload stored in the retrieval cache. It keeps the ranking metadata and
/// the symbol identifiers, never the symbol bodies, so a cache entry stays
/// small and a hit does not have to read the sources again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSet {
    pub entries: Vec<CachedEntry>,
    pub files: Vec<FileMatch>,
    pub ambiguity: Option<Ambiguity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEntry {
    pub id: String,
    pub source: ResolutionSource,
    pub score: f64,
    pub confidence: Confidence,
    pub note: Option<String>,
    pub matches: usize,
    /// Query terms corroborated by the symbol identity (its name and
    /// qualified name), counted separately from body-word hits.
    #[serde(default)]
    pub identity_matches: usize,
    /// Ordered name-segment subsequence that follows the query token order.
    #[serde(default)]
    pub sequence_cover: usize,
    /// Weighted evidence the candidate carries for the query: rare terms
    /// weigh more, and a term found in the name weighs more than one found
    /// in the body.
    #[serde(default)]
    pub evidence: f64,
    pub path_hint: bool,
    /// Strength of the file name evidence, from 0 to 1.
    #[serde(default)]
    pub hint_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    pub symbol: Symbol,
    pub source: ResolutionSource,
    pub score: f64,
    pub confidence: Confidence,
    pub note: Option<String>,
    /// Number of query terms matched by the symbol name.
    pub matches: usize,
    /// Query terms corroborated by the symbol identity (its name and
    /// qualified name), counted separately from body-word hits.
    pub identity_matches: usize,
    /// Ordered name-segment subsequence that follows the query token order.
    pub sequence_cover: usize,
    /// Weighted evidence the candidate carries for the query.
    pub evidence: f64,
    /// True when the candidate was boosted by a file name present in the query.
    pub path_hint: bool,
    /// Strength of the file name evidence, from 0 to 1. It is the primary
    /// ranking key, because a file named in the query is stronger evidence
    /// than a word that happens to appear in a symbol.
    pub hint_score: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ResolveQuery {
    pub query: String,
    pub path: Option<String>,
    pub language: Option<Language>,
    pub limit: usize,
    /// Upper bound of candidates collected before ranking.
    pub candidate_limit: usize,
    /// Collect why each candidate matched. The details are bounded and cost
    /// a little more work, so they are produced only when asked.
    pub diagnostics: bool,
}

impl ResolveQuery {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            limit: 20,
            candidate_limit: 200,
            ..Default::default()
        }
    }
}

const STOP_WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "from",
    "that",
    "this",
    "function",
    "method",
    "class",
    "where",
    "find",
    "search",
    "code",
    "file",
    "responsible",
    "used",
    "using",
    "does",
    "handle",
    "returns",
    // Common Portuguese function words of behavioral sentences. They carry
    // no identity evidence and register hits against many files.
    "como",
    "para",
    "com",
    "sem",
    "sobre",
    "entre",
    "depois",
    "antes",
    "quando",
    "onde",
    "qual",
    "quem",
    "mais",
    "mas",
    "todas",
    "todos",
    "tudo",
    "uma",
    "ate",
    // Words that talk about the code itself instead of about a subject.
    // A question that names them does not name what it is looking for.
    "funcao",
    "funcoes",
    "metodo",
    "metodos",
    "classe",
    "classes",
    "codigo",
    "arquivo",
    "arquivos",
    "simbolo",
    "simbolos",
    "variavel",
    "variaveis",
    "parametro",
    "parametros",
    "execucao",
    "implementacao",
    "trecho",
    // Negations and filler of a question about something that does not
    // exist.
    "nao",
    "nenhum",
    "nenhuma",
    "nada",
    "existe",
    "inexistente",
    "inexistentes",
    "lugar",
    "deste",
    "desta",
    "disso",
    "aqui",
    // Function words that carry no subject of their own.
    "que",
    "nos",
    "nas",
    "apos",
    "desde",
    "seus",
    "suas",
    "pela",
    "pelas",
    "pelo",
    "pelos",
    "desse",
    "desta",
    "nesta",
    "neste",
    "nessa",
    "nesse",
    "ser",
    "sao",
    "foi",
    "era",
    "muito",
    "pouco",
    "ainda",
    "entao",
    "assim",
    "cada",
    "todo",
    "toda",
    "outro",
    "outra",
    "mesmo",
    "mesma",
    "grande",
    "maior",
    "menor",
    "novo",
    "nova",
    "antigo",
    "antiga",
    "proximo",
    "proxima",
    "anterior",
    "seguinte",
    "segundo",
    "durante",
    "atraves",
    "tanto",
    "tao",
    "tal",
    "bem",
    "ali",
    "talvez",
    "apenas",
    "somente",
    "unico",
    "unica",
    "alguem",
    "algo",
    "ter",
    "esta",
    "este",
];

/// Splits a free-form query into candidate identifier tokens.
pub fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for raw in query.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '$') {
        let t = raw.trim();
        if t.len() < 3 {
            continue;
        }
        if STOP_WORDS.contains(&t.to_ascii_lowercase().as_str()) {
            continue;
        }
        if !tokens.iter().any(|e: &String| e.eq_ignore_ascii_case(t)) {
            tokens.push(t.to_string());
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_drop_stop_words_and_short_terms() {
        let tokens = query_tokens("find the function responsible for validating sessions");
        assert!(tokens.contains(&"validating".to_string()));
        assert!(tokens.contains(&"sessions".to_string()));
        assert!(!tokens.contains(&"the".to_string()));
        assert!(!tokens.contains(&"function".to_string()));
    }

    #[test]
    fn tokens_keep_file_names_and_extensions() {
        let tokens = query_tokens("limites.ts objeto");
        assert!(tokens.contains(&"limites".to_string()));
        assert!(tokens.contains(&"objeto".to_string()));
    }
}
