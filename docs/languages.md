# Matriz de linguagens

## Suporte com gramática Tree-sitter

Estas linguagens têm parser compilado dentro do binário. A extração é estrutural: percorre a
árvore sintática e reconhece funções, métodos, classes, interfaces, tipos, enums, traits,
módulos, constantes, campos, imports e exports com posições exatas em bytes e linhas.

| Linguagem | Extensões | Extrai | Observações |
| --- | --- | --- | --- |
| TypeScript | `.ts`, `.mts`, `.cts` | funções, classes, interfaces, enums, aliases de tipo, métodos, campos, variáveis, imports, exports | Inclui `export default`, `export const` e `export { a, b }` |
| TSX | `.tsx` | idem TypeScript | Mesma gramática, com suporte a JSX |
| JavaScript | `.js`, `.mjs`, `.cjs` | funções, classes, métodos, campos, variáveis, imports, exports | Inclui `require()` como aresta de importação |
| JSX | `.jsx` | idem JavaScript | |
| Python | `.py`, `.pyi`, `.pyw` | funções, classes, docstrings, imports | Decoradores entram na assinatura; docstring vira documentação do símbolo |
| Rust | `.rs` | funções, structs, enums, traits, módulos, constantes, tipos, macros, `impl` e `use` | `impl` define o contêiner dos métodos; `use` é resolvido por `crate::`, `self::` e `super::` |

## Suporte por fallback heurístico

Estas linguagens não têm gramática compilada, mas recebem indexação útil por regras de
expressão regular. As posições são exatas (arquivo, linha e coluna), mas a estrutura semântica
é aproximada: a confiança das correspondências é `heuristic`.

| Linguagem | Extensões | Reconhece |
| --- | --- | --- |
| Go | `.go` | funções, métodos, structs, interfaces, constantes |
| Java | `.java` | classes, interfaces, enums, records, métodos |
| Kotlin | `.kt`, `.kts` | classes, interfaces, enums, funções |
| C# | `.cs` | classes, structs, interfaces, enums, métodos |
| C | `.c`, `.h` | funções, structs, classes |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp` | funções, structs, classes |
| Ruby | `.rb` | métodos, classes, módulos |
| PHP | `.php` | funções, classes, interfaces, traits |
| Swift | `.swift` | funções, classes, structs, enums, protocolos, extensões |

## Sem suporte

Arquivos em linguagens sem parser e sem regra de fallback são ignorados na extração de símbolos.
A varredura e a indexação de metadados continuam funcionando, e o arquivo permanece visível em
`devzymer status` sob a linguagem `unknown`.

A ausência de parser é detectável: `devzymer status` lista a contagem por linguagem.

## Resolução de importações

| Linguagem | Forma | Resolução |
| --- | --- | --- |
| TypeScript, TSX, JavaScript, JSX | `./modulo` | Relativa ao diretório do arquivo, com troca de extensão e índice de diretório |
| TypeScript, JavaScript | `pacote` | Externa, marcada como não resolvida |
| Python | `modulo.submodulo` | `modulo/submodulo.py` e `modulo/submodulo/__init__.py` |
| Python | `.modulo` (relativa) | Relativa ao pacote de origem, conforme o número de pontos |
| Rust | `crate::a::b` | `src/a/b.rs` e `src/a/b/mod.rs` |
| Rust | `self::a`, `super::a` | Relativa ao módulo de origem |
| Rust | `outro_crate::a` | Externa, marcada como não resolvida |

## Cobertura de símbolos

| Conceito | TypeScript | JavaScript | Python | Rust |
| --- | --- | --- | --- | --- |
| Função | sim | sim | sim | sim |
| Método | sim | sim | não | sim (dentro de `impl` ou `trait`) |
| Classe | sim | sim | sim | `struct` |
| Interface | sim | não | não | `trait` |
| Enum | sim | não | não | sim |
| Alias de tipo | sim | não | não | sim |
| Constante | não | não | não | sim |
| Campo | sim | sim | não | não |
| Variável local | sim | sim | não | não |
| Módulo | sim (`namespace`) | não | não | sim |
| Macro | não | não | não | sim |
| Importação | sim | sim | sim | sim |
| Exportação | sim | sim | não | não |
| Docstring | não | não | sim | comentário de documentação |

## Expansão do suporte

O registro de parsers é versionado. Para adicionar uma linguagem:

1. incluir a gramática no workspace, com a dependência correspondente;
2. mapear a linguagem em `devzymer-core::model::Language`;
3. implementar a extração em `devzymer-indexer`, seguindo o padrão de caminhamento de nós;
4. acrescentar as regras de resolução de importação em `devzymer-graph`;
5. registrar a versão do parser em `devzymer-core::store`, que força a reindexação ao mudar.
