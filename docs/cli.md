# Linha de comando

Todas as opções globais:

| Opção | Efeito |
| --- | --- |
| `--root <DIR>` | Raiz do workspace a operar. Sem ela, sobe a árvore procurando `.git` ou `.devzymer` |
| `--json` | Saída em JSON, para consumo por script |
| `-v`, `--verbose` | Log em nível `debug` |

As mensagens de erro são escritas em português do Brasil. Em modo `--json`, o corpo da mensagem
muda para inglês técnico, mantendo os códigos de erro estáveis.

---

## `devzymer init`

Detecta o ambiente, indexa o repositório, constrói o grafo, instala a integração com os agentes
escolhidos e verifica o resultado.

```bash
devzymer init                 # abre a interface de seleção de agentes
devzymer init --dry-run       # só detecta, não escreve nada
devzymer init --full          # reindexa tudo, ignorando o cache incremental
devzymer init --no-integrate  # indexa sem tocar em .mcp.json, CLAUDE.md ou settings
devzymer init --scope user    # registra o servidor em todos os projetos e hosts
devzymer init --agents claude,codex,opencode   # escolha pela linha de comando
devzymer init --yes           # não abre a interface interativa
```

`--agents` aceita `claude`, `codex` e `opencode`, separados por vírgula. Sem terminal
interativo, ou com `--yes`, o comando integra apenas o Claude Code e avisa sobre a alternativa.
A interface e as opções de escopo estão em
[docs/installation.md](installation.md).

**Escopos de instalação**

| Escopo | Onde grava | Quando usar |
| --- | --- | --- |
| `project` | `.mcp.json` do projeto | Padrão. Compartilhado com a equipe pelo controle de versão |
| `local` | `~/.claude.json`, na chave do projeto | Só nesta máquina, só neste projeto |
| `user` | `~/.claude.json`, na raiz | Em todos os projetos, inclusive na extensão do VS Code, sem aprovação por projeto |

Etapas:

1. **Descoberta** — sistema operacional, raiz do repositório, worktrees e repositórios aninhados,
   linguagens, gerenciadores de pacote, configuração existente do Claude Code, integrações MCP
   existentes, instalações do CodeGraph.
2. **Varredura** — respeita `.gitignore`, `.ignore` e `.devzymerignore`; ignora binários,
   arquivos acima do limite de tamanho e artefatos de build. Links simbólicos não saem do
   workspace.
3. **Análise estrutural** — extração de símbolos por Tree-sitter, com fallback heurístico nas
   linguagens sem gramática.
4. **Análise semântica** — resolução de importações e arestas do grafo.
5. **Construção do índice** — arquivos, caminhos, símbolos, assinaturas, conteúdo lexical,
   relações, localizações, hashes e revisão.
6. **Persistência** — SQLite transacional com migrações e FTS5.
7. **Integração** — servidor MCP, regras de agente, hooks, permissão das ferramentas e proteção
   do índice no `.gitignore`, de forma aditiva e idempotente.
8. **Verificação** — busca real, integridade do banco e handshake MCP executado de verdade.

O índice local é um arquivo SQLite específico da máquina. Em um repositório git, o `init`
acrescenta `.devzymer/` ao `.gitignore`, para que ele não entre no histórico. A entrada não é
duplicada em execuções seguintes, e projetos fora de um repositório não recebem arquivo.

Saída resumida:

```
implementado: índice criado em .\.devzymer\index.sqlite3
  arquivos: 85 indexados, 0 inalterados, 0 removidos, 1 ignorados
  símbolos: 802
  grafo: 2285 relações (1483 chamadas, 0 importações)
validado: busca=true | banco=true | mcp=true
```

`--dry-run` não escreve nada e serve para inspecionar o que seria indexado.

---

## `devzymer integrations`

Gerencia as integrações com os agentes de código.

```bash
devzymer integrations            # lista os agentes e o estado
devzymer integrations add codex  # integra mais um agente
devzymer integrations remove codex
devzymer integrations repair     # reaplica e verifica o que está instalado
devzymer integrations verify     # handshake real com o servidor MCP
devzymer integrations scope codex user
```

Adicionar um agente não exige recriar o índice. Remover exige uma ação explícita: desmarcar um
agente na interface apenas não o modifica.

## `devzymer status`

Mostra cobertura, atualidade e estado da integração.

```bash
devzymer status
devzymer status --json
```

```
raiz: .
índice: 65 arquivos, 712 símbolos, 1873 relações, revisão 3
idiomas: rust=53, unknown=12
atualidade: 0 arquivo(s) alterado(s) desde o último índice
integração MCP: instalada | Claude Code CLI: presente
CodeGraph: detectado | decisões externas: desabilitadas
```

---

## `devzymer sync`

Reindexa o que mudou. Compara hash de conteúdo e metadados, reparseia os arquivos alterados e
reconstrói o grafo.

```bash
devzymer sync
devzymer sync --full        # reindexa tudo
devzymer sync --no-graph    # não reconstrói o grafo
```

A sincronização incremental é o caminho normal. A reconstrução completa só é necessária quando o
banco está corrompido, a versão do parser mudou ou a configuração de exclusões foi alterada de
forma incompatível.

**O comando não é necessário durante uma sessão comum.** O servidor MCP mantém o índice
sincronizado sozinho, por observador de filesystem e por reconciliação na abertura da sessão.
`devzymer sync` continua disponível para recuperação explícita.

---

## `devzymer doctor`

Diagnostica instalação, banco, parsers e integrações. Sai com código 1 se alguma verificação
falhar.

```
[ok] workspace: .
[aviso] git: repositório git não detectado
[ok] banco: ok | 65 arquivos, 712 símbolos, 1873 relações | revisão 3
[ok] fts5: busca lexical disponível
[ok] frescor: 0 arquivo(s) alterado(s) desde o último índice
[ok] claude-code: CLI presente no PATH
[ok] mcp: .\.mcp.json
[aviso] codegraph: detectado: C:\Users\<user>\.claude.json
[ok] decisões: desabilitadas; o roteamento usa apenas regras locais
```

---

## `devzymer config`

```bash
devzymer config list                      # configuração efetiva
devzymer config get index.max_file_size_bytes
devzymer config set decisions.enabled true
devzymer config set index.exclude "node_modules,dist,target"
devzymer config path                      # caminhos dos arquivos de configuração
```

A configuração é resolvida em três camadas: padrões embutidos, arquivo do usuário
(`~/.config/devzymer/config.toml`) e arquivo do projeto (`.devzymer/config.toml`). O projeto
sobrescreve o usuário. Referência completa em [configuration.md](configuration.md).

---

## `devzymer benchmark`

```bash
devzymer benchmark
devzymer benchmark --tasks 20 --format markdown
devzymer benchmark --output relatorio.json --format json
```

Gera tarefas a partir do índice vivo e mede precisão, latência e economia de tokens. Metodologia
em [benchmarks.md](benchmarks.md).

---

## `devzymer migrate`

Inspeciona e executa a migração a partir de outras ferramentas de indexação.

```bash
devzymer migrate --dry-run          # só o plano
devzymer migrate --confirm          # aplica
devzymer migrate --confirm --scope user
```

Sem `--confirm` ou com `--dry-run`, nada é alterado. Detalhes em [migration.md](migration.md).

---

## `devzymer uninstall`

Remove apenas o que o devzymer instalou: a entrada do servidor MCP, o bloco de regras no
`CLAUDE.md` e os hooks. Preserva todos os outros servidores, hooks e regras.

```bash
devzymer uninstall
devzymer uninstall --purge           # remove também o índice local
devzymer uninstall --scope user
```

---

## `devzymer serve`

Executa o servidor MCP por stdio. É o que o Claude Code invoca.

```bash
devzymer serve --root /caminho/do/projeto
```

Protocolo: JSON-RPC 2.0, uma mensagem por linha, Methods `initialize`, `notifications/initialized`,
`tools/list`, `tools/call` e `ping`.

---

## `devzymer watch`

Observa o filesystem e sincroniza o índice após cada rajada de alterações (debounce de 800 ms).

```bash
devzymer watch
```

---

## `devzymer hook`

Executa um manipulador de hook do Claude Code. Lê JSON do `stdin` e escreve JSON no `stdout`.
Não é um comando de uso direto: o Claude Code o invoca.

```bash
devzymer hook post-tool-use      # comprime saída grande de ferramenta
devzymer hook guard-test-files   # bloqueia criação de arquivos de teste sem aprovação
```

---

## `devzymer resolve`

Busca local no índice, sem passar pelo MCP. Útil para depuração e para scripting.

```bash
devzymer resolve "validateSession"
devzymer resolve "content hash" --limit 5
devzymer resolve --json "validateSession"
```

```
crates/devzymer-indexer/src/scan.rs:273-275 | content_hash | function | confirmed
```
