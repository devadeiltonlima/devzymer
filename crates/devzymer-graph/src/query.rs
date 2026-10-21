use std::collections::{HashMap, HashSet, VecDeque};

use devzymer_core::model::{Entity, Relation, RelationKind, Symbol, SymbolId};
use devzymer_core::store::Store;
use devzymer_core::Result;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImpactReport {
    pub symbols: Vec<Symbol>,
    pub relations: Vec<Relation>,
    pub depth_reached: usize,
    pub truncated: bool,
}

pub struct GraphQuery<'a> {
    store: &'a Store,
}

impl<'a> GraphQuery<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Symbols that directly reference the given symbol.
    pub fn callers(&self, symbol: &SymbolId) -> Result<Vec<Relation>> {
        let incoming = self
            .store
            .relations_to(&Entity::Symbol(symbol.clone()).key())?;
        Ok(incoming
            .into_iter()
            .filter(|r| {
                matches!(
                    r.kind,
                    RelationKind::Calls | RelationKind::References | RelationKind::TestedBy
                )
            })
            .collect())
    }

    /// Symbols directly referenced by the given symbol.
    pub fn callees(&self, symbol: &SymbolId) -> Result<Vec<Relation>> {
        let outgoing = self
            .store
            .relations_from(&Entity::Symbol(symbol.clone()).key())?;
        Ok(outgoing
            .into_iter()
            .filter(|r| {
                matches!(
                    r.kind,
                    RelationKind::Calls
                        | RelationKind::References
                        | RelationKind::Implements
                        | RelationKind::Extends
                )
            })
            .collect())
    }

    /// Files imported by the given file.
    pub fn dependencies(&self, file: &str) -> Result<Vec<Relation>> {
        let relations = self
            .store
            .relations_from(&Entity::File(file.to_string()).key())?;
        Ok(relations
            .into_iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .collect())
    }

    /// Files that import the given file.
    pub fn dependents(&self, file: &str) -> Result<Vec<Relation>> {
        let relations = self
            .store
            .relations_to(&Entity::File(file.to_string()).key())?;
        Ok(relations
            .into_iter()
            .filter(|r| r.kind == RelationKind::Imports)
            .collect())
    }

    /// Symbols defined in a file.
    pub fn defines(&self, file: &str) -> Result<Vec<Relation>> {
        let relations = self
            .store
            .relations_from(&Entity::File(file.to_string()).key())?;
        Ok(relations
            .into_iter()
            .filter(|r| r.kind == RelationKind::Defines)
            .collect())
    }

    /// Reverse dependency closure of a symbol: everything that can be affected
    /// by changing it.
    pub fn impact(&self, symbol: &SymbolId, depth: usize) -> Result<ImpactReport> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut relations: Vec<Relation> = Vec::new();
        let mut ids: Vec<SymbolId> = Vec::new();
        let mut queue: VecDeque<(SymbolId, usize)> = VecDeque::from([(symbol.clone(), 0)]);
        visited.insert(symbol.0.clone());
        let mut truncated = false;

        while let Some((current, level)) = queue.pop_front() {
            if level >= depth {
                continue;
            }
            for relation in self.callers(&current)? {
                if let Entity::Symbol(src) = &relation.src {
                    if visited.insert(src.0.clone()) {
                        ids.push(src.clone());
                        queue.push_back((src.clone(), level + 1));
                    }
                }
                relations.push(relation);
            }
            if relations.len() > 5_000 {
                truncated = true;
                break;
            }
        }

        let mut symbols = Vec::new();
        for id in &ids {
            if let Some(s) = self.store.symbol_by_id(id.as_str())? {
                symbols.push(s);
            }
        }
        let depth_reached = ids
            .iter()
            .filter_map(|id| self.store.symbol_by_id(id.as_str()).ok().flatten())
            .count();

        Ok(ImpactReport {
            symbols,
            relations,
            depth_reached,
            truncated,
        })
    }

    /// Shortest dependency path between two entities.
    pub fn shortest_path(&self, from: &Entity, to: &Entity) -> Result<Vec<Relation>> {
        crate::build::shortest_path(self.store, from, to)
    }

    /// Maps every relation to the symbol it points at, for reporting.
    pub fn resolve_entities(&self, relations: &[Relation]) -> Result<HashMap<String, Symbol>> {
        let mut out = HashMap::new();
        for r in relations {
            if let Entity::Symbol(id) = &r.dst {
                if let Some(s) = self.store.symbol_by_id(id.as_str())? {
                    out.insert(id.0.clone(), s);
                }
            }
        }
        Ok(out)
    }
}
