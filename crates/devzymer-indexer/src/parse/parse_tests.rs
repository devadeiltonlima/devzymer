#[cfg(test)]
mod tests {
    use super::super::extract_symbols;
    use devzymer_core::model::{Language, SymbolKind};

    fn ts(source: &str) -> Vec<devzymer_core::model::Symbol> {
        extract_symbols("src/hooks/x.ts", Language::TypeScript, source)
    }

    fn tsx(source: &str) -> Vec<devzymer_core::model::Symbol> {
        extract_symbols("src/hooks/x.tsx", Language::Tsx, source)
    }

    /// Finds the declaration itself, ignoring the export and import markers
    /// that carry the same name.
    fn find<'a>(
        symbols: &'a [devzymer_core::model::Symbol],
        name: &str,
    ) -> Option<&'a devzymer_core::model::Symbol> {
        symbols.iter().find(|s| {
            s.name == name && s.kind != SymbolKind::Export && s.kind != SymbolKind::Import
        })
    }

    #[test]
    fn exported_function_is_indexed() {
        let symbols = ts("export function useVoiceCall() {\n  return 1;\n}\n");
        let symbol = find(&symbols, "useVoiceCall").expect("missing symbol");
        assert_eq!(symbol.kind, SymbolKind::Function);
        assert_eq!(symbol.span.start_line, 1);
        assert!(symbol.exported);
    }

    #[test]
    fn exported_arrow_function_is_indexed() {
        let symbols = ts("export const useVoiceCall = () => {\n  return 1;\n};\n");
        let symbol = find(&symbols, "useVoiceCall").expect("missing symbol");
        assert_eq!(symbol.kind, SymbolKind::Variable);
        assert_eq!(symbol.span.start_line, 1);
    }

    #[test]
    fn function_expression_is_indexed() {
        let symbols = ts("const useVoiceCall = function () {\n  return 1;\n};\n");
        let symbol = find(&symbols, "useVoiceCall").expect("missing symbol");
        assert_eq!(symbol.kind, SymbolKind::Variable);
    }

    #[test]
    fn default_exported_function_is_indexed() {
        let symbols = ts("export default function useVoiceCall() {\n  return 1;\n}\n");
        let symbol = find(&symbols, "useVoiceCall").expect("missing symbol");
        assert_eq!(symbol.kind, SymbolKind::Function);
    }

    #[test]
    fn generic_names_do_not_carry_the_type_parameters() {
        let symbols = ts(
            "export function useLeituraVoiceCall<T>(acao: string): T {\n  return null as T;\n}\n",
        );
        let symbol = find(&symbols, "useLeituraVoiceCall").expect("missing symbol");
        assert_eq!(symbol.kind, SymbolKind::Function);
        assert!(symbol
            .signature
            .as_deref()
            .unwrap_or_default()
            .contains("useLeituraVoiceCall"));
        let exported = symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Export)
            .expect("missing export");
        assert_eq!(exported.name, "useLeituraVoiceCall");
    }

    #[test]
    fn async_functions_are_marked() {
        let symbols = ts("export async function useAsync() {\n  return 1;\n}\n");
        let symbol = find(&symbols, "useAsync").expect("missing symbol");
        assert!(symbol.is_async);
    }

    #[test]
    fn tsx_components_are_indexed() {
        let symbols = tsx("export function Painel(): JSX.Element {\n  return <div />;\n}\n");
        let symbol = find(&symbols, "Painel").expect("missing symbol");
        assert_eq!(symbol.kind, SymbolKind::Function);
    }

    #[test]
    fn classes_and_methods_are_indexed() {
        let symbols = ts("export class Servico {\n  executar(): void {}\n}\n");
        let class = find(&symbols, "Servico").expect("missing class");
        assert_eq!(class.kind, SymbolKind::Class);
        let method = find(&symbols, "executar").expect("missing method");
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.qualified_name, "Servico.executar");
    }

    #[test]
    fn imports_are_indexed_with_the_module_specifier() {
        let symbols = ts("import { useState } from 'react';\n");
        let import = symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Import)
            .expect("missing import");
        assert_eq!(import.name, "react");
    }
}
