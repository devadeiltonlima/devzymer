import pathlib

TARGET = pathlib.Path("crates/devzymer-search/src/resolver.rs")

text = TARGET.read_text(encoding="utf-8")
old = """        // A term of the query may name a file, which hints at the candidates.
        for token in query_tokens(trimmed) {
            let stem = devzymer_core::store::stem_of(&token);
            if stem.len() < 3 {
                continue;
            }
            for file in self.store.files_by_stem(&stem)? {"""
new = """        // A term of the query may name a file, which hints at the candidates.
        for token in query_tokens(trimmed) {
            let stem = devzymer_core::store::stem_of(&token);
            if stem.len() < 3 {
                continue;
            }
            if std::env::var("DEVZYMER_DEBUG").is_ok() {
                eprintln!("token={token:?} stem={stem:?}");
            }
            for file in self.store.files_by_stem(&stem)? {"""
if text.count(old) != 1:
    print(f"padrao encontrado {text.count(old)} vezes")
    raise SystemExit(1)
TARGET.write_text(text.replace(old, new), encoding="utf-8", newline="\n")
print("debug ligado")
