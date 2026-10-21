use std::collections::{HashMap, HashSet, VecDeque};

use devzymer_core::model::{Confidence, Entity, Relation, RelationKind, Symbol, SymbolKind};
use devzymer_core::store::Store;
use devzymer_core::Result;
use regex::Regex;

const MAX_CALLS_PER_SYMBOL: usize = 48;
const MAX_RELATIONS_PER_FILE: usize = 8_000;
/// Upper bound of the distinct identifiers read from one body. A long
/// function calls many names, and the call that matters can be anywhere in
/// it, so the budget grows with the size of the body.
const MAX_IDENTIFIERS_PER_SYMBOL: usize = 2_000;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GraphReport {
    pub relations: usize,
    pub defines: usize,
    pub imports: usize,
    pub calls: usize,
    pub implements: usize,
    pub extends: usize,
    pub exports: usize,
    pub tested_by: usize,
    pub unresolved_imports: usize,
    pub duration_ms: u64,
}

/// Index of the symbols of a repository, used to resolve relation targets.
struct SymbolIndex<'a> {
    all: &'a [Symbol],
    by_file: HashMap<String, Vec<&'a Symbol>>,
    by_name: HashMap<String, Vec<&'a Symbol>>,
    paths: HashSet<String>,
}

impl<'a> SymbolIndex<'a> {
    fn new(all: &'a [Symbol], paths: HashSet<String>) -> Self {
        let mut by_file: HashMap<String, Vec<&'a Symbol>> = HashMap::new();
        let mut by_name: HashMap<String, Vec<&'a Symbol>> = HashMap::new();
        for symbol in all {
            if matches!(symbol.kind, SymbolKind::Import | SymbolKind::Export) {
                continue;
            }
            by_file.entry(symbol.path.clone()).or_default().push(symbol);
            by_name.entry(symbol.name.clone()).or_default().push(symbol);
        }
        Self {
            all,
            by_file,
            by_name,
            paths,
        }
    }

    fn of_file(&self, path: &str) -> Vec<&'a Symbol> {
        self.by_file.get(path).cloned().unwrap_or_default()
    }
}

/// Rebuilds the dependency graph from the current index contents.
pub fn build_graph(store: &Store) -> Result<GraphReport> {
    let started = std::time::Instant::now();
    let files = store.all_files()?;
    let symbols = store.all_symbols()?;
    let paths: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let index = SymbolIndex::new(&symbols, paths);

    let mut relations: Vec<Relation> = Vec::new();
    let mut report = GraphReport::default();

    for file in &files {
        let produced = file_relations(file, &index);
        report.defines += produced
            .iter()
            .filter(|r| r.kind == RelationKind::Defines)
            .count();
        report.imports += produced
            .iter()
            .filter(|r| r.kind == RelationKind::Imports && r.confidence == Confidence::Confirmed)
            .count();
        report.unresolved_imports += produced
            .iter()
            .filter(|r| r.kind == RelationKind::Imports && r.confidence == Confidence::Unresolved)
            .count();
        report.calls += produced
            .iter()
            .filter(|r| r.kind == RelationKind::Calls)
            .count();
        report.implements += produced
            .iter()
            .filter(|r| r.kind == RelationKind::Implements)
            .count();
        report.extends += produced
            .iter()
            .filter(|r| r.kind == RelationKind::Extends)
            .count();
        report.exports += produced
            .iter()
            .filter(|r| r.kind == RelationKind::Exports)
            .count();
        report.tested_by += produced
            .iter()
            .filter(|r| r.kind == RelationKind::TestedBy)
            .count();
        relations.extend(produced);
    }

    store.clear_relations()?;
    store.insert_relations(&relations)?;

    report.relations = relations.len();
    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

/// Recomputes the relations that belong to a single file, after the file was
/// created, changed or removed.
pub fn rebuild_file_relations(store: &Store, path: &str) -> Result<usize> {
    let symbols = store.all_symbols()?;
    let files = store.all_files()?;
    let paths: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let index = SymbolIndex::new(&symbols, paths);

    store.delete_relations_for_file(path)?;
    let Some(file) = files.iter().find(|f| f.path == path) else {
        return Ok(0);
    };
    let relations = file_relations(file, &index);
    let count = relations.len();
    store.insert_relations(&relations)?;
    Ok(count)
}

/// Computes every relation whose source belongs to the given file.
fn file_relations(file: &devzymer_core::store::FileRecord, index: &SymbolIndex) -> Vec<Relation> {
    let mut relations: Vec<Relation> = Vec::new();
    let file_symbols = index.of_file(&file.path);
    let file_key = Entity::File(file.path.clone());

    for symbol in &file_symbols {
        relations.push(Relation {
            src: file_key.clone(),
            dst: Entity::Symbol(symbol.id.clone()),
            kind: RelationKind::Defines,
            confidence: Confidence::Confirmed,
            evidence: Some(format!("{}:{}", symbol.path, symbol.span.start_line)),
            file: Some(file.path.clone()),
        });
    }

    for import in index
        .all
        .iter()
        .filter(|s| s.kind == SymbolKind::Import && s.path == file.path)
    {
        let specifier = import.name.trim();
        match resolve_import(specifier, &file.path, file.language, &index.paths) {
            Some(target) if target != file.path => relations.push(Relation {
                src: file_key.clone(),
                dst: Entity::File(target),
                kind: RelationKind::Imports,
                confidence: Confidence::Confirmed,
                evidence: Some(specifier.to_string()),
                file: Some(file.path.clone()),
            }),
            Some(_) => {}
            None => relations.push(Relation {
                src: file_key.clone(),
                dst: Entity::External(specifier.to_string()),
                kind: RelationKind::Imports,
                confidence: Confidence::Unresolved,
                evidence: Some(specifier.to_string()),
                file: Some(file.path.clone()),
            }),
        }
    }

    for symbol in &file_symbols {
        if symbol.kind != SymbolKind::Export {
            continue;
        }
        for name in symbol
            .name
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let candidates = index.by_name.get(name).cloned().unwrap_or_default();
            let target = candidates
                .iter()
                .find(|c| c.path == file.path)
                .copied()
                .or_else(|| candidates.first().copied());
            if let Some(target) = target {
                relations.push(Relation {
                    src: file_key.clone(),
                    dst: Entity::Symbol(target.id.clone()),
                    kind: RelationKind::Exports,
                    confidence: Confidence::Confirmed,
                    evidence: Some(name.to_string()),
                    file: Some(file.path.clone()),
                });
            }
        }
    }

    for symbol in &file_symbols {
        if !matches!(
            symbol.kind,
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
        ) {
            continue;
        }
        let Some(body) = symbol.body.as_deref() else {
            continue;
        };
        let mut added = 0usize;
        for ident in referenced_identifiers(body) {
            if added >= MAX_CALLS_PER_SYMBOL {
                break;
            }
            let Some(candidates) = index.by_name.get(&ident) else {
                continue;
            };
            for candidate in candidates {
                if candidate.id == symbol.id {
                    continue;
                }
                if !matches!(
                    candidate.kind,
                    SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
                ) {
                    continue;
                }
                if candidate.path == symbol.path && candidate.name == symbol.name {
                    continue;
                }
                relations.push(Relation {
                    src: Entity::Symbol(symbol.id.clone()),
                    dst: Entity::Symbol(candidate.id.clone()),
                    kind: RelationKind::Calls,
                    confidence: Confidence::Heuristic,
                    evidence: Some(ident.clone()),
                    file: Some(file.path.clone()),
                });
                added += 1;
                break;
            }
        }
    }

    for symbol in &file_symbols {
        if !matches!(
            symbol.kind,
            SymbolKind::Class | SymbolKind::Interface | SymbolKind::Struct | SymbolKind::Trait
        ) {
            continue;
        }
        let Some(body) = symbol.body.as_deref() else {
            continue;
        };
        for (kind, base) in heritage_bases(symbol.language, body) {
            let candidates = index.by_name.get(&base).cloned().unwrap_or_default();
            let Some(target) = candidates
                .iter()
                .find(|c| c.kind != symbol.kind || c.path != symbol.path)
                .copied()
            else {
                continue;
            };
            if target.id == symbol.id {
                continue;
            }
            relations.push(Relation {
                src: Entity::Symbol(symbol.id.clone()),
                dst: Entity::Symbol(target.id.clone()),
                kind,
                confidence: Confidence::Heuristic,
                evidence: Some(base.clone()),
                file: Some(file.path.clone()),
            });
        }
    }

    for relation in relations.clone() {
        if relation.kind != RelationKind::Calls {
            continue;
        }
        let (Entity::Symbol(src), Entity::Symbol(dst)) = (&relation.src, &relation.dst) else {
            continue;
        };
        let src_file = index
            .all
            .iter()
            .find(|s| &s.id == src)
            .map(|s| s.path.clone())
            .unwrap_or_default();
        let dst_file = index
            .all
            .iter()
            .find(|s| &s.id == dst)
            .map(|s| s.path.clone())
            .unwrap_or_default();
        if is_test_path(&src_file) && !is_test_path(&dst_file) {
            relations.push(Relation {
                src: Entity::Symbol(dst.clone()),
                dst: Entity::Symbol(src.clone()),
                kind: RelationKind::TestedBy,
                confidence: Confidence::Heuristic,
                evidence: Some(src_file.clone()),
                file: Some(src_file.clone()),
            });
        }
    }

    relations.truncate(MAX_RELATIONS_PER_FILE);
    relations
}

/// Extracts the base types referenced by a class or trait definition.
fn heritage_bases(
    language: devzymer_core::model::Language,
    body: &str,
) -> Vec<(RelationKind, String)> {
    let mut out = Vec::new();
    let header: String = body.chars().take(2000).collect();
    match language {
        devzymer_core::model::Language::Python => {
            if let Ok(re) = Regex::new(r"class\s+\w+\s*\(([^)]*)\)") {
                if let Some(caps) = re.captures(&header) {
                    for base in caps[1].split(',') {
                        let base = base.split('=').next().unwrap_or("").trim();
                        let base = base.rsplit('.').next().unwrap_or(base).trim();
                        if !base.is_empty() && base != "object" {
                            out.push((RelationKind::Extends, base.to_string()));
                        }
                    }
                }
            }
        }
        devzymer_core::model::Language::Rust => {
            if let Ok(re) =
                Regex::new(r"impl(?:<[^>]*>)?\s+([A-Za-z_][\w:<>]*)\s+for\s+([A-Za-z_][\w:<>]*)")
            {
                if let Some(caps) = re.captures(&header) {
                    out.push((RelationKind::Implements, caps[1].to_string()));
                    out.push((RelationKind::Implements, caps[2].to_string()));
                }
            }
        }
        _ => {
            if let Ok(re) = Regex::new(r"extends\s+([A-Za-z_$][\w$.]*)") {
                if let Some(caps) = re.captures(&header) {
                    out.push((RelationKind::Extends, caps[1].to_string()));
                }
            }
            if let Ok(re) = Regex::new(r"implements\s+([^{]+)") {
                if let Some(caps) = re.captures(&header) {
                    for base in caps[1].split(',') {
                        let base = base.trim();
                        if !base.is_empty() {
                            out.push((RelationKind::Implements, base.to_string()));
                        }
                    }
                }
            }
        }
    }
    out.retain(|(_, base)| !base.is_empty());
    out
}

pub fn is_test_path(path: &str) -> bool {
    devzymer_core::util::is_test_path(path)
}

/// Resolves an import specifier to a repository file when possible.
pub fn resolve_import(
    specifier: &str,
    from_path: &str,
    language: devzymer_core::model::Language,
    path_set: &HashSet<String>,
) -> Option<String> {
    let spec = specifier.trim();
    if spec.is_empty() {
        return None;
    }
    let from_dir = std::path::Path::new(from_path)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    let candidates: Vec<String> = match language {
        devzymer_core::model::Language::TypeScript
        | devzymer_core::model::Language::Tsx
        | devzymer_core::model::Language::JavaScript
        | devzymer_core::model::Language::Jsx => {
            if !spec.starts_with('.') {
                return None;
            }
            let base = join_relative(&from_dir, spec);
            let stem = strip_known_extension(&base);
            let mut cands = vec![base.clone()];
            for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
                cands.push(format!("{stem}.{ext}"));
                cands.push(format!("{stem}/index.{ext}"));
            }
            cands
        }
        devzymer_core::model::Language::Python => {
            if spec.starts_with('.') {
                let level = spec.chars().take_while(|c| *c == '.').count();
                let rest = &spec[level..];
                let mut dir = from_dir.clone();
                for _ in 1..level {
                    dir = std::path::Path::new(&dir)
                        .parent()
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                }
                let base = if rest.is_empty() {
                    dir
                } else {
                    join_relative(&dir, &rest.replace('.', "/"))
                };
                vec![format!("{base}.py"), format!("{base}/__init__.py")]
            } else {
                let rel = spec.replace('.', "/");
                vec![format!("{rel}.py"), format!("{rel}/__init__.py")]
            }
        }
        devzymer_core::model::Language::Rust => {
            let parts: Vec<&str> = spec.split("::").collect();
            if parts.is_empty() {
                return None;
            }
            let (prefix, rest) = match parts[0] {
                "crate" => ("src".to_string(), parts[1..].to_vec()),
                "self" => (from_dir.clone(), parts[1..].to_vec()),
                "super" => (
                    std::path::Path::new(&from_dir)
                        .parent()
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default(),
                    parts[1..].to_vec(),
                ),
                _ => return None,
            };
            if rest.is_empty() {
                return None;
            }
            let base = if prefix.is_empty() {
                rest.join("/")
            } else {
                format!("{prefix}/{}", rest.join("/"))
            };
            vec![format!("{base}.rs"), format!("{base}/mod.rs")]
        }
        _ => return None,
    };

    candidates.into_iter().find(|c| path_set.contains(c))
}

fn join_relative(dir: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches("./");
    if dir.is_empty() {
        rel.to_string()
    } else {
        format!("{dir}/{rel}")
    }
}

/// Removes a known JavaScript or TypeScript extension from a module path.
fn strip_known_extension(path: &str) -> String {
    for ext in [".d.ts", ".tsx", ".jsx", ".mjs", ".cjs", ".ts", ".js"] {
        if let Some(stem) = path.strip_suffix(ext) {
            return stem.to_string();
        }
    }
    path.to_string()
}

/// Extracts the identifiers referenced by a body, used for call edges.
pub fn referenced_identifiers(body: &str) -> Vec<String> {
    let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("static regex");
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for m in re.find_iter(body) {
        let text = m.as_str();
        if text.len() > 2 && !text.starts_with("__") {
            // A large body mentions the same name many times. Counting it
            // once keeps the whole body visible: the call that matters can
            // be at its end.
            if seen.insert(text.to_string()) {
                out.push(text.to_string());
            }
        }
        if out.len() >= MAX_IDENTIFIERS_PER_SYMBOL {
            break;
        }
    }
    out
}

/// Breadth first search over the relation table.
pub fn shortest_path(store: &Store, from: &Entity, to: &Entity) -> Result<Vec<Relation>> {
    let all = load_relations(store)?;
    let mut adjacency: HashMap<String, Vec<(String, &Relation)>> = HashMap::new();
    for r in &all {
        adjacency
            .entry(r.src.key())
            .or_default()
            .push((r.dst.key(), r));
    }

    let start = from.key();
    let goal = to.key();
    if start == goal {
        return Ok(Vec::new());
    }
    let mut queue: VecDeque<String> = VecDeque::from([start.clone()]);
    let mut visited: HashSet<String> = HashSet::from([start.clone()]);
    let mut previous: HashMap<String, (String, Relation)> = HashMap::new();

    while let Some(node) = queue.pop_front() {
        for (next, relation) in adjacency.get(&node).into_iter().flatten() {
            if !visited.insert(next.clone()) {
                continue;
            }
            previous.insert(next.clone(), (node.clone(), (*relation).clone()));
            if *next == goal {
                let mut path = Vec::new();
                let mut cursor = goal.clone();
                while cursor != start {
                    let Some((parent, rel)) = previous.get(&cursor) else {
                        break;
                    };
                    path.push(rel.clone());
                    cursor = parent.clone();
                }
                path.reverse();
                return Ok(path);
            }
            queue.push_back(next.clone());
        }
    }
    Ok(Vec::new())
}

pub fn load_relations(store: &Store) -> Result<Vec<Relation>> {
    let mut all = Vec::new();
    for file in store.all_files()? {
        all.extend(store.relations_from(&Entity::File(file.path).key())?);
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn relative_javascript_imports_resolve_to_the_source_file() {
        let set = paths(&["src/auth/session.ts", "src/auth/index.ts"]);
        let resolved = resolve_import(
            "./session",
            "src/auth/routes.ts",
            devzymer_core::model::Language::TypeScript,
            &set,
        );
        assert_eq!(resolved.as_deref(), Some("src/auth/session.ts"));
    }

    #[test]
    fn javascript_imports_with_an_explicit_extension_resolve() {
        let set = paths(&["src/parser.js"]);
        let resolved = resolve_import(
            "./parser.js",
            "src/app.js",
            devzymer_core::model::Language::JavaScript,
            &set,
        );
        assert_eq!(resolved.as_deref(), Some("src/parser.js"));
    }

    #[test]
    fn package_imports_stay_external() {
        let set = paths(&["src/app.js"]);
        assert!(resolve_import(
            "react",
            "src/app.js",
            devzymer_core::model::Language::JavaScript,
            &set
        )
        .is_none());
    }

    #[test]
    fn python_packages_resolve_to_modules() {
        let set = paths(&["app/models/user.py"]);
        let resolved = resolve_import(
            "app.models.user",
            "app/views.py",
            devzymer_core::model::Language::Python,
            &set,
        );
        assert_eq!(resolved.as_deref(), Some("app/models/user.py"));
    }

    #[test]
    fn rust_crate_paths_resolve_to_modules() {
        let set = paths(&["src/core/model.rs"]);
        let resolved = resolve_import(
            "crate::core::model",
            "src/lib.rs",
            devzymer_core::model::Language::Rust,
            &set,
        );
        assert_eq!(resolved.as_deref(), Some("src/core/model.rs"));
    }

    #[test]
    fn test_paths_are_recognised() {
        assert!(is_test_path("src/foo.test.ts"));
        assert!(is_test_path("src/__tests__/foo.ts"));
        assert!(is_test_path("tests/test_foo.py"));
        assert!(!is_test_path("src/foo.ts"));
    }
}
