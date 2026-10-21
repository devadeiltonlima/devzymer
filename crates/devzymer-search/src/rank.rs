use crate::resolve::{Resolution, ResolutionSource};

/// Combines the retrieval cascade score with small structural signals so that
/// a symbol defined in a file the query mentions outranks an incidental
/// lexical match.
pub struct Ranker {
    pub lexical_weight: f64,
}

impl Default for Ranker {
    fn default() -> Self {
        Self {
            lexical_weight: ResolutionSource::Lexical.base_score(),
        }
    }
}

impl Ranker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps a raw BM25 score (larger is better) into a comparable range.
    pub fn normalize_bm25(&self, score: f64) -> f64 {
        if score <= 0.0 {
            return 0.0;
        }
        (score / (score + 4.0)).min(1.0) * self.lexical_weight
    }

    /// The strength of the evidence that produced a candidate.
    ///
    /// A query that names a symbol exactly is stronger evidence than a term
    /// of the query that happens to match a file name, which in turn is
    /// stronger than an incidental lexical match.
    pub fn tier(source: ResolutionSource) -> u8 {
        match source {
            ResolutionSource::ExactPath
            | ResolutionSource::ExactSymbol
            | ResolutionSource::QualifiedSymbol => 3,
            ResolutionSource::FileMatch => 2,
            ResolutionSource::TokenMatch
            | ResolutionSource::Prefix
            | ResolutionSource::Lexical
            | ResolutionSource::FileHint => 1,
            ResolutionSource::GraphExpansion => 0,
        }
    }

    pub fn sort(&self, items: &mut [Resolution]) {
        items.sort_by(|a, b| {
            Self::tier(b.source)
                .cmp(&Self::tier(a.source))
                .then_with(|| {
                    b.hint_score
                        .partial_cmp(&a.hint_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                // Evidence weighs the rarity of the terms a candidate
                // covers: a symbol that answers a rare concept outranks a
                // component that only repeats common words.
                .then_with(|| {
                    b.evidence
                        .partial_cmp(&a.evidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                // Total term coverage decides first, as before. Identity
                // matches (the symbol name itself) then break the ties in
                // favor of the definition rather than body words. One identity
                // term never buries a candidate that matches more terms.
                .then_with(|| b.matches.cmp(&a.matches))
                .then_with(|| b.identity_matches.cmp(&a.identity_matches))
                // Among equal evidence, a name that reads in the same order
                // as the phrase outranks one that carries the same parts.
                .then_with(|| b.sequence_cover.cmp(&a.sequence_cover))
                .then_with(|| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.symbol.path.cmp(&b.symbol.path))
                .then_with(|| a.symbol.span.start_line.cmp(&b.symbol.span.start_line))
        });
    }
}
