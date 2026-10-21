use std::path::Path;

use devzymer_core::config::Config;
use devzymer_core::store::Store;
use devzymer_core::Result;
use devzymer_search::resolve::Resolution;
use devzymer_search::Resolver;
use serde::Serialize;

use crate::budget::{fragment_tokens, ContextLevel, Fragment};

#[derive(Debug, Clone, Serialize)]
pub struct ContextBundle {
    pub level: ContextLevel,
    pub fragments: Vec<Fragment>,
    pub tokens: usize,
    pub bytes: usize,
    pub truncated: bool,
    pub notes: Vec<String>,
    pub revision: u64,
}

impl ContextBundle {
    /// Compact representation used for the MCP text payload.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for fragment in &self.fragments {
            out.push_str(&fragment.location_line());
            out.push('\n');
            if let Some(sig) = &fragment.signature {
                out.push_str("  sig: ");
                out.push_str(sig);
                out.push('\n');
            }
            if let Some(text) = &fragment.text {
                for line in text.lines() {
                    out.push_str("  ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        out.trim_end().to_string()
    }
}

pub struct ContextManager<'a> {
    store: &'a Store,
    config: &'a Config,
    #[allow(dead_code)]
    root: &'a Path,
}

impl<'a> ContextManager<'a> {
    pub fn new(store: &'a Store, config: &'a Config, root: &'a Path) -> Self {
        Self {
            store,
            config,
            root,
        }
    }

    /// Builds a context bundle honouring the requested disclosure level and
    /// the token budget. Fragments are dropped from the lowest ranked end
    /// when the budget is exceeded; essential evidence is never silently
    /// removed, the bundle is marked as truncated instead.
    pub fn bundle(
        &self,
        resolutions: &[Resolution],
        level: ContextLevel,
        budget: Option<usize>,
    ) -> Result<ContextBundle> {
        let budget = budget.unwrap_or(self.config.search.default_token_budget);
        let mut notes: Vec<String> = Vec::new();
        let mut fragments: Vec<Fragment> = Vec::new();
        let mut seen: Vec<(String, u32, u32)> = Vec::new();

        for resolution in resolutions {
            let symbol = &resolution.symbol;
            let key = (
                symbol.path.clone(),
                symbol.span.start_line,
                symbol.span.end_line,
            );
            if self.config.context.dedupe && seen.contains(&key) {
                continue;
            }
            seen.push(key);

            let mut fragment = Fragment {
                path: symbol.path.clone(),
                symbol: symbol.qualified_name.clone(),
                kind: symbol.kind.display().to_string(),
                start_line: symbol.span.start_line,
                end_line: symbol.span.end_line,
                confidence: resolution.confidence.display().to_string(),
                score: resolution.score,
                signature: None,
                doc: None,
                text: None,
                tokens: 0,
            };

            if level >= ContextLevel::Evidence {
                fragment.signature = symbol.signature.clone();
                fragment.doc = symbol.doc.clone();
            }

            if level >= ContextLevel::Implementation {
                if let Some(body) = symbol.body.as_ref() {
                    let max = self.config.search.max_fragment_bytes;
                    let clipped: String = if body.len() > max {
                        let mut end = max;
                        while end > 0 && !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        let mut s = body[..end].to_string();
                        s.push_str("\n… (trecho truncado)");
                        s
                    } else {
                        body.clone()
                    };
                    fragment.text = Some(clipped);
                }
            }

            fragment.tokens = fragment_tokens(&fragment);
            fragments.push(fragment);
        }

        if level >= ContextLevel::Dependency {
            let mut extra = Vec::new();
            for resolution in resolutions.iter().take(5) {
                let resolver = Resolver::new(self.store, self.config);
                if let Ok(related) = resolver.related(resolution.symbol.id.as_str()) {
                    for r in related.into_iter().take(5) {
                        let key = (
                            r.symbol.path.clone(),
                            r.symbol.span.start_line,
                            r.symbol.span.end_line,
                        );
                        if seen.contains(&key) {
                            continue;
                        }
                        seen.push(key);
                        extra.push(Fragment {
                            path: r.symbol.path.clone(),
                            symbol: r.symbol.qualified_name.clone(),
                            kind: r.symbol.kind.display().to_string(),
                            start_line: r.symbol.span.start_line,
                            end_line: r.symbol.span.end_line,
                            confidence: r.confidence.display().to_string(),
                            score: r.score * 0.5,
                            signature: r.symbol.signature.clone(),
                            doc: None,
                            text: None,
                            tokens: 0,
                        });
                    }
                }
            }
            for mut fragment in extra {
                fragment.tokens = fragment_tokens(&fragment);
                fragments.push(fragment);
            }
        }

        let mut total_tokens: usize = fragments.iter().map(|f| f.tokens).sum();
        let mut truncated = false;
        if total_tokens > budget {
            truncated = true;
            fragments.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            while total_tokens > budget && fragments.len() > 1 {
                let removed = fragments.pop();
                if let Some(removed) = removed {
                    total_tokens = total_tokens.saturating_sub(removed.tokens);
                }
            }
            fragments.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            notes.push(format!(
                "orçamento de {budget} tokens excedido: contexto truncado"
            ));
        }

        let bytes = fragments
            .iter()
            .map(|f| f.text.as_ref().map(|t| t.len()).unwrap_or(0))
            .sum();

        Ok(ContextBundle {
            level,
            fragments,
            tokens: total_tokens,
            bytes,
            truncated,
            notes,
            revision: self.store.revision()?,
        })
    }
}
