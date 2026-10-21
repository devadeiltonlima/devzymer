# Instalação

## Visão geral

O `devzymer init` indexa o repositório e instala a integração com um ou mais agentes de código.
A escolha é feita em uma interface de terminal interativa, ou pela linha de comando quando o
terminal não é interativo.

## Instalar o binário sem clonar o repositório

```powershell
irm https://github.com/devadeiltonlima/devzymer/releases/latest/download/install.ps1 | iex
```

```bash
curl -fsSL https://github.com/devadeiltonlima/devzymer/releases/latest/download/install.sh | sh
```

Os scripts (`scripts/install.ps1` e `scripts/install.sh`) baixam o arquivo da plataforma na página de
releases, conferem o SHA-256 publicado ao lado dele e só então instalam. Aceitam `DEVZYMER_VERSION`,
`DEVZYMER_INSTALL_DIR`, `DEVZYMER_REPO` e `DEVZYMER_BASE_URL`; o de Windows aceita também
`DEVZYMER_NO_MODIFY_PATH`. Os arquivos de release são produzidos por
`.github/workflows/release.yml` a cada tag `v*`:

| Arquivo | Plataforma |
| --- | --- |
| `devzymer-x64-pc-windows-msvc.zip` | Windows x64 |
| `devzymer-x64-unknown-linux-gnu.tar.gz` | Linux x64 |
| `devzymer-arm64-unknown-linux-gnu.tar.gz` | Linux arm64 |
| `devzymer-x64-apple-darwin.tar.gz` | macOS Intel |
| `devzymer-arm64-apple-darwin.tar.gz` | macOS Apple Silicon |

Quem preferir compilar: `cargo install --git https://github.com/devadeiltonlima/devzymer devzymer-cli --locked`.

## Agentes suportados

Agentes suportados:

| Agente | Chave | Configuração do servidor MCP | Instruções |
| --- | --- | --- | --- |
| Claude Code | `claude` | `.mcp.json` do projeto, `~/.claude.json` no escopo de usuário | `CLAUDE.md` |
| Codex | `codex` | `~/.codex/config.toml` ou `<projeto>/.codex/config.toml` | `AGENTS.md` |
| OpenCode | `opencode` | `opencode.json` do projeto, `~/.config/opencode/opencode.json(c)` | `AGENTS.md` |

Os três agentes usam **o mesmo motor**: um único índice por repositório, o mesmo grafo de
dependências, o mesmo gerenciador de contexto e o mesmo servidor MCP.

## Interface interativa

```bash
devzymer init
```

A interface abre com a seleção de agentes:

```
┌────────────────────────────────────────────────────────────────────────┐
│  DEVZYMER                                                             │
│  Less Tokens. More Engineering.                                       │
│                                                                        │
│   [✓] Claude Code     Detectado                                       │
│   [ ] Codex           Detectado                                       │
│   [ ] OpenCode        Detectado                                       │
│                                                                        │
│  Selecionados: 1 de 3   [ Enter: continuar ]   [ Esc: cancelar ]     │
└────────────────────────────────────────────────────────────────────────┘
```

| Tecla | Efeito |
| --- | --- |
| `↑` / `↓` | Move o cursor entre os agentes |
| `espaço` | Marca ou desmarca o agente sob o cursor |
| `a` | Marca todos ou desmarca todos |
| `enter` | Confirma e avança para a escolha do escopo |
| `esc` | Cancela, sem alterar nada |

A segunda tela escolhe o escopo:

| Tecla | Efeito |
| --- | --- |
| `←` / `→` | Alterna entre projeto e usuário |
| `enter` | Instala |
| `esc` | Volta para a seleção de agentes |

O agente que já está instalado entra marcado, então uma reinstalação mantém a escolha. Sem
nenhum agente configurado, o Claude Code entra marcado, que é o comportamento histórico.

A interface funciona em PowerShell, Windows Terminal, Terminal do macOS e terminais Linux. O
terminal volta ao estado normal ao sair, inclusive quando a instalação falha.

## Linha de comando

```bash
devzymer init --agents claude
devzymer init --agents codex,opencode
devzymer init --agents claude,codex,opencode
devzymer init --agents claude --scope user
```

`--agents` aceita `claude`, `codex` e `opencode`, separados por vírgula. Um agente desconhecido
faz o comando falhar, com a lista dos valores aceitos.

### Quando não há terminal interativo

Sem terminal, ou com `--yes`, o `init` não espera por teclado. O comportamento é o
histórico: apenas o Claude Code é integrado, e um aviso explica como escolher:

```bash
devzymer init --agents claude,codex    # sem espera por teclado
devzymer init --yes                    # ignora a interface
```

## Escopo de instalação

| Escopo | Onde registra | Quando usar |
| --- | --- | --- |
| `project` | Arquivos do repositório | Padrão. Compartilhado com a equipe pelo controle de versão |
| `user` | Arquivos do usuário | Em todos os repositórios e hosts, sem aprovação por projeto |
| `local` | `~/.claude.json`, na chave do projeto | Só nesta máquina, só neste repositório |

O escopo da linha de comando tem precedência sobre a escolha da interface, que tem precedência
sobre a configuração `mcp.install_scope`. Uma instalação de projeto nunca é convertida em global
sem uma escolha explícita.

**O índice local continua isolado por repositório em qualquer escopo.** O escopo decide onde o
servidor MCP é registrado, não onde o índice vive.

## Índice compartilhado

O índice fica em `<repositório>/.devzymer/index.sqlite3`. Selecionar os três agentes não cria
três índices:

```
Projeto
  .devzymer/
      index.sqlite3
      config.toml
      backups/

Claude Code ─┐
Codex       ─┼── devzymer Core ── index.sqlite3
OpenCode    ─┘
```

Cada agente pode iniciar o seu próprio processo MCP. O acesso ao banco é seguro para processos
concorrentes: o SQLite opera em modo WAL, com `busy_timeout` de dez segundos, e as escritas são
transacionais.

A raiz do repositório é resolvida nesta ordem: o argumento `--root`, a variável
`DEVZYMER_PROJECT_DIR`, a `CLAUDE_PROJECT_DIR` e, por fim, a busca do diretório de trabalho até
encontrar a raiz do projeto. O primeiro candidato que tem índice vence. Sem índice, o servidor
não devolve dados de outro repositório: ele falha de forma explícita.

## O que cada agente recebe

### Claude Code

- Servidor MCP em `.mcp.json` (projeto) ou `~/.claude.json` (usuário).
- Regras de uso em `CLAUDE.md`.
- Hooks de compressão de saída, guarda de arquivos de teste e guarda de busca.
- Permissão `mcp__devzymer__*` na lista `permissions.allow`, sem a qual o host rejeita toda
  chamada.

### Codex

- Servidor MCP em `[mcp_servers.devzymer]`, com `command`, `args` e `enabled`.
- Regras de uso em `AGENTS.md`.
- Sem hooks: o protocolo de hooks do Claude Code não é o do Codex, e nada é instalado no lugar
  errado.

A configuração do Codex é editada de forma cirúrgica, com `toml_edit`: o resto do arquivo, com
os comentários e a formatação do usuário, permanece intacto.

### OpenCode

- Servidor MCP em `mcp.servers.devzymer`, do tipo `local`.
- Regras de uso em `AGENTS.md`.

O arquivo global pode ser `opencode.jsonc`, com comentários. Nesse caso a alteração é feita
sobre o texto original, e o que o usuário escreveu é preservado. Servidores, plugins, modelos,
permissões e instruções existentes ficam como estavam.

## Gerenciamento das integrações

```bash
devzymer integrations            # lista os agentes e o estado
devzymer integrations add codex  # integra mais um agente
devzymer integrations remove codex
devzymer integrations repair     # reaplica e verifica o que está instalado
devzymer integrations verify     # handshake real com o servidor MCP
devzymer integrations scope codex user
```

Adicionar um agente não exige recriar o índice: o motor é o mesmo e o banco continua o mesmo.

Desmarcar um agente na interface significa "não instalar ou não modificar este agente". Ele
**não** é removido. A remoção é uma ação explícita, em `devzymer integrations remove` ou em
`devzymer uninstall`.

O `devzymer doctor` confere o estado de cada agente, junto com o workspace, o git, o banco, o
FTS5, a atualidade do índice, a integração MCP e o provedor de decisão.

## Reinstalação

A reinstalação é idempotente. O instalador inspeciona a configuração existente, identifica as
entradas do devzymer, faz backup dos arquivos afetados, aplica a menor mudança possível e
verifica o resultado. Uma segunda execução não duplica nada:

```bash
devzymer init --agents claude,codex,opencode   # pode rodar quantas vezes for
```

## Cancelamento e falha

- `esc` na interface cancela antes de qualquer escrita: nada é alterado.
- Se um agente falhar, os outros permanecem instalados e a falha é mostrada separadamente.
  Uma instalação parcial nunca é descrita como completa.
- Todo arquivo alterado tem backup em `.devzymer/backups/`.

## Verificação da instalação

```bash
devzymer --version
devzymer doctor
devzymer integrations
```

`doctor` confere o workspace, o git, o banco, o FTS5, a atualidade do índice, os três agentes, a
integração MCP, a presença de CodeGraph e o estado do provedor de decisão. Sai com código 1 se
alguma verificação falhar.

## Uso sem agente de código

O binário funciona de forma autônoma:

```bash
devzymer init --no-integrate
devzymer resolve "minha funcao"
devzymer benchmark --format markdown
```

O servidor MCP também pode ser conectado a qualquer cliente compatível, recebendo JSON-RPC por
stdio.

## Solução de problemas

| Sintoma | Causa provável | Verificação |
| --- | --- | --- |
| O agente não usa o devzymer | Permissão ausente no `permissions.allow` | `devzymer integrations` e `devzymer doctor` |
| `ERR SYMBOL_NOT_FOUND` | Índice desatualizado ou nome inexistente | `devzymer sync` e `devzymer resolve "termo"` |
| `SYMBOL_NOT_FOUND` em outro repositório | Servidor iniciado fora do projeto | `devzymer doctor` mostra a raiz usada |
| A interface não abre | Terminal sem suporte a TTY | Use `--agents` |
| O Codex não encontra o servidor | Caminho do executável inválido | `devzymer integrations repair` |
| O OpenCode não carrega o servidor | JSON inválido após a edição | `devzymer doctor` e o arquivo de configuração |

Detalhes em [docs/troubleshooting.md](troubleshooting.md).

## Scripts de verificação

O repositório traz dois scripts que exercitam as jornadas de instalação sem tocar na
configuração real do usuário:

```bash
python scripts/journeys.py       #Claude, Codex, OpenCode, combinações, reinstalação, JSONC
python scripts/concurrency.py    # vários processos MCP no mesmo índice
```

Ambos usam projetos temporários e verificam o resultado no disco.
