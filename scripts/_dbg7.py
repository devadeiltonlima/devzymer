import pathlib

STORE = pathlib.Path("crates/devzymer-core/src/store.rs")
text = STORE.read_text(encoding="utf-8")
old = """        let match_query = build_match_query(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }"""
new = """        let match_query = build_match_query(query);
        if std::env::var("DEVZYMER_DEBUG").is_ok() {
            eprintln!("[fts] query={query:?} match={match_query:?}");
        }
        if match_query.is_empty() {
            return Ok(Vec::new());
        }"""
assert text.count(old) == 1
STORE.write_text(text.replace(old, new), encoding="utf-8", newline="\n")
print("debug do fts ligado")
