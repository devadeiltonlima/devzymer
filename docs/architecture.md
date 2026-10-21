# Arquitetura

## Princípio

O modelo caro deve gastar tokens com decisão de engenharia e geração de código. Tudo que é
determinístico, repetitivo, pesquisável ou barato de decidir é resolvido localmente.

Consequência prática: o agente recebe `caminho:linha_inicial-linha_final | símbolo | origem` em
vez de arquivos inteiros, e recebe um aviso explícito quando a evidência é parcial, ambígua ou
potencialmente obsoleta.

## Visão geral

```
                 ┌──────────────┐
   agente ──────▶│  MCP (stdio) │──────▶ contexto + erros estruturados
                 └──────┬───────┘
                        │
       ┌────────────────┼─────────────────┬──────────────────┐
       ▼                ▼                 ▼                  ▼
   search           context          decisions            patch
  (cascata)     (orçamento de      (Jev ou local)     (atômico, revisado)
       │            tokens)               │
       └────────┬───────┘                 └── circuit breaker
                ▼
             store (SQLite + FTS5)
                ▲
        indexer │ (Tree-sitter)
                │
          repositório local
```

## Crates

### devzymer-core

Modelos de domínio (`Symbol`, `Relation`, `Entity`, `Span`, `Language`, `SymbolKind`), erros
com código estável, configuração em TOML com mesclagem usuário/projeto, detecção de workspace
(git, worktrees, repositórios aninhados, Claude Code, CodeGraph) e a camada de persistência
SQLite com migrações, FTS5 e triggers.

### devzymer-indexer

Varredura com respeito a `.gitignore`, `.ignore` e `.devzymerignore`; detecção de binários e
limites de tamanho; extração de símbolos por Tree-sitter (TypeScript, TSX, JavaScript, JSX,
Python, Rust) com fallback heurístico por regex nas demais linguagens; indexação incremental por
hash de conteúdo e metadados; commits em blocos para manter o pico de memória limitado.

Três módulos de manutenção:

- `index` — indexação completa, usada por `init` e por `context_sync --full`.
- `sync` — reindexação de um conjunto específico de arquivos e reconciliação do índice contra o
  disco. É o caminho incremental, sem nova varredura completa.
- `watch` — observador de filesystem que agrupa eventos e dispara a reindexação do que mudou.

### devzymer-graph

Constrói o grafo a partir do índice: `defines`, `imports`, `exports`, `calls`, `implements`,
`extends`, `tested_by`. Toda aresta carrega confiança (`confirmed`, `heuristic`, `unresolved`) e
evidência. A resolução de importações é específica por linguagem: relativa em TS/JS, por pacote
em Python, por `crate::`/`self::`/`super::` em Rust.

A construção por arquivo (`rebuild_file_relations`) permite manter o grafo consistente quando
apenas alguns arquivos mudaram, sem refazer o grafo inteiro.

### devzymer-search

Recuperação em cascata, do mais barato e determinístico ao mais caro:

1. caminho exato
2. nome do arquivo (stem indexado)
3. símbolo exato
4. símbolo qualificado (`Classe.metodo`)
5. correspondência por token da consulta
6. prefixo
7. busca lexical FTS5/BM25
8. expansão pelo grafo

Cada resultado carrega a origem da correspondência e a confiança, o que permite ao agente
distinguir um achado exato de uma aproximação lexical.

O ranqueamento é hierárquico por **força da evidência**:

| Nível | Origem da correspondência |
| --- | --- |
| 3 | caminho exato, símbolo exato, símbolo qualificado |
| 2 | nome do arquivo, com ou sem a dica do caminho |
| 1 | correspondência por token, prefixo, busca lexical |
| 0 | expansão pelo grafo |

Dentro do nível, decide o número de termos cobertos e depois a pontuação. Um símbolo nomeado
exatamente na consulta vence um arquivo que só coincide com uma palavra da consulta.

Um termo da consulta que nomeia um arquivo não é apenas um reforço: os símbolos desse arquivo
entram como candidatos, e o sinal é proporcional ao tamanho do nome. É o que permite
responder "onde está o hook useVoiceCall" devolvendo `src/hooks/useVoiceCall.ts` e seus símbolos,
e não os símbolos de um arquivo homônimo citado por acaso na frase.

O cache de recuperação é invalidado pela revisão do índice **e** pela versão do algoritmo. Sem
a versão, uma correção no ranqueamento continuaria servindo o resultado antigo, porque a
revisão do índice não muda quando só o código muda.

A busca lexical divide um identificador escrito em `snake_case` nas suas palavras. O índice
FTS5 guarda um nome em `camelCase` como um único token, então a consulta inteira viraria uma
frase que nada corresponde. Dividir faz `call_preparar_numero_de_teste` alcançar
`prepararGptLiveParaTeste`, que é o símbolo que o agente procurava. A divisão não se aplica a
consultas de caminho, que são respondidas com precisão pela etapa de arquivo.

### devzymer-context

Orçamento de tokens com divulgação progressiva em cinco níveis (localização, evidência,
implementação, dependência, expandido), deduplicação de trechos, revalidação contra a revisão
corrente do índice e cache de recuperação versionado por revisão.

### devzymer-decisions

Roteamento de decisão com três níveis: regras determinísticas locais primeiro, provedor externo
opcional (Jev, API System One da TypeSafe) quando a regra local não é decisiva, e escalonamento
para o agente quando o caso é aberto. Inclui cache de decisão, timeout, retentativa limitada e
circuit breaker. Sem credencial, o sistema continua utilizável.

### devzymer-mcp

Servidor JSON-RPC 2.0 por stdio com `initialize`, `tools/list`, `tools/call` e `ping`. As sete
ferramentas validam entrada, devolvem erros com código estável e mantêm as respostas curtas por
padrão. `context_patch` confere hash do arquivo, aplica a escrita de forma atômica e preserva
quebras de linha e codificação.

O servidor mantém o estado de frescor: último sincronismo, sincronizações em andamento, mudanças
pendentes, arquivos ignorados e falhas. O observador de filesystem vive no processo do servidor e
escreve em uma conexão própria do SQLite, em modo WAL, para não bloquear as leituras.

### devzymer-integrations

Adaptadores de instalação, um por agente, sobre o mesmo motor:

| Adaptador | Arquivo | Formato |
| --- | --- | --- |
| Claude Code | `.mcp.json`, `~/.claude.json`, `CLAUDE.md`, hooks | JSON e Markdown |
| Codex | `~/.codex/config.toml`, `<projeto>/.codex/config.toml`, `AGENTS.md` | TOML |
| OpenCode | `opencode.json(c)`, `AGENTS.md` | JSON com comentários |

Cada adaptador grava de forma aditiva: a configuração existente é lida, as entradas do devzymer
são identificadas, o arquivo é copiado para `.devzymer/backups/` e só o trecho afetado é
reescrito. O TOML usa `toml_edit` e o JSON com comentários usa um analisador posicional, para
que comentários e formatação sobrevivam.

A seleção de agentes, a detecção e a instalação compartilhada ficam em `agents.rs`. O estado de
cada agente distingue quatro situações: **conectado** (a configuração existe e o servidor
respondeu a um handshake real), **configurado** (a configuração aponta para o devzymer),
**detectado** (o executável existe) e **não instalado**.

### devzymer-tui

Interface de terminal da seleção de agentes, com `ratatui` e `crossterm`. A lógica da seleção
fica separada do desenho, em `state.rs`, e é testada sem terminal. O terminal volta ao estado
anterior em `Drop`, mesmo quando a execução falha.

Instalação aditiva e idempotente do servidor MCP, das regras de agente e dos hooks do Claude
Code, sempre com backup antes de modificar. Detecção de CodeGraph, plano de migração com
aprovação explícita e desinstalação que remove apenas o que o devzymer criou.

### devzymer-bench

Gera tarefas de benchmark a partir do índice vivo, mede precisão, latência, tokens de contexto e
tokens da linha de base, e emite relatório em JSON ou Markdown com a distinção explícita entre
medido e estimado.

### devzymer-cli

Executável `devzymer` com subcomandos, saída em JSON opcional e mensagens em português do Brasil.

## Dependências externas

- **SQLite (rusqlite, build embarcado):** persistência e FTS5.
- **Tree-sitter:** gramáticas compiladas dentro do binário, sem dependência de runtime.
- **ignore:** traversal com semântica de `.gitignore`.
- **notify:** observação de filesystem.
- **reqwest (rustls):** único cliente HTTP, usado apenas pelo provedor de decisão.
- **clap:** parsing de linha de comando.

O núcleo não depende de CodeGraph, Graphify, banco de vetores hospedado ou serviço de IA
externo.

## Índice compartilhado entre agentes

Os agentes não têm índices próprios. O índice vive em `<repositório>/.devzymer/index.sqlite3`
e é aberto em modo WAL, com `busy_timeout` de dez segundos, o que permite que vários processos
MCP leiam e escrevam no mesmo banco sem corrompê-lo.

A raiz do repositório é resolvida em cascata: o argumento `--root`, a variável
`DEVZYMER_PROJECT_DIR`, a `CLAUDE_PROJECT_DIR` do Claude Code e a busca do diretório de trabalho
até a raiz do projeto. O primeiro candidato que tem índice vence; sem índice, o servidor falha
em vez de responder com dados de outro repositório.

## Decisões de projeto

**Caminhamento da árvore por nós, não por queries.** A API de `Query` do Tree-sitter mudou entre
versões; o walked de nós é estável. O custo é um extrator por família de linguagem, o benefício é
ausência de retrabalho a cada atualização de dependência.

**Uma aresta por vez, com confiança explícita.** O grafo nunca afirma precisão que não tem. Uma
relação estabelecida por coincidência de nome é `heuristic`, não `confirmed`.

**Escrita atômica em toda parte.** Configuração, backup e patch usam arquivo temporário seguido
de renomeação, de modo que uma interrupção nunca deixa um arquivo pela metade.

**Regras locais antes de rede.** O provedor de decisão é opcional por construção: desligá-lo não
altera a capacidade de recuperação do sistema.
