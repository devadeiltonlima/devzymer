use std::collections::HashMap;

use devzymer_core::config::Config;
use devzymer_core::model::{Confidence, Language, Symbol};
use devzymer_core::store::{FileRecord, Store};
use devzymer_core::Result;
use serde_json::json;

use crate::rank::Ranker;
use crate::resolve::{
    query_tokens, Ambiguity, CacheStatus, CachedEntry, CachedSet, CandidateEvidence,
    EvidenceDetail, FileMatch, Resolution, ResolutionSet, ResolutionSource, ResolveQuery,
};

/// A file that matched the query by name.
struct MatchedFile {
    file: FileRecord,
    score: f64,
    reason: String,
    /// True when the file was named by a term of the query rather than by the
    /// whole query, which makes it a path hint for the candidates.
    hint: bool,
    /// Length of the term that named the file. A longer name is a
    /// more specific hint.
    hint_weight: usize,
}

pub struct Resolver<'a> {
    store: &'a Store,
    config: &'a Config,
    ranker: Ranker,
}

impl<'a> Resolver<'a> {
    pub fn new(store: &'a Store, config: &'a Config) -> Self {
        Self {
            store,
            config,
            ranker: Ranker::new(),
        }
    }

    /// Cascading retrieval: exact path, file name, exact symbol, qualified
    /// symbol, token match, prefix, lexical search, then graph expansion.
    ///
    /// The result is cached per index revision. The cache removes repeated
    /// database work, never information: the same context is returned.
    pub fn resolve(&self, query: &ResolveQuery) -> Result<ResolutionSet> {
        self.store
            .ensure_resolver_version(crate::resolve::RESOLVER_VERSION)?;
        let revision = self.store.revision()?;
        let key = self.cache_key(query, revision);
        if self.config.context.cache_enabled {
            if let Some(payload) = self.store.cache_get(&key, revision)? {
                if let Ok(cached) = serde_json::from_str::<CachedSet>(&payload) {
                    if let Some(set) = self.rehydrate(&cached) {
                        let mut set = set;
                        set.cache = CacheStatus::Hit;
                        return Ok(set);
                    }
                }
            }
        }

        let set = self.resolve_uncached(query)?;
        if self.config.context.cache_enabled {
            let cached = CachedSet {
                entries: set
                    .results
                    .iter()
                    .map(|r| CachedEntry {
                        id: r.symbol.id.0.clone(),
                        source: r.source,
                        score: r.score,
                        confidence: r.confidence,
                        note: r.note.clone(),
                        matches: r.matches,
                        identity_matches: r.identity_matches,
                        sequence_cover: r.sequence_cover,
                        evidence: r.evidence,
                        path_hint: r.path_hint,
                        hint_score: r.hint_score,
                    })
                    .collect(),
                files: set.files.clone(),
                ambiguity: set.ambiguity.clone(),
            };
            if let Ok(payload) = serde_json::to_string(&cached) {
                let _ = self.store.cache_put(&key, revision, &payload);
            }
        }
        Ok(set)
    }

    /// Rebuilds a cached result, reading the symbols back from the index.
    fn rehydrate(&self, cached: &CachedSet) -> Option<ResolutionSet> {
        let mut results = Vec::with_capacity(cached.entries.len());
        for entry in &cached.entries {
            let symbol = self.store.symbol_by_id(&entry.id).ok()??;
            results.push(Resolution {
                symbol,
                source: entry.source,
                score: entry.score,
                confidence: entry.confidence,
                note: entry.note.clone(),
                matches: entry.matches,
                identity_matches: entry.identity_matches,
                sequence_cover: entry.sequence_cover,
                evidence: entry.evidence,
                path_hint: entry.path_hint,
                hint_score: entry.hint_score,
            });
        }
        Some(ResolutionSet {
            results,
            files: cached.files.clone(),
            ambiguity: cached.ambiguity.clone(),
            cache: CacheStatus::Hit,
            evidence: Vec::new(),
        })
    }

    fn cache_key(&self, query: &ResolveQuery, revision: u64) -> String {
        let payload = json!({
            "v": crate::resolve::RESOLVER_VERSION,
            "q": query.query,
            "p": query.path,
            "l": query.language.map(|l| l.display()),
            "n": query.limit,
            "d": query.diagnostics,
            "r": revision,
        });
        blake3::hash(payload.to_string().as_bytes()).to_hex()[..32].to_string()
    }

    /// Terms the repository itself uses for the concept, harvested from the
    /// names of the candidates the first pass returned. The harvest follows
    /// the vocabulary of the code, not the words of the question, and only
    /// rare names are kept, so the expansion stays on the subject.
    ///
    /// The harvest is anchored: only a candidate that already covers a rare
    /// term of the question contributes names. A candidate that matched only
    /// common words says nothing about the subject of the question, and
    /// following it would lead the search away.
    fn expansion_terms(
        &self,
        seeds: &[Resolution],
        known: &[String],
        anchor: f64,
    ) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for resolution in seeds.iter().take(EXPANSION_SEEDS) {
            if resolution.evidence < anchor {
                continue;
            }
            let mut words: Vec<String> =
                devzymer_core::util::split_identifier(&resolution.symbol.qualified_name)
                    .split(' ')
                    .map(devzymer_core::util::diacritic_fold)
                    .filter(|word| word.len() >= 4)
                    .collect();
            if let Some(doc) = resolution.symbol.doc.as_deref() {
                words.extend(
                    doc.split(|c: char| !c.is_alphanumeric())
                        .map(|word| devzymer_core::util::diacritic_fold(&word.to_lowercase()))
                        .filter(|word| word.len() >= 4),
                );
            }
            for word in words {
                if known.iter().any(|term| term == &word) {
                    continue;
                }
                let entry = counts.entry(word).or_insert(0);
                *entry += 1;
            }
        }
        let mut candidates: Vec<(String, usize)> = counts
            .into_iter()
            .filter(|(_, count)| *count >= 2)
            .collect();
        candidates.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        candidates.truncate(EXPANSION_CANDIDATES);
        if candidates.is_empty() {
            return Vec::new();
        }
        let terms: Vec<String> = candidates.iter().map(|(term, _)| term.clone()).collect();
        let frequencies = self
            .store
            .term_document_frequencies(&terms)
            .unwrap_or_default();
        let total = self.store.symbol_count().unwrap_or(1).max(1) as usize;
        let mut kept: Vec<(String, usize)> = candidates
            .into_iter()
            .filter_map(|(term, _)| {
                let frequency = frequencies.get(&term).copied().unwrap_or(0);
                if frequency == 0 || frequency * EXPANSION_RARITY > total {
                    return None;
                }
                Some((term, frequency))
            })
            .collect();
        kept.sort_by_key(|(_, frequency)| std::cmp::Reverse(*frequency));
        kept.truncate(EXPANSION_TERMS);
        kept
    }

    /// The files a path filter covers: the file with that exact path, or
    /// every file under it when the path names a directory. A path that
    /// matches nothing covers nothing.
    fn path_scope(&self, path: &str) -> Result<Vec<devzymer_core::store::FileRecord>> {
        let all = self.store.all_files()?;
        if let Some(file) = all.iter().find(|file| file.path == path) {
            return Ok(vec![file.clone()]);
        }
        let prefix = format!("{path}/");
        Ok(all
            .into_iter()
            .filter(|file| file.path.starts_with(&prefix))
            .collect())
    }

    /// True when the candidate belongs to the path filter.
    fn within_path_scope(path: Option<&str>, candidate_path: &str) -> bool {
        let Some(path) = path else {
            return true;
        };
        candidate_path == path || candidate_path.starts_with(&format!("{path}/"))
    }

    /// Normalizes a repository relative path so that a Windows separator and
    /// a POSIX separator describe the same file.
    pub fn normalize_repository_path(path: &str) -> String {
        let unified = path.replace('\\', "/");
        let trimmed = unified.trim_start_matches("./").trim();
        trimmed.to_string()
    }

    /// True when the path points inside the workspace. A path that escapes
    /// the repository is never accepted as a filter.
    pub fn path_is_inside_workspace(path: &str) -> bool {
        let normalized = Self::normalize_repository_path(path);
        if normalized.is_empty() {
            return false;
        }
        if normalized.starts_with('/') {
            return false;
        }
        let mut depth = 0i32;
        for segment in normalized.split('/') {
            match segment {
                "" | "." => continue,
                ".." => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => depth += 1,
            }
        }
        true
    }

    /// Grounds the query terms in the repository: each term keeps the forms
    /// that really exist in the index, with how many symbols contain them.
    /// A form that exists in few symbols carries more meaning than one that
    /// exists in every file.
    fn ground_tokens(&self, tokens: &[String]) -> Vec<GroundedToken> {
        let mut per_token: Vec<Vec<String>> = Vec::new();
        let mut all_variants: Vec<String> = Vec::new();
        for token in tokens {
            let variants = term_variants(token);
            for variant in &variants {
                if !all_variants.contains(variant) {
                    all_variants.push(variant.clone());
                }
            }
            per_token.push(variants);
        }
        let frequencies = self
            .store
            .term_document_frequencies(&all_variants)
            .unwrap_or_default();
        per_token
            .into_iter()
            .zip(tokens.iter())
            .map(|(variants, original)| {
                let mut grounded: Vec<(String, usize)> = variants
                    .iter()
                    .map(|variant| {
                        let frequency = frequencies.get(variant).copied().unwrap_or(0);
                        (variant.clone(), frequency)
                    })
                    .collect();
                grounded.sort_by_key(|(_, frequency)| std::cmp::Reverse(*frequency));
                GroundedToken {
                    term: original.clone(),
                    variants: grounded,
                    weight: 1.0,
                }
            })
            .collect()
    }

    /// Runs the cascade without consulting the cache.
    pub fn resolve_uncached(&self, query: &ResolveQuery) -> Result<ResolutionSet> {
        let limit = if query.limit > 0 {
            query.limit
        } else {
            self.config.search.max_results
        };
        let candidate_limit = query.candidate_limit.max(limit).max(50);
        let mut collected: Vec<Resolution> = Vec::new();
        let mut files: Vec<FileMatch> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();

        let push = |candidates: Vec<Resolution>,
                    out: &mut Vec<Resolution>,
                    seen: &mut HashMap<String, usize>| {
            for candidate in candidates {
                let key = candidate.symbol.id.0.clone();
                match seen.get(&key) {
                    Some(idx) => {
                        if candidate.score > out[*idx].score {
                            out[*idx] = candidate;
                        }
                    }
                    None => {
                        seen.insert(key, out.len());
                        out.push(candidate);
                    }
                }
            }
        };

        // The path filter restricts the search to one file or to one
        // directory. It is a filter, not a hint: a candidate outside of it
        // never reaches the result, and a file that does not exist yields
        // nothing at all.
        let path_filter = query
            .path
            .as_ref()
            .map(|path| Self::normalize_repository_path(path))
            .filter(|path| !path.is_empty());

        if let Some(path) = path_filter.as_ref() {
            let scope = self.path_scope(path)?;
            for file in scope {
                let symbols = self.store.symbols_for_file(file.id)?;
                let candidates = symbols
                    .into_iter()
                    .map(|symbol| {
                        self.to_resolution(
                            symbol,
                            ResolutionSource::ExactPath,
                            ResolutionSource::ExactPath.base_score(),
                            None,
                        )
                    })
                    .collect();
                push(candidates, &mut collected, &mut seen);
            }
        }

        let trimmed = query.query.trim();
        let path_like = looks_like_path(trimmed);
        let tokens = query_tokens(trimmed);
        let grounded = if path_like {
            Vec::new()
        } else {
            self.ground_tokens(&tokens)
        };
        let mut hints: Vec<(String, usize)> = Vec::new();

        if !trimmed.is_empty() {
            if let Some(file) = self.store.file_by_path(trimmed)? {
                let symbols = self.store.symbols_for_file(file.id)?;
                let candidates = symbols
                    .into_iter()
                    .map(|symbol| {
                        self.to_resolution(
                            symbol,
                            ResolutionSource::ExactPath,
                            ResolutionSource::ExactPath.base_score(),
                            None,
                        )
                    })
                    .collect();
                push(candidates, &mut collected, &mut seen);
            }

            // A query frequently names a file instead of a symbol, for example
            // "useVoiceCall" for "src/hooks/useVoiceCall.ts".
            //
            // The name hint only applies when the query carries an identifier.
            // In a sentence every word could be the name of a file, and the
            // hint would bury the real answer under the members of an
            // unrelated file.
            let identifier_query = trimmed.contains('.')
                || query_tokens(trimmed)
                    .iter()
                    .any(|token| is_identifier_like(token));
            for matched in self.match_files(trimmed)? {
                if matched.hint && identifier_query {
                    hints.push((matched.file.path.clone(), matched.hint_weight));
                }
                let source = if matched.hint {
                    ResolutionSource::FileHint
                } else {
                    ResolutionSource::FileMatch
                };
                let candidates = self
                    .store
                    .symbols_for_file(matched.file.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|symbol| {
                        !matches!(
                            symbol.kind,
                            devzymer_core::model::SymbolKind::Import
                                | devzymer_core::model::SymbolKind::Export
                        )
                    })
                    .take(MAX_FILE_CANDIDATES)
                    .map(|symbol| {
                        self.to_resolution(
                            symbol,
                            source,
                            source.base_score(),
                            Some(matched.reason.clone()),
                        )
                    })
                    .collect();
                push(candidates, &mut collected, &mut seen);
                if Self::within_path_scope(path_filter.as_deref(), &matched.file.path) {
                    files.push(FileMatch {
                        path: matched.file.path.clone(),
                        symbols: matched.file.symbol_count,
                        score: matched.score,
                        reason: matched.reason,
                        stale: false,
                    });
                }
            }

            let exact = self.store.symbols_by_name(trimmed)?;
            let candidates = exact
                .into_iter()
                .map(|symbol| {
                    self.to_resolution(
                        symbol,
                        ResolutionSource::ExactSymbol,
                        ResolutionSource::ExactSymbol.base_score(),
                        None,
                    )
                })
                .collect();
            push(candidates, &mut collected, &mut seen);

            if trimmed.contains('.') {
                let qualified = self.store.symbols_by_qualified_name(trimmed)?;
                let candidates = qualified
                    .into_iter()
                    .map(|symbol| {
                        self.to_resolution(
                            symbol,
                            ResolutionSource::QualifiedSymbol,
                            ResolutionSource::QualifiedSymbol.base_score(),
                            None,
                        )
                    })
                    .collect();
                push(candidates, &mut collected, &mut seen);
            }

            for token in query_tokens(trimmed) {
                if path_like {
                    break;
                }
                // The sentence brings the inflected or accented word and the
                // code holds the plain identifier (`limitador` vs `limitar`,
                // `transferência` vs `transferenciaChamada`), so the lookups
                // run on the accented folded form of the token.
                let token = devzymer_core::util::diacritic_fold(&token);
                let exact = self.store.symbols_by_name(&token)?;
                if !exact.is_empty() {
                    let candidates = exact
                        .into_iter()
                        .map(|symbol| {
                            self.to_resolution(
                                symbol,
                                ResolutionSource::TokenMatch,
                                ResolutionSource::TokenMatch.base_score(),
                                Some(format!("token:{token}")),
                            )
                        })
                        .collect();
                    push(candidates, &mut collected, &mut seen);
                }
                // The prefix matches are kept even when an exact match
                // exists: a generic token such as "update" must not hide the
                // symbol named `update_json_file`.
                let prefixed = self.store.symbols_with_prefix(&token)?;
                // A word of a sentence matches a lot of symbols and buries
                // the real answer. The prefix is only used when the token
                // looks like an identifier, or when it matches few symbols.
                let identifier_like = is_identifier_like(&token);
                if identifier_like || prefixed.len() <= 3 {
                    // When the whole word answered nothing, its leading form
                    // may still name the definition: the sentence carries the
                    // inflected word (`limitador`) and the code names the
                    // infinitive (`limitar`). The bound keeps a vague stem
                    // from flooding the candidates.
                    let mut matches = prefixed;
                    let mut reason = format!("prefix:{token}");
                    if matches.is_empty() {
                        if let Some(stem) = stem_prefix(&token) {
                            let stemmed = self.store.symbols_with_prefix(&stem)?;
                            if stemmed.len() <= 6 {
                                reason = format!("stem:{stem}");
                                matches = stemmed;
                            }
                        }
                    }
                    if matches.is_empty() && token.chars().count() >= 5 {
                        // The word may only appear inside a larger camel case
                        // name. The scan is the last resort and is kept small.
                        matches = self.store.symbols_by_name_contains(&token, 12)?;
                        if !matches.is_empty() {
                            reason = format!("contains:{token}");
                        }
                    }
                    let candidates = matches
                        .into_iter()
                        .map(|symbol| {
                            self.to_resolution(
                                symbol,
                                ResolutionSource::Prefix,
                                ResolutionSource::Prefix.base_score(),
                                Some(reason.clone()),
                            )
                        })
                        .collect();
                    push(candidates, &mut collected, &mut seen);
                }
            }

            // The lexical stage searches the forms of the query that really
            // exist in the repository. Identity stages never expand. A path
            // query has no terms of its own, so it searches the words of
            // the path.
            let mut lexical_terms: Vec<String> = Vec::new();
            if grounded.is_empty() {
                lexical_terms = query_tokens(trimmed);
            } else {
                for term in &grounded {
                    for (variant, _) in &term.variants {
                        if !lexical_terms.contains(variant) {
                            lexical_terms.push(variant.clone());
                        }
                    }
                }
            }
            let lexical = self.store.fts_search_terms(
                &lexical_terms,
                candidate_limit,
                [
                    self.config.search.bm25_name_weight,
                    self.config.search.bm25_qualified_weight,
                    self.config.search.bm25_signature_weight,
                    self.config.search.bm25_doc_weight,
                    1.0,
                    self.config.search.bm25_body_weight,
                ],
            )?;
            for (id, score) in lexical {
                if let Some(symbol) = self.store.symbol_by_id(&id)? {
                    let normalized = self.ranker.normalize_bm25(score);
                    if normalized < self.config.search.min_bm25_score {
                        continue;
                    }
                    let candidate = self.to_resolution(
                        symbol,
                        ResolutionSource::Lexical,
                        normalized,
                        Some(format!("bm25:{score:.3}")),
                    );
                    push(vec![candidate], &mut collected, &mut seen);
                }
            }
        }

        let mut filtered: Vec<Resolution> = collected
            .into_iter()
            .filter(|r| match query.language {
                Some(lang) => r.symbol.language == lang,
                None => true,
            })
            .filter(|r| {
                !matches!(
                    r.symbol.kind,
                    devzymer_core::model::SymbolKind::Import
                        | devzymer_core::model::SymbolKind::Export
                )
            })
            .filter(|r| Self::within_path_scope(path_filter.as_deref(), &r.symbol.path))
            .collect();

        // A behavioral question names a concept, and the repository may call
        // that concept by another name. The candidates of the first pass
        // carry the vocabulary of the repository, so their rare names are
        // harvested and searched again, and the terms they contribute weigh
        // less than the words of the question.
        let mut expanded: Vec<GroundedToken> = grounded.clone();
        if !path_like
            && path_filter.is_none()
            && !single_identifier_query(trimmed)
            && !filtered.is_empty()
        {
            let total = self.store.symbol_count().unwrap_or(1).max(1) as f64;
            let mut seeds = filtered.clone();
            for resolution in seeds.iter_mut() {
                let (_, _, _, evidence) = evidence_for(&resolution.symbol, &grounded, total);
                resolution.evidence = evidence;
            }
            seeds.sort_by(|a, b| {
                b.evidence
                    .partial_cmp(&a.evidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let known: Vec<String> = grounded
                .iter()
                .flat_map(|term| term.variants.iter().map(|(form, _)| form.clone()))
                .collect();
            // The anchor is the evidence a candidate needs to show that it
            // knows the subject of the question. A candidate below it only
            // repeats common words, and its names would lead the search away.
            let anchor = grounded
                .iter()
                .map(|term| {
                    term.variants
                        .iter()
                        .map(|(_, frequency)| (1.0 + total / (*frequency).max(1) as f64).ln())
                        .fold(0.0f64, f64::max)
                })
                .fold(0.0f64, f64::max)
                * EXPANSION_ANCHOR;
            let harvested = self.expansion_terms(&seeds, &known, anchor);
            if !harvested.is_empty() {
                let mut terms: Vec<String> = grounded
                    .iter()
                    .flat_map(|term| term.variants.iter().map(|(form, _)| form.clone()))
                    .collect();
                for (term, frequency) in &harvested {
                    terms.push(term.clone());
                    expanded.push(GroundedToken {
                        term: term.clone(),
                        variants: vec![(term.clone(), *frequency)],
                        weight: EXPANSION_WEIGHT,
                    });
                }
                let lexical = self.store.fts_search_terms(
                    &terms,
                    candidate_limit,
                    [
                        self.config.search.bm25_name_weight,
                        self.config.search.bm25_qualified_weight,
                        self.config.search.bm25_signature_weight,
                        self.config.search.bm25_doc_weight,
                        1.0,
                        self.config.search.bm25_body_weight,
                    ],
                )?;
                for (id, score) in lexical {
                    let Some(symbol) = self.store.symbol_by_id(&id)? else {
                        continue;
                    };
                    if !Self::within_path_scope(path_filter.as_deref(), &symbol.path) {
                        continue;
                    }
                    if matches!(
                        symbol.kind,
                        devzymer_core::model::SymbolKind::Import
                            | devzymer_core::model::SymbolKind::Export
                    ) {
                        continue;
                    }
                    if !matches!(query.language, Some(lang) if lang != symbol.language) {
                        let normalized = self.ranker.normalize_bm25(score);
                        if normalized < self.config.search.min_bm25_score {
                            continue;
                        }
                        let candidate = self.to_resolution(
                            symbol,
                            ResolutionSource::Lexical,
                            normalized,
                            Some(format!("bm25:{score:.3}")),
                        );
                        let key = candidate.symbol.id.0.clone();
                        match filtered.iter().position(|r| r.symbol.id.0 == key) {
                            Some(index) => {
                                if candidate.score > filtered[index].score {
                                    filtered[index] = candidate;
                                }
                            }
                            None => filtered.push(candidate),
                        }
                    }
                }
            }
        }

        // A file named in the query is strong evidence: its candidates win.
        let strongest = hints.iter().map(|(_, weight)| *weight).max().unwrap_or(0);
        for (path, weight) in &hints {
            let ratio = if strongest > 0 {
                *weight as f64 / strongest as f64
            } else {
                0.0
            };
            for resolution in filtered.iter_mut() {
                if &resolution.symbol.path == path {
                    resolution.path_hint = true;
                    resolution.hint_score = resolution.hint_score.max(ratio);
                    resolution.score += PATH_HINT_BONUS * ratio;
                    resolution.matches += 1;
                }
            }
        }

        if !path_like {
            let total = self.store.symbol_count().unwrap_or(1).max(1) as f64;
            for resolution in filtered.iter_mut() {
                let ((identity, body, doc, evidence), _) =
                    evidence_detail(&resolution.symbol, &expanded, total);
                resolution.identity_matches += identity;
                resolution.matches += identity + body + doc;
                resolution.sequence_cover = sequence_cover(&resolution.symbol, &tokens);
                resolution.evidence += evidence;
            }
        }

        // A symbol-shaped query that no name corroborates found nothing real:
        // the indexed text merely contains its parts, but no symbol is about
        // the query. An honest empty answer beats unrelated fragments. A
        // behavioral sentence keeps its shortlist even when only doc text
        // carries the evidence.
        let single_identifier = !path_like && !trimmed.contains(' ') && is_identifier_like(trimmed);
        if single_identifier {
            let identity_less = filtered
                .iter()
                .all(|r| matches!(r.source, ResolutionSource::Lexical) && r.identity_matches == 0);
            if identity_less {
                filtered.clear();
            }
        } else if !path_like {
            // A question whose terms no candidate covers carries no evidence
            // at all: the words of the question are about the code, not about
            // a subject of the repository. Answering with fragments would
            // invent a relation that does not exist. A candidate that comes
            // from a file named in the question is evidence on its own, and
            // it is kept.
            let without_evidence = filtered.iter().all(|r| {
                matches!(r.source, ResolutionSource::Lexical)
                    && r.evidence <= 0.0
                    && r.identity_matches == 0
            });
            if without_evidence {
                filtered.clear();
            }
        }

        // A query that names a file already disambiguates the bare name.
        let ambiguity = self.detect_ambiguity(trimmed, &filtered, path_like || !hints.is_empty());

        self.ranker.sort(&mut filtered);
        filtered.truncate(limit);
        files.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        files.truncate(MAX_FILE_MATCHES);

        // The diagnostics follow the order of the results, so a reader can
        // match them without guessing.
        let mut evidence: Vec<CandidateEvidence> = Vec::new();
        if query.diagnostics && !path_like {
            let total = self.store.symbol_count().unwrap_or(1).max(1) as f64;
            for resolution in filtered.iter().take(DIAGNOSTIC_CANDIDATES) {
                let (_, terms) = evidence_detail(&resolution.symbol, &expanded, total);
                evidence.push(CandidateEvidence {
                    qualified_name: resolution.symbol.qualified_name.clone(),
                    terms: terms.into_iter().take(DIAGNOSTIC_TERMS).collect(),
                });
            }
        }

        Ok(ResolutionSet {
            results: filtered,
            files,
            ambiguity,
            cache: if self.config.context.cache_enabled {
                CacheStatus::Miss
            } else {
                CacheStatus::Disabled
            },
            evidence,
        })
    }

    /// Reports a bare name that resolves to more than one definition.
    fn detect_ambiguity(
        &self,
        query: &str,
        _results: &[Resolution],
        path_like: bool,
    ) -> Option<Ambiguity> {
        if path_like || query.contains('.') || query.contains(' ') {
            return None;
        }
        // The ambiguity is about the query itself. The first candidate is not
        // consulted, because a file name match can put a symbol with an
        // unrelated name first, and the warning would name the wrong symbol.
        let matches = self.store.symbols_by_name(query).ok()?;
        // The count uses the same set the result carries, so an exported
        // symbol is not reported as a second definition of itself.
        let distinct: Vec<String> = matches
            .iter()
            .filter(|symbol| {
                !matches!(
                    symbol.kind,
                    devzymer_core::model::SymbolKind::Import
                        | devzymer_core::model::SymbolKind::Export
                )
            })
            .map(|symbol| symbol.qualified_name.clone())
            .collect();
        if distinct.len() < 2 {
            return None;
        }
        Some(Ambiguity {
            name: query.to_string(),
            count: distinct.len(),
            candidates: distinct,
        })
    }

    /// Matches a query against the indexed file names, so a query that names a
    /// file resolves to that file even when no symbol carries the name.
    fn match_files(&self, query: &str) -> Result<Vec<MatchedFile>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let normalized = trimmed.replace('\\', "/");
        let mut out: Vec<MatchedFile> = Vec::new();

        if let Some(file) = self.store.file_by_path(&normalized)? {
            out.push(MatchedFile {
                file,
                score: 1.0,
                reason: "caminho exato".to_string(),
                hint: false,
                hint_weight: 0,
            });
            return Ok(out);
        }

        for file in self.store.files_by_path_suffix(&normalized)? {
            if out.len() >= MAX_FILE_MATCHES {
                break;
            }
            if !out.iter().any(|f| f.file.path == file.path) {
                out.push(MatchedFile {
                    file,
                    score: 0.96,
                    reason: "sufixo de caminho".to_string(),
                    hint: false,
                    hint_weight: 0,
                });
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }

        // A multi word query names a file only through one of its terms, so
        // the whole query is never treated as a file name.
        if !trimmed.contains(' ') {
            let stem = devzymer_core::store::stem_of(&normalized);
            if stem.len() >= 3 {
                for file in self.store.files_by_stem(&stem)? {
                    if out.len() >= MAX_FILE_MATCHES {
                        break;
                    }
                    if !out.iter().any(|f| f.file.path == file.path) {
                        out.push(MatchedFile {
                            file,
                            score: 0.93,
                            reason: "nome do arquivo".to_string(),
                            hint: false,
                            hint_weight: 0,
                        });
                    }
                }
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }

        // A term of the query may name a file, which hints at the candidates.
        for token in query_tokens(trimmed) {
            let stem = devzymer_core::store::stem_of(&token);
            if stem.len() < 3 {
                continue;
            }
            if out.len() >= MAX_FILE_MATCHES {
                break;
            }
            for file in self.store.files_by_stem(&stem)? {
                if !out.iter().any(|f| f.file.path == file.path) {
                    out.push(MatchedFile {
                        file,
                        score: 0.6,
                        reason: format!("nome do arquivo na consulta: {token}"),
                        hint: true,
                        hint_weight: token.chars().count(),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Symbols that reference the given symbol, used for impact analysis.
    pub fn related(&self, symbol_id: &str) -> Result<Vec<Resolution>> {
        let graph = devzymer_graph::GraphQuery::new(self.store);
        let id = devzymer_core::model::SymbolId(symbol_id.to_string());
        let relations = graph.callers(&id)?;
        let mut out = Vec::new();
        for relation in relations {
            if let devzymer_core::model::Entity::Symbol(src) = &relation.src {
                if let Some(symbol) = self.store.symbol_by_id(src.as_str())? {
                    out.push(self.to_resolution(
                        symbol,
                        ResolutionSource::GraphExpansion,
                        ResolutionSource::GraphExpansion.base_score(),
                        Some(format!("{:?}", relation.kind)),
                    ));
                }
            }
        }
        Ok(out)
    }

    fn to_resolution(
        &self,
        symbol: Symbol,
        source: ResolutionSource,
        score: f64,
        note: Option<String>,
    ) -> Resolution {
        let confidence = match source {
            ResolutionSource::ExactPath
            | ResolutionSource::ExactSymbol
            | ResolutionSource::QualifiedSymbol => Confidence::Confirmed,
            _ => Confidence::Heuristic,
        };
        Resolution {
            symbol,
            source,
            score,
            confidence,
            note,
            matches: 0,
            identity_matches: 0,
            sequence_cover: 0,
            evidence: 0.0,
            path_hint: false,
            hint_score: 0.0,
        }
    }
}

/// Bonus applied to candidates that live in a file named by the query.
const PATH_HINT_BONUS: f64 = 0.30;

/// The number of symbols a single file contributes to a result.
const MAX_FILE_CANDIDATES: usize = 6;

/// The maximum number of file matches a resolution reports. File answers
/// are presented before the symbol candidates, so the cap keeps the
/// response short even when many files share a name.
const MAX_FILE_MATCHES: usize = 5;

/// Version of the technical vocabulary used to bridge Portuguese and English
/// terms. The table only proposes candidates: a proposal is used only when
/// the repository really contains that form.
pub const LEXICON_VERSION: &str = "lexicon_v2";

/// Technical vocabulary shared by Portuguese and English codebases. Both
/// directions are used, so a question written in English reaches a symbol
/// written in Portuguese and the other way around.
const TECHNICAL_LEXICON: &[(&str, &str)] = &[
    ("ligacao", "call"),
    ("chamada", "call"),
    ("atendente", "attendant"),
    ("atendente", "agent"),
    ("operador", "operator"),
    ("humano", "human"),
    ("disponivel", "available"),
    ("disponibilidade", "availability"),
    ("transferir", "transfer"),
    ("transferencia", "transfer"),
    ("encaminhar", "handoff"),
    ("encaminhamento", "handoff"),
    ("telefonia", "telephony"),
    ("telefone", "phone"),
    ("ponte", "bridge"),
    ("atendimento", "support"),
    ("agendar", "schedule"),
    ("agendamento", "scheduling"),
    ("sessao", "session"),
    ("gravacao", "recording"),
    ("mensagem", "message"),
    ("resposta", "response"),
    ("cliente", "customer"),
    ("contato", "contact"),
    ("pedido", "request"),
    ("pagamento", "payment"),
    ("cancelamento", "cancellation"),
    ("erro", "error"),
    ("limite", "limit"),
    ("limiar", "threshold"),
    ("janela", "window"),
    ("tempo", "timeout"),
    ("dispositivo", "device"),
    ("roteiro", "script"),
    ("resumo", "summary"),
    ("estado", "state"),
    ("fila", "queue"),
    ("conexao", "connection"),
    ("envio", "send"),
    ("recebimento", "receive"),
    ("notificacao", "notification"),
    ("cadastro", "register"),
    ("busca", "search"),
    ("consulta", "query"),
    ("relatorio", "report"),
    ("configuracao", "config"),
    ("credencial", "credential"),
    ("autenticacao", "auth"),
    ("senha", "password"),
    ("assinatura", "subscription"),
    ("fatura", "invoice"),
    ("reembolso", "refund"),
    ("webhook", "webhook"),
    ("roteamento", "routing"),
    ("cache", "cache"),
    ("prioridade", "priority"),
    ("motivo", "reason"),
    ("pacote", "package"),
    ("numero", "number"),
    ("data", "date"),
    ("hora", "hour"),
    ("minuto", "minute"),
];

/// The forms a term may take in the repository: itself, a morphological stem
/// and the technical vocabulary in the other language.
fn term_variants(token: &str) -> Vec<String> {
    let folded = devzymer_core::util::diacritic_fold(&token.to_lowercase());
    let mut variants = vec![folded.clone()];
    if let Some(stem) = stem_variant(&folded) {
        if !variants.contains(&stem) {
            variants.push(stem);
        }
    }
    for alias in lexicon_variants(&folded) {
        if !variants.contains(&alias) {
            variants.push(alias);
        }
    }
    variants
}

/// The stem of an inflected word, shared by the Portuguese and the English
/// form of the same root: `transferring` and `transferir` both stem to
/// `transfer`.
fn stem_variant(token: &str) -> Option<String> {
    for suffix in ["indo", "ando", "endo", "ing", "ed", "ir", "es", "s"] {
        if token.len() > suffix.len() + 3 && token.ends_with(suffix) {
            let base = &token[..token.len() - suffix.len()];
            let chars: Vec<char> = base.chars().collect();
            let collapsed: String =
                if chars.len() >= 2 && chars[chars.len() - 1] == chars[chars.len() - 2] {
                    chars[..chars.len() - 1].iter().collect()
                } else {
                    base.to_string()
                };
            return Some(collapsed);
        }
    }
    None
}

/// The technical vocabulary entries that match the term, in both directions.
fn lexicon_variants(token: &str) -> Vec<String> {
    let folded = devzymer_core::util::diacritic_fold(&token.to_lowercase());
    let mut out: Vec<String> = Vec::new();
    for (left, right) in TECHNICAL_LEXICON {
        let left = devzymer_core::util::diacritic_fold(&left.to_lowercase());
        let right = devzymer_core::util::diacritic_fold(&right.to_lowercase());
        let other = if folded == left {
            Some(right)
        } else if folded == right {
            Some(left)
        } else {
            None
        };
        if let Some(other) = other {
            if !out.contains(&other) {
                out.push(other);
            }
        }
    }
    out
}

/// Length of the largest ordered subsequence of name segments that follows
/// the query token order. A name whose segments read like the query phrase
/// ("token de acesso do gateway" vs `API_GATEWAY_TOKEN`) is a closer answer
/// than one that merely carries the same segments in another order.
fn sequence_cover(symbol: &Symbol, tokens: &[String]) -> usize {
    if tokens.is_empty() {
        return 0;
    }
    let segments: Vec<String> = devzymer_core::util::split_identifier(&symbol.qualified_name)
        .split(' ')
        .map(devzymer_core::util::diacritic_fold)
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return 0;
    }
    let mut best = 0;
    for start in 0..tokens.len() {
        let mut matched = 0;
        let mut seg_index = 0usize;
        for token in &tokens[start..] {
            let token = devzymer_core::util::diacritic_fold(&token.to_lowercase());
            if token.len() < 3 {
                continue;
            }
            if let Some(index) = segments
                .iter()
                .skip(seg_index)
                .position(|seg| seg == &token)
            {
                matched += 1;
                seg_index += index + 1;
            }
        }
        if matched > best {
            best = matched;
        }
    }
    best
}

/// True when a token looks like an identifier rather than a word of a
/// sentence.
pub fn is_identifier_like(token: &str) -> bool {
    token
        .chars()
        .skip(1)
        .any(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
}

/// True when the question asks for one specific symbol, in which case the
/// query is not expanded with the vocabulary of the repository.
fn single_identifier_query(query: &str) -> bool {
    !query.contains(' ') && is_identifier_like(query)
}

/// True when the query names a file rather than a symbol. A path query is
/// resolved by the file stages, and its parts must not be matched against
/// symbol names.
fn looks_like_path(query: &str) -> bool {
    if query.contains('/') || query.contains('\\') {
        return true;
    }
    const EXTENSIONS: [&str; 12] = [
        "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rs", "go", "java", "cs", "rb",
    ];
    match query.rsplit_once('.') {
        Some((_, ext)) => EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// A query term with the forms it may take in the repository and the
/// rarity of each form.
#[derive(Clone)]
struct GroundedToken {
    /// The term as the question wrote it.
    term: String,
    /// The forms of the term, with the number of symbols that contain them.
    variants: Vec<(String, usize)>,
    /// How much the term counts. A term the user wrote counts fully; a term
    /// inferred from the repository vocabulary counts less, because it is a
    /// hypothesis about the question.
    weight: f64,
}

/// How many top candidates feed the expansion of the query.
const EXPANSION_SEEDS: usize = 8;
/// How many harvested names are weighed before the rarity filter.
const EXPANSION_CANDIDATES: usize = 24;
/// How many terms the expansion adds to the query.
const EXPANSION_TERMS: usize = 4;
/// A harvested name is kept when it appears in at most one in this many
/// symbols, so the expansion follows a specific vocabulary.
const EXPANSION_RARITY: usize = 60;
/// How much an inferred term weighs against a term the user wrote.
const EXPANSION_WEIGHT: f64 = 0.55;
/// The share of the evidence of the rarest term of the question that a
/// candidate needs to show before its names are used to expand the search.
const EXPANSION_ANCHOR: f64 = 0.5;

/// Weight of a term found in the body: a word inside the implementation is
/// weaker evidence than a word inside the name.
const BODY_EVIDENCE_WEIGHT: f64 = 0.6;
/// Weight of a term found in the documentation of the symbol.
const DOC_EVIDENCE_WEIGHT: f64 = 0.9;
/// A vocabulary of the words of a text, indexed by the first letters, so a
/// morphological variant of a term is found without scanning every word.
struct WordIndex {
    exact: std::collections::HashSet<String>,
    by_prefix: std::collections::HashMap<String, Vec<String>>,
}

impl WordIndex {
    fn build(text: &str) -> WordIndex {
        let mut exact = std::collections::HashSet::new();
        let mut by_prefix: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for part in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if part.len() < 3 {
                continue;
            }
            let folded = devzymer_core::util::diacritic_fold(&part.to_lowercase());
            let mut words = vec![folded];
            for word in devzymer_core::util::split_identifier(part).split(' ') {
                let word = devzymer_core::util::diacritic_fold(word);
                if word.len() >= 3 && !words.contains(&word) {
                    words.push(word);
                }
            }
            for word in words {
                exact.insert(word.clone());
                let key: String = word.chars().take(4).collect();
                by_prefix.entry(key).or_default().push(word);
            }
        }
        WordIndex { exact, by_prefix }
    }

    /// True when the text contains the term or a morphological variant of it.
    fn covers(&self, form: &str) -> bool {
        if form.len() < 3 {
            return false;
        }
        if self.exact.contains(form) {
            return true;
        }
        let key: String = form.chars().take(4).collect();
        let Some(candidates) = self.by_prefix.get(&key) else {
            return false;
        };
        candidates.iter().any(|word| {
            let shorter = form.chars().count().min(word.chars().count());
            shared_prefix(form, word) >= 4 && shared_prefix(form, word) * 10 >= 7 * shorter
        })
    }
}

/// Counts how a candidate covers the query terms and weighs that coverage.
/// Every form a term may take is tried, so a question that says "call"
/// still reaches a symbol that says "ligacao", and a term found in the name
/// weighs more than one found in the body. Only distinct terms count, so a
/// large file does not win by mentioning more words.
fn evidence_for(
    symbol: &Symbol,
    grounded: &[GroundedToken],
    total: f64,
) -> (usize, usize, usize, f64) {
    evidence_detail(symbol, grounded, total).0
}

/// Counts how a candidate covers the query terms, weighs that coverage and
/// records which term was found where. Every form a term may take is tried,
/// so a question that says "call" still reaches a symbol that says
/// "ligacao", and a term found in the name weighs more than one found in
/// the body. Only distinct terms count, so a large file does not win by
/// mentioning more words.
fn evidence_detail(
    symbol: &Symbol,
    grounded: &[GroundedToken],
    total: f64,
) -> ((usize, usize, usize, f64), Vec<EvidenceDetail>) {
    if grounded.is_empty() {
        return ((0, 0, 0, 0.0), Vec::new());
    }
    let name = devzymer_core::util::diacritic_fold(&symbol.name.to_lowercase());
    let mut segments: Vec<String> = devzymer_core::util::split_identifier(&symbol.qualified_name)
        .split(' ')
        .map(devzymer_core::util::diacritic_fold)
        .filter(|s| !s.is_empty())
        .collect();
    segments.sort_by_key(|s| std::cmp::Reverse(s.len()));
    segments.dedup();
    let segment_index = WordIndex::build(&segments.join(" "));
    let body_index = WordIndex::build(symbol.body.as_deref().unwrap_or(""));
    let doc_index = WordIndex::build(symbol.doc.as_deref().unwrap_or(""));
    let mut identity = 0usize;
    let mut body_hits = 0usize;
    let mut doc_hits = 0usize;
    let mut evidence = 0.0f64;
    let mut details: Vec<EvidenceDetail> = Vec::new();
    for term in grounded {
        let mut best_weight = 0.0f64;
        let mut best_field = 0u8;
        let mut best_form = String::new();
        for (form, frequency) in &term.variants {
            if form.len() < 3 {
                continue;
            }
            let idf = (1.0 + total / (*frequency).max(1) as f64).ln() * term.weight;
            let in_name = segment_index.covers(form)
                || name.starts_with(form)
                || segments.iter().any(|segment| segment.starts_with(form));
            let (weight, field) = if in_name {
                (idf, 1u8)
            } else if doc_index.covers(form) {
                (idf * DOC_EVIDENCE_WEIGHT, 2u8)
            } else if body_index.covers(form) {
                (idf * BODY_EVIDENCE_WEIGHT, 3u8)
            } else {
                continue;
            };
            let better = match best_field {
                0 => true,
                1 => field == 1,
                2 => field == 2,
                _ => false,
            };
            if better {
                best_weight = weight;
                best_field = field;
                best_form = form.clone();
            }
        }
        match best_field {
            1 => identity += 1,
            2 => doc_hits += 1,
            3 => body_hits += 1,
            _ => continue,
        }
        evidence += best_weight;
        details.push(EvidenceDetail {
            term: term.term.clone(),
            form: best_form,
            field: match best_field {
                1 => "name",
                2 => "doc",
                _ => "body",
            }
            .to_string(),
            weight: (best_weight * 1000.0).round() / 1000.0,
        });
    }
    ((identity, body_hits, doc_hits, evidence), details)
}

/// How many candidates carry the diagnostic of the terms they matched.
const DIAGNOSTIC_CANDIDATES: usize = 10;
/// How many terms are reported for each candidate.
const DIAGNOSTIC_TERMS: usize = 8;

/// Length of the longest common prefix of two strings.
fn shared_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// The leading form of a long word, for the fallback that matches the
/// infinitive named in the code from an inflected word of the sentence.
fn stem_prefix(token: &str) -> Option<String> {
    let mut chars: Vec<char> = token.chars().collect();
    if chars.len() < 7 {
        return None;
    }
    chars.truncate(chars.len() - 3);
    Some(chars.into_iter().collect())
}

/// Language filter helper used by the CLI.
pub fn parse_language(value: &str) -> Option<Language> {
    Language::from_display(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use devzymer_core::model::Span;

    fn symbol(name: &str, qualified: &str) -> Symbol {
        Symbol {
            id: devzymer_core::model::SymbolId::new("src/lib.rs", qualified),
            name: name.to_string(),
            qualified_name: qualified.to_string(),
            kind: devzymer_core::model::SymbolKind::Function,
            container: None,
            signature: None,
            doc: None,
            path: "src/lib.rs".to_string(),
            language: Language::Rust,
            span: Span::new(0, 10, 1, 1),
            exported: true,
            is_async: false,
            body: None,
        }
    }

    fn store_with(path: &str, name: &str, qualified: &str) -> Store {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        let record = devzymer_core::store::FileRecord {
            id: 0,
            path: path.to_string(),
            language: Language::TypeScript,
            size_bytes: 120,
            mtime_ms: 1000,
            content_hash: "abc".to_string(),
            symbol_count: 1,
            indexed_at_ms: 2000,
        };
        let file_id = store.upsert_file(&record).unwrap();
        let mut sym = symbol(name, qualified);
        sym.path = path.to_string();
        sym.id = devzymer_core::model::SymbolId::new(path, qualified);
        store.replace_symbols(file_id, &[sym]).unwrap();
        store
    }

    #[test]
    fn a_symbol_covering_every_term_scores_higher() {
        let terms = grounded_terms(&["symbol", "count"]);
        let target = symbol("symbol_count", "Store.symbol_count");
        let partial = symbol("count_lines", "count_lines");
        assert_eq!(evidence_for(&target, &terms, 100.0).0, 2);
        assert_eq!(evidence_for(&partial, &terms, 100.0).0, 1);
    }

    #[test]
    fn the_camel_case_words_of_a_name_cover_the_terms() {
        let terms = grounded_terms(&["leitura", "voice", "call"]);
        let target = symbol("useLeituraVoiceCall", "useLeituraVoiceCall");
        assert_eq!(evidence_for(&target, &terms, 100.0).0, 3);
    }

    #[test]
    fn an_inflected_portuguese_word_counts_as_the_code_word() {
        let terms = grounded_terms(&["limitador"]);
        let target = symbol("limitar", "limitar");
        let other = symbol("registros", "registros");
        assert_eq!(evidence_for(&target, &terms, 100.0).0, 1);
        assert_eq!(evidence_for(&other, &terms, 100.0).0, 0);
    }

    #[test]
    fn body_only_word_hits_are_counted_apart_from_identity_hits() {
        let terms = grounded_terms(&["banco", "real"]);
        let mut target = symbol("calculoDeNota", "calculoDeNota");
        target.body = Some("if (!bancoReal()) { throw }".to_string());
        let (identity, body, doc, evidence) = evidence_for(&target, &terms, 100.0);
        assert_eq!((identity, body, doc), (0, 2, 0));
        assert!(evidence > 0.0);
    }

    #[test]
    fn the_vocabulary_bridges_the_two_languages_and_the_inflections() {
        let variants = term_variants("transferring");
        assert!(variants.contains(&"transfer".to_string()));
        let aliases = lexicon_variants("attendant");
        assert!(aliases.contains(&"atendente".to_string()));
        let aliases = lexicon_variants("atendente");
        assert!(aliases.contains(&"attendant".to_string()));
    }

    #[test]
    fn the_path_filter_restricts_the_result_to_the_file() {
        let store = store_with_files(&[
            (
                "src/lib/financeiro.ts",
                vec![("calculoDeNota", "calculoDeNota")],
            ),
            ("src/lib/outro.ts", vec![("outraFuncao", "outraFuncao")]),
        ]);
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let mut query = ResolveQuery::new("funcao");
        query.path = Some("src/lib/financeiro.ts".to_string());
        let set = resolver.resolve(&query).expect("resolve");
        assert!(!set.results.is_empty());
        assert!(set
            .results
            .iter()
            .all(|r| r.symbol.path == "src/lib/financeiro.ts"));
    }

    #[test]
    fn the_path_filter_covers_a_directory() {
        let store = store_with_files(&[
            ("src/lib/a.ts", vec![("primeira", "primeira")]),
            ("src/lib/b.ts", vec![("segunda", "segunda")]),
            ("src/outro/c.ts", vec![("terceira", "terceira")]),
        ]);
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let mut query = ResolveQuery::new("funcao");
        query.path = Some("src/lib".to_string());
        let set = resolver.resolve(&query).expect("resolve");
        assert!(!set.results.is_empty());
        assert!(set
            .results
            .iter()
            .all(|r| r.symbol.path.starts_with("src/lib/")));
    }

    #[test]
    fn a_path_filter_without_a_match_returns_nothing() {
        let store = store_with_files(&[("src/lib/financeiro.ts", vec![("nota", "nota")])]);
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let mut query = ResolveQuery::new("nota");
        query.path = Some("src/nao/existe.ts".to_string());
        let set = resolver.resolve(&query).expect("resolve");
        assert!(set.results.is_empty());
    }

    #[test]
    fn a_path_outside_the_workspace_is_refused() {
        assert!(!Resolver::path_is_inside_workspace("../fora.ts"));
        assert!(!Resolver::path_is_inside_workspace("/etc/passwd"));
        assert!(Resolver::path_is_inside_workspace("src/lib/financeiro.ts"));
        assert!(Resolver::path_is_inside_workspace(
            "src\\lib\\financeiro.ts"
        ));
    }

    #[test]
    fn a_windows_path_describes_the_same_file() {
        assert_eq!(
            Resolver::normalize_repository_path("src\\lib\\financeiro.ts"),
            "src/lib/financeiro.ts"
        );
        assert_eq!(
            Resolver::normalize_repository_path("./src/lib/financeiro.ts"),
            "src/lib/financeiro.ts"
        );
    }

    #[test]
    fn the_diagnostics_explain_why_a_candidate_matched() {
        let store = store_with_files(&[(
            "src/lib/financeiro.ts",
            vec![("calculoDeNota", "calculoDeNota")],
        )]);
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let mut query = ResolveQuery::new("calculo de nota do banco");
        query.diagnostics = true;
        let set = resolver.resolve(&query).expect("resolve");
        assert!(!set.evidence.is_empty());
        assert_eq!(set.evidence[0].qualified_name, "calculoDeNota");
        assert!(!set.evidence[0].terms.is_empty());
    }

    #[test]
    fn a_query_without_the_diagnostics_carries_no_explanation() {
        let store = store_with_files(&[(
            "src/lib/financeiro.ts",
            vec![("calculoDeNota", "calculoDeNota")],
        )]);
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("calculo de nota do banco"))
            .expect("resolve");
        assert!(set.evidence.is_empty());
    }

    fn grounded_terms(terms: &[&str]) -> Vec<GroundedToken> {
        terms
            .iter()
            .map(|term| GroundedToken {
                term: term.to_string(),
                variants: vec![(term.to_string(), 1usize)],
                weight: 1.0,
            })
            .collect()
    }

    #[test]
    fn a_search_without_identity_evidence_returns_honest_nothing() {
        // The only candidate is a lexical row whose terms appear only in the
        // indexed doc text; no symbol identity corroborates the query.
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        let record = devzymer_core::store::FileRecord {
            id: 0,
            path: "src/lib/financeiro.ts".to_string(),
            language: devzymer_core::model::Language::TypeScript,
            size_bytes: 40,
            mtime_ms: 1,
            content_hash: "x".to_string(),
            symbol_count: 1,
            indexed_at_ms: 1,
        };
        let file_id = store.upsert_file(&record).unwrap();
        let mut sym = symbol("calculoDeNota", "calculoDeNota");
        sym.doc = Some("sistema de banco real para testes de faturamento".to_string());
        store.replace_symbols(file_id, &[sym]).unwrap();
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("faturamento do banco real"))
            .expect("resolve");
        assert!(set.results.is_empty());
    }

    fn store_with_files(files: &[(&str, Vec<(&str, &str)>)]) -> Store {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        for (path, syms) in files {
            let record = devzymer_core::store::FileRecord {
                id: 0,
                path: (*path).to_string(),
                language: Language::TypeScript,
                size_bytes: 120,
                mtime_ms: 1000,
                content_hash: "abc".to_string(),
                symbol_count: syms.len(),
                indexed_at_ms: 2000,
            };
            let file_id = store.upsert_file(&record).unwrap();
            let list: Vec<Symbol> = syms
                .iter()
                .map(|(name, qualified)| {
                    let mut sym = symbol(name, qualified);
                    sym.path = (*path).to_string();
                    sym.id = devzymer_core::model::SymbolId::new(path, qualified);
                    sym
                })
                .collect();
            store.replace_symbols(file_id, &list).unwrap();
        }
        store
    }

    fn resolved_first(store: &Store, query: &str) -> String {
        let config = Config::default();
        let resolver = Resolver::new(store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new(query))
            .expect("resolve");
        set.results
            .first()
            .map(|r| r.symbol.name.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_behaviour_sentence_finds_the_camel_case_hook() {
        let store = store_with_files(&[
            (
                "src/hooks/useVoiceCall.ts",
                vec![("useLeituraVoiceCall", "useLeituraVoiceCall")],
            ),
            (
                "src/lib/Metricas.ts",
                vec![("Metricas", "Metricas"), ("leitura", "leitura")],
            ),
        ]);
        let first = resolved_first(
            &store,
            "leitura de uma acao do voice-call com estado visivel",
        );
        assert_eq!(first, "useLeituraVoiceCall");
    }

    #[test]
    fn an_accented_inflected_sentence_still_finds_the_symbol() {
        let store = store_with_files(&[
            ("src/limite/guardiao.ts", vec![("limitar", "limitar")]),
            (
                "src/lib/tratar.ts",
                vec![("tratarAcaoDaEscalada", "tratarAcaoDaEscalada")],
            ),
            (
                "src/lib/aplicar.ts",
                vec![("aplicarEfeitoDaDecisao", "aplicarEfeitoDaDecisao")],
            ),
        ]);
        let first = resolved_first(
            &store,
            "consulta o limitador no banco e recusa o pedido quando a janela esgota",
        );
        assert_eq!(first, "limitar");
    }

    #[test]
    fn a_sentence_from_a_doc_still_finds_the_documented_symbol() {
        let store = store_with_files(&[
            (
                "src/banco/atendimento.ts",
                vec![(
                    "transferenciaChamadaParaAtendente",
                    "transferenciaChamadaParaAtendente",
                )],
            ),
            (
                "src/lib/outros.ts",
                vec![("encaminharMensagem", "encaminharMensagem")],
            ),
        ]);
        let first = resolved_first(
            &store,
            "transferência de uma chamada para o atendente humano",
        );
        assert_eq!(first, "transferenciaChamadaParaAtendente");
    }

    #[test]
    fn a_query_naming_a_file_resolves_to_that_file() {
        let store = store_with(
            "src/hooks/useVoiceCall.ts",
            "useLeituraVoiceCall",
            "useLeituraVoiceCall",
        );
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("useVoiceCall"))
            .expect("resolve");
        assert!(set
            .files
            .iter()
            .any(|f| f.path == "src/hooks/useVoiceCall.ts"));
        assert!(set
            .results
            .iter()
            .any(|r| r.symbol.name == "useLeituraVoiceCall"));
    }

    #[test]
    fn a_path_query_ranks_the_exact_file_first() {
        let store = store_with(
            "src/hooks/useVoiceCall.ts",
            "useLeituraVoiceCall",
            "useLeituraVoiceCall",
        );
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("src/hooks/useVoiceCall.ts"))
            .expect("resolve");
        assert!(!set.results.is_empty());
        assert_eq!(set.results[0].symbol.path, "src/hooks/useVoiceCall.ts");
        assert_eq!(set.results[0].source, ResolutionSource::ExactPath);
    }

    fn store_file_only(path: &str) -> Store {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        let record = devzymer_core::store::FileRecord {
            id: 0,
            path: path.to_string(),
            language: Language::Sql,
            size_bytes: 2048,
            mtime_ms: 1000,
            content_hash: "abc".to_string(),
            symbol_count: 0,
            indexed_at_ms: 2000,
        };
        store.upsert_file(&record).unwrap();
        store
    }

    #[test]
    fn a_file_without_symbols_is_still_a_file_answer() {
        let store = store_file_only("appwrite/banco/20261005120000_voz_twilio.sql");
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("20261005120000_voz_twilio"))
            .expect("resolve");
        assert_eq!(set.files.len(), 1);
        assert_eq!(
            set.files[0].path,
            "appwrite/banco/20261005120000_voz_twilio.sql"
        );
        assert_eq!(set.files[0].reason, "nome do arquivo");
        assert!(set.results.is_empty());
    }

    #[test]
    fn file_matches_are_capped_by_the_limit() {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        for index in 0..20 {
            let record = devzymer_core::store::FileRecord {
                id: 0,
                path: format!("d{index}/dup.sql"),
                language: Language::Sql,
                size_bytes: 100,
                mtime_ms: 1000,
                content_hash: "abc".to_string(),
                symbol_count: 0,
                indexed_at_ms: 2000,
            };
            store.upsert_file(&record).unwrap();
        }
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("dup"))
            .expect("resolve");
        assert_eq!(set.files.len(), MAX_FILE_MATCHES);
    }

    #[test]
    fn a_file_named_in_the_query_boosts_its_candidates() {
        let store = store_with("supabase/limites.ts", "objeto", "objeto");
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("limites.ts objeto"))
            .expect("resolve");
        assert!(!set.results.is_empty());
        assert_eq!(set.results[0].symbol.path, "supabase/limites.ts");
        assert!(set.results[0].path_hint);
    }

    #[test]
    fn a_bare_name_with_several_definitions_is_reported_as_ambiguous() {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        for (path, qualified) in [("a.ts", "objeto"), ("b.ts", "objeto")] {
            let record = devzymer_core::store::FileRecord {
                id: 0,
                path: path.to_string(),
                language: Language::TypeScript,
                size_bytes: 10,
                mtime_ms: 1,
                content_hash: "x".to_string(),
                symbol_count: 1,
                indexed_at_ms: 1,
            };
            let file_id = store.upsert_file(&record).unwrap();
            let mut sym = symbol("objeto", qualified);
            sym.path = path.to_string();
            sym.id = devzymer_core::model::SymbolId::new(path, qualified);
            store.replace_symbols(file_id, &[sym]).unwrap();
        }
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("objeto"))
            .expect("resolve");
        let ambiguity = set.ambiguity.expect("ambiguity");
        assert_eq!(ambiguity.count, 2);
        assert_eq!(ambiguity.name, "objeto");
    }

    #[test]
    fn a_term_of_a_long_query_that_names_a_file_contributes_its_candidates() {
        let store = store_with(
            "src/hooks/useVoiceCall.ts",
            "useLeituraVoiceCall",
            "useLeituraVoiceCall",
        );
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new(
                "Onde esta o hook useVoiceCall? Apenas localize, nao leia arquivos inteiros.",
            ))
            .expect("resolve");
        assert!(set
            .files
            .iter()
            .any(|f| f.path == "src/hooks/useVoiceCall.ts"));
        assert!(set
            .results
            .iter()
            .any(|r| r.symbol.name == "useLeituraVoiceCall"));
    }

    #[test]
    fn an_exact_symbol_outranks_a_file_named_by_a_term_of_the_query() {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        for (path, name, qualified) in [
            ("src/estado.ts", "Estado", "Estado"),
            ("src/avaliacao.ts", "estado", "AdaptadorDeAvaliacao.estado"),
        ] {
            let record = devzymer_core::store::FileRecord {
                id: 0,
                path: path.to_string(),
                language: Language::TypeScript,
                size_bytes: 10,
                mtime_ms: 1,
                content_hash: "x".to_string(),
                symbol_count: 1,
                indexed_at_ms: 1,
            };
            let file_id = store.upsert_file(&record).unwrap();
            let mut sym = symbol(name, qualified);
            sym.path = path.to_string();
            sym.id = devzymer_core::model::SymbolId::new(path, qualified);
            store.replace_symbols(file_id, &[sym]).unwrap();
        }
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("AdaptadorDeAvaliacao.estado"))
            .expect("resolve");
        assert!(!set.results.is_empty());
        assert_eq!(
            set.results[0].symbol.qualified_name,
            "AdaptadorDeAvaliacao.estado"
        );
    }

    #[test]
    fn a_phrase_word_does_not_bury_the_named_file() {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        let record = devzymer_core::store::FileRecord {
            id: 0,
            path: "src/hooks/useVoiceCall.ts".to_string(),
            language: Language::TypeScript,
            size_bytes: 10,
            mtime_ms: 1,
            content_hash: "x".to_string(),
            symbol_count: 1,
            indexed_at_ms: 1,
        };
        let file_id = store.upsert_file(&record).unwrap();
        let mut sym = symbol("useLeituraVoiceCall", "useLeituraVoiceCall");
        sym.path = "src/hooks/useVoiceCall.ts".to_string();
        sym.id =
            devzymer_core::model::SymbolId::new("src/hooks/useVoiceCall.ts", "useLeituraVoiceCall");
        store.replace_symbols(file_id, &[sym]).unwrap();
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let set = resolver
            .resolve(&ResolveQuery::new("useVoiceCall"))
            .expect("resolve");
        assert!(!set.results.is_empty());
        assert_eq!(set.results[0].symbol.path, "src/hooks/useVoiceCall.ts");
    }

    #[test]
    fn the_cache_serves_the_second_identical_query() {
        let store = store_with(
            "src/hooks/useVoiceCall.ts",
            "useLeituraVoiceCall",
            "useLeituraVoiceCall",
        );
        let config = Config::default();
        let resolver = Resolver::new(&store, &config);
        let first = resolver
            .resolve(&ResolveQuery::new("useVoiceCall"))
            .expect("resolve");
        assert_eq!(first.cache, CacheStatus::Miss);
        let second = resolver
            .resolve(&ResolveQuery::new("useVoiceCall"))
            .expect("resolve");
        assert_eq!(second.cache, CacheStatus::Hit);
        assert_eq!(first.results.len(), second.results.len());
    }
}
