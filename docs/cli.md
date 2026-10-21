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

Detecta o ambiente, indexa o repositório, constrói o grafo e **verifica o índice**. Só depois
disso prepara os arquivos do agente no projeto (instruções e skill) e registra o servidor MCP nos
agentes escolhidos. Pode ser repetido sem efeito colateral.

```bash
devzymer init                 # abre a interface de seleção de agentes
devzymer init --dry-run       # só detecta e mostra o plano, não escreve nada
devzymer init --root ./projeto   # raiz explícita
devzymer init --agents claude,codex,opencode   # escolha pela linha de comando
devzymer init --yes           # não abre a interface interativa
devzymer init --full          # reindexa tudo, ignorando o cache incremental
devzymer init --no-agent-setup   # não grava CLAUDE.md, AGENTS.md nem a skill
devzymer init --no-integrate  # indexa sem registrar agentes e sem arquivos do agente
devzymer init --repair        # reaplica os arquivos do agente; guarda um índice danificado e reconstrói
devzymer init --scope user    # registra o servidor em todos os projetos e hosts
```

| Opção | Efeito |
| --- | --- |
| `--agents <lista>` | `claude`, `codex` e `opencode`, separados por vírgula. Define onde o servidor MCP é registrado **e** quais arquivos do projeto são preparados |
| `--no-agent-setup` | Não grava `CLAUDE.md`, `AGENTS.md` nem a skill. O registro do servidor, a permissão e os hooks seguem as outras opções |
| `--no-integrate` | Não registra agentes e não grava nada para eles, arquivos do projeto incluídos |
| `--repair` | Autoriza guardar ao lado (sem apagar) um índice danificado ou de outro workspace e reconstruí-lo; reaplica os arquivos do agente |
| `--dry-run` | Mostra a detecção e o plano dos arquivos do agente, sem gravar |

Sem `--agents` e sem terminal interativo (ou com `--yes`), o servidor MCP é registrado apenas no
Claude Code, como sempre, e os arquivos do projeto cobrem os três agentes. A interface e as
opções de escopo estão em [docs/installation.md](installation.md).

**Raiz do projeto.** O `init` nunca escolhe a pasta do usuário (nem um ancestral dela, nem a raiz
de um disco) por descoberta automática: nesses casos ele recusa e pede `--root <pasta>`
explícito. Um diretório sem marcadores de projeto continua aceito quando é onde você está.

**Escopos de instalação**

| Escopo | Onde grava | Quando usar |
| --- | --- | --- |
| `project` | `.mcp.json` do projeto | Padrão. Compartilhado com a equipe pelo controle de versão |
| `local` | `~/.claude.json`, na chave do projeto | Só nesta máquina, só neste projeto |
| `user` | `~/.claude.json`, na raiz | Em todos os projetos, inclusive na extensão do VS Code, sem aprovação por projeto |

Os arquivos do agente são **sempre locais ao projeto**. Nada em `~/.claude`, `~/.codex` ou na
configuração global do OpenCode é alterado sem `--scope user`.

Etapas, nesta ordem:

1. **Descoberta** — sistema operacional, raiz do repositório, worktrees e repositórios aninhados,
   linguagens, gerenciadores de pacote, configuração existente do Claude Code, integrações MCP
   existentes, instalações do CodeGraph.
2. **Inspeção do índice existente** — um índice que não abre, falha na verificação do SQLite,
   tem o índice lexical FTS5 inconsistente ou pertence a outro workspace **não é sobrescrito**:
   o comando para com uma mensagem e nada é alterado. `--repair` guarda o arquivo ao lado
   (`index.sqlite3.corrupt-<carimbo>`) e reconstrói.
3. **Varredura** — respeita `.gitignore`, `.ignore` e `.devzymerignore`; ignora binários,
   arquivos acima do limite de tamanho e artefatos de build. Links simbólicos não saem do
   workspace.
4. **Análise estrutural e semântica** — símbolos por Tree-sitter (com fallback heurístico),
   importações e arestas do grafo.
5. **Construção do índice** — SQLite transacional com migrações e FTS5. Um índice novo, ou
   feito por um parser anterior, é relido por completo; só então `indexed_parser_version` passa
   a registrar o parser atual. Um índice antigo nunca é rotulado como atual sem ser relido.
6. **Verificação** — integridade do banco, consistência do FTS5, busca real e handshake MCP
   executado de verdade.
7. **Arquivos do agente** — só se a verificação passou. `CLAUDE.md`, `AGENTS.md` e a skill
   `devzymer` (veja [agent-skill.md](agent-skill.md)).
8. **Registro dos agentes** — servidor MCP, permissão das ferramentas, hooks de roteamento e
   proteção do índice no `.gitignore`, de forma aditiva e idempotente.

O estágio atingido fica em `.devzymer/init-state.json` (`indexing`, `indexed`, `verified`,
`agent_files`, `complete`). Se o `init` for interrompido, `devzymer doctor` diz até onde foi e o
que fazer; os arquivos do agente nunca existem sem que o índice tenha sido verificado.

**Arquivos do agente**

| Arquivo | Agentes | Conteúdo |
| --- | --- | --- |
| `CLAUDE.md` | Claude Code | Bloco curto entre `<!-- devzymer:start -->` e `<!-- devzymer:end -->` |
| `AGENTS.md` | Codex, OpenCode | O mesmo bloco, com os nomes de ferramenta dos dois hosts |
| `.claude/skills/devzymer/` | Claude Code | A skill, com referências |
| `.agents/skills/devzymer/` | Codex, OpenCode | A mesma skill |

Só o texto entre os marcadores é do devzymer. O que está fora nunca é reescrito; um bloco ausente
é acrescentado ao fim; um bloco antigo é atualizado no mesmo lugar; blocos duplicados viram um;
marcadores malformados deixam o arquivo como está e o `init` avisa. Quebras de linha (CRLF/LF) e
BOM são preservados, e a escrita é atômica. Se você escreveu uma instrução que parece contrariar
o devzymer ("nunca use o devzymer", "sempre use grep"), ela é mantida e apontada no relatório.

O índice local é um arquivo SQLite específico da máquina. Em um repositório git, o `init`
acrescenta `.devzymer/` ao `.gitignore`, para que ele não entre no histórico. A entrada não é
duplicada em execuções seguintes, e projetos fora de um repositório não recebem arquivo.

Saída resumida:

```
implementado: índice criado em .\.devzymer\index.sqlite3
  arquivos: 85 indexados, 0 inalterados, 0 removidos, 1 ignorados
  símbolos: 802
  grafo: 2285 relações (1483 chamadas, 0 importações)
arquivos do agente:
  [✓] CLAUDE.md: criado
  [✓] AGENTS.md: criado
  [✓] .claude/skills/devzymer: criado
  [✓] .agents/skills/devzymer: criado
validado: busca=true | banco=true | fts5=true | mcp=true | parser=true
```

`--dry-run` não escreve nada e serve para inspecionar o que seria indexado e quais arquivos do
agente seriam criados ou alterados.

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

Executa um manipulador de hook (Claude Code, Codex ou plugin do OpenCode). Lê JSON do `stdin` e
escreve JSON no `stdout`. Não é um comando de uso direto: o host o invoca. Fora de um workspace
com `.devzymer/`, nunca faz nada.

```bash
devzymer hook post-tool-use      # comprime saída grande de ferramenta
devzymer hook guard-test-files   # bloqueia criação de arquivos de teste sem aprovação
devzymer hook guard-search --host claude|codex|opencode   # roteamento da busca
devzymer hook note-mcp --host ...         # registra o resultado de uma chamada ao devzymer
devzymer hook note-prompt --host ...      # registra um pedido explícito de busca nativa
```

---

## `devzymer routing`

```bash
devzymer routing status [--json]     # modo, mecanismo por host, saúde do índice, resumo de 7 dias
devzymer routing mode [off|prefer|guard]
devzymer routing explain --tool <T> --input '<json>' [--host H] [--mode M]   # simulação, sem registro
devzymer routing stats [--days N] [--json]
devzymer routing reset               # apaga telemetria, sessões e cache de saúde
```

Ver [routing.md](routing.md).

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
