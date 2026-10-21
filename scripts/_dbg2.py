import pathlib

RESOLVER = pathlib.Path("crates/devzymer-search/src/resolver.rs")
text = RESOLVER.read_text(encoding="utf-8")
old = """            let lexical = self.store.fts_search(trimmed, candidate_limit)?;"""
new = """            let lexical = self.store.fts_search(trimmed, candidate_limit)?;
            if std::env::var("DEVZYMER_DEBUG").is_ok() {
                eprintln!("[lexical] query={trimmed:?} hits={} limit={}", lexical.len(), candidate_limit);
            }"""
assert text.count(old) == 1
text = text.replace(old, new)

old = """                    let normalized = self.ranker.normalize_bm25(score);
                    if normalized < self.config.search.min_bm25_score {
                        continue;
                    }"""
new = """                    let normalized = self.ranker.normalize_bm25(score);
                    if std::env::var("DEVZYMER_DEBUG").is_ok() {
                        eprintln!("[lexical] id={id} bm25={score:.3} norm={normalized:.4} min={}", self.config.search.min_bm25_score);
                    }
                    if normalized < self.config.search.min_bm25_score {
                        continue;
                    }"""
assert text.count(old) == 1
RESOLVER.write_text(text.replace(old, new), encoding="utf-8", newline="\n")
print("debug ligado")
