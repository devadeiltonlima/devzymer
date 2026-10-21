# Roteamento do agente

O servidor MCP não garante que o modelo o escolha. O roteamento é o que torna a escolha
verificável: um motor de política local classifica cada chamada de busca nativa do agente
(`Grep`, `Glob`, `rg`, `grep`, `findstr`, `Select-String`, `find`...), decide se o devzymer
pode responder e, conforme o modo, orienta ou redireciona. O índice e a recuperação não mudam.

## Modos

| Modo | Efeito |
| --- | --- |
| `off` | Nenhum hook ou plugin interfere. |
| `prefer` (padrão) | Orienta com uma dica curta, no máximo 3 por sessão, e registra telemetria. Nunca bloqueia. |
| `guard` | Redireciona **uma vez** a busca que o índice sabe responder. A repetição da mesma busca é sempre permitida. |

`guard` só vale por escolha explícita, nunca por padrão nem por instalação:

```bash
devzymer routing mode guard      # ou: devzymer config set routing.mode guard
devzymer routing mode            # mostra o modo e de onde ele vem
```

O antigo `context.guard_search = true` continua funcionando, mas apenas como pedido explícito de
`guard`; `routing.mode` tem precedência. O padrão de `context.guard_search` passou a ser `false`.

## Quando o `guard` redireciona

O hook nega somente se **todas** estas condições forem verdadeiras:

1. a chamada é uma busca de código ou de nome de arquivo no repositório;
2. o termo é um identificador (`snake_case`, `camelCase`, `PascalCase` ou com `.`) que existe
   no índice como símbolo ou nome de arquivo;
3. o índice existe, é íntegro (`quick_check` e consulta FTS), tem o esquema atual, pertence a
   este workspace e não tem arquivos alterados desde a indexação;
4. o servidor MCP está vivo (batimento em `.devzymer/routing/heartbeat.json`, no máximo 90 s);
5. o devzymer não falhou nesta sessão nos últimos 5 minutos;
6. o usuário não pediu busca nativa e o agente não usou o devzymer nos últimos 3 minutos;
7. esta busca ainda não foi redirecionada, e a sessão não esgotou o orçamento de 6 redirecionamentos.

Fora disso, a busca passa e o motivo é registrado (`index_stale`, `mcp_unavailable`,
`workspace_mismatch`, `index_unhealthy`, `retry_after_deny`, `recent_devzymer_failure`,
`explicit_request`, `outside_coverage`, `not_answerable`...). O hook nunca indexa nem repara.

Nunca são interceptados: logs e saída de comandos (`cmd | grep`), um único arquivo, `find` com
`-exec`/`-delete`, busca em `node_modules`/`target`/`.git`, regex sem identificador, execução de
testes, builds e Git, e o texto citado em `echo` ou em mensagem de commit. `Read` nunca é
negado: o `Edit` do host exige o `Read`; o hook apenas sugere `context_read` para arquivo
indexado grande.

## Mecanismo por host

| Host | Mecanismo | Garantia |
| --- | --- | --- |
| Claude Code | hooks `PreToolUse` (`Grep\|Glob\|Read\|Bash\|PowerShell`), `PostToolUse`/`PostToolUseFailure` do MCP e `UserPromptSubmit` em `.claude/settings.json` | Técnica, no modo `guard`. |
| OpenCode 2 | plugin `.opencode/plugins/devzymer-routing/index.ts` com `execute.before`, que consulta `devzymer hook guard-search` e bloqueia lançando erro | Técnica, no modo `guard`. OpenCode 1 usa outra API: só orientação. |
| Codex | `.codex/hooks.json` (`PreToolUse` em `Bash`, `PostToolUse` do MCP, `UserPromptSubmit`) mais o `AGENTS.md` | O Codex só executa hooks depois de você revisá-los em `/hooks`. Antes disso, vale apenas a orientação do `AGENTS.md`, que não é imposição técnica. |

Em `prefer`, o Claude Code e o Codex recebem a dica como `additionalContext`; o plugin do
OpenCode anexa a dica à saída da ferramenta.

O `AGENTS.md` do Codex e o `CLAUDE.md` carregam, literalmente: *Use devzymer MCP as the first
retrieval tool for locating repository files, symbols, dependencies and implementation behavior.
Use native search when devzymer is unavailable, stale, incomplete, or when native search is more
appropriate.*

## Diagnóstico

```bash
devzymer routing status                  # modo, mecanismo de cada host, saúde, resumo de 7 dias
devzymer routing explain --tool Grep --input '{"pattern":"UserService"}' --mode guard
devzymer routing stats --days 30 --json
devzymer routing reset                   # apaga telemetria, sessões e cache de saúde
```

`explain` simula a decisão sem registrar nada.

## Telemetria

Local e agregada, em `.devzymer/routing/events.jsonl` (rotaciona em 1 MB). Cada evento guarda
data, host, modo, tipo (`mcp_call`, `native_search`, `redirected`, `fallback_allowed`, `advised`),
categoria da ferramenta (`grep`, `rg`...), motivo e latência. **Nunca** guarda prompt, comando,
padrão de busca, código ou credencial; a sessão é um hash de 12 hex. Cada evento diz de onde veio:
`host` (visto pelo hook), `server` (visto pelo servidor MCP) ou `inferred`; o host de uma chamada
MCP vem do `clientInfo` do cliente. A taxa de "devzymer primeiro" só usa sessões observadas pelo
host. Economia de tokens não é medida nem reportada. Desligue com `routing.telemetry = false`.

## Configuração

| Chave | Padrão | Efeito |
| --- | --- | --- |
| `routing.mode` | `prefer` | `off`, `prefer` ou `guard` |
| `routing.telemetry` | `true` | grava os eventos agregados |
| `routing.max_denials_per_session` | 6 | orçamento de redirecionamentos |
| `routing.max_advisories_per_session` | 3 | orçamento de dicas |
| `routing.health_cache_secs` | 15 | cache da verificação de saúde |
| `routing.failure_window_secs` | 300 | janela em que uma falha libera a busca nativa |
| `routing.recent_use_window_secs` | 180 | janela do "uso recente" do devzymer |

## Implantação

Os hooks e o plugin são gravados por `devzymer init` (ou `devzymer integrations repair`) no
**projeto**. Nada é alterado em escopo global sem `--scope user`. Depois de instalar ou atualizar:
reinicie o agente; no Codex, revise os hooks em `/hooks`; reinicie o servidor MCP para que ele
publique o batimento e a telemetria. Uma versão antiga do hook `guard-search` é migrada, não
duplicada.
