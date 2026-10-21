# Integração MCP

## Transporte

O devzymer expõe um servidor **stdio**: JSON-RPC 2.0, uma mensagem por linha, sobre stdin e
stdout. Não há porta, não há processo de segundo plano, não há estado remoto.

O servidor é iniciado pelo Claude Code com o comando registrado em `.mcp.json`:

```json
{
  "mcpServers": {
    "devzymer": {
      "type": "stdio",
      "command": "C:\\caminho\\devzymer.exe",
      "args": ["serve", "--root", "${CLAUDE_PROJECT_DIR:-.}"]
    }
  }
}
```

`${CLAUDE_PROJECT_DIR:-.}` é a variável estável de raiz do projeto que o Claude Code exporta para
servidores stdio. O `:-.` cobre clientes que não a definem.

## Métodos

| Método | Resultado |
| --- | --- |
| `initialize` | Versão do protocolo, capacidades e informações do servidor |
| `notifications/initialized` | Notificação, sem resposta |
| `tools/list` | Definições das ferramentas com esquema JSON |
| `tools/call` | Execução de uma ferramenta |
| `ping` | Verificação de vivacidade |

Versões de protocolo aceitas: `2024-11-05`, `2025-03-26`, `2025-06-18`. O servidor devolve a
versão solicitada quando ela é conhecida e, caso contrário, a versão configurada.

## Ferramentas

### `context_resolve`

Localiza símbolos, arquivos e faixas de linhas verificadas.

```json
{
  "query": "validateSession",
  "path": "src/auth",
  "language": "typescript",
  "limit": 10
}
```

Somente `query` é obrigatório. A cascata de recuperação é, nesta ordem: caminho exato, nome de
arquivo, símbolo exato, símbolo qualificado (`Classe.metodo`), correspondência por token, prefixo,
busca lexical FTS5/BM25 e expansão pelo grafo.

**Uma consulta pode nomear um arquivo.** Quando a consulta é o nome de um arquivo sem extensão
(`useVoiceCall`), o devzymer casa com o `stem` indexado e devolve o arquivo com os seus símbolos:

```
arquivo: src/hooks/useVoiceCall.ts | 28 símbolo(s) | nome do arquivo
src/hooks/useVoiceCall.ts:11-69 | useLeituraVoiceCall | function | heuristic
```

A última coluna é a confiança: `confirmed` (caminho, símbolo exato ou qualificado),
`heuristic` (token, prefixo, lexical, nome de arquivo ou grafo). O resultado estruturado traz
também a origem da correspondência e o escore.

**Nomes ambíguos.** Quando um nome simples corresponde a mais de uma definição, a resposta
diz isso explicitamente e não escolhe um vencedor arbitrário:

```
ambiguidade: 'objeto' tem 26 definições; use o nome qualificado para desambiguar
```

O bloco `ambiguity` traz o nome, a contagem e a lista de nomes qualificados. Para desambiguar,
use o nome qualificado (`Container.metodo`), que resolve na primeira posição, ou acrescente o
nome do arquivo à consulta (`limites.ts objeto`), que também resolve.

**Cache de recuperação.** A resolução é memorizada por revisão do índice. Um acerto devolve
exatamente o mesmo contexto de uma falha: o cache economiza trabalho de banco, não tokens. O
status aparece em `cache` (`hit`, `miss` ou `disabled`).

Sem resultados, a ferramenta responde `SYMBOL_NOT_FOUND`. Antes de devolver essa resposta, o
servidor sincroniza o índice de forma limitada e repete a busca, para não reportar ausência com
base em um índice desatualizado.

### `context_read`

Lê um trecho preciso de um arquivo.

```json
{
  "path": "src/auth/session.ts",
  "symbol": "validateSession",
  "max_bytes": 8000,
  "numbered": true
}
```

Pode ser chamada por faixa de linhas (`start_line`, `end_line`), por símbolo, ou apenas por
caminho, caso em que lê o arquivo inteiro respeitando os limites de bytes e linhas. A resposta
inclui o hash do arquivo, que serve como token de revisão para o patch.

### `context_impact`

Mostra chamadores, dependências e o alcance de uma mudança.

```json
{ "symbol": "validateSession", "depth": 2 }
{ "path": "src/auth/session.ts" }
```

Com `symbol`, devolve o fechamento reverso de dependências: quem chama o símbolo, direta e
indiretamente. Com `path`, devolve os arquivos que importam o arquivo e os arquivos que ele
importa.

### `context_patch`

Aplica uma alteração precisa e revisada.

```json
{
  "path": "src/auth/session.ts",
  "symbol": "validateSession",
  "replacement": "export async function validateSession(token: string) { ... }",
  "expected_sha": "14d51643188290e1",
  "dry_run": false
}
```

Regras:

- o caminho precisa permanecer dentro do workspace;
- `expected_sha`, quando informado, precisa bater com o hash atual do arquivo;
- a faixa precisa existir no arquivo;
- a escrita é atômica, preservando a codificação e o estilo de quebra de linha;
- `dry_run: true` valida sem escrever.

A resposta é compacta:

```
OK | src/auth/session.ts:44->73 | sha 14d51643188290e1 -> 9f0c1a77b3d2e410 | 812 linhas -> 818 linhas
```

### `context_status`

Freshness, cobertura, integridade e capacidades.

```
index: 893 files, 24666 symbols, 42428 relations, 10380364 bytes | revision 16 | schema 2
integrity: ok | languages: typescript=364 tsx=218 sql=193
freshness: sync 1792464135097 | reconciling false | pending 0 | skipped 4 | falhas 0
```

O bloco `freshness` expõe `last_sync_ms`, `last_reconcile_ms`, `reconciling`, `syncs`,
`pending_changes`, `skipped` (arquivo e motivo), `failures` e `truncated`. O bloco `watcher`
mostra se o observador de filesystem está ativo, quantas sincronizações ele fez e o último erro.

### `context_sync`

Sincroniza o índice com o que está no disco. Útil para recuperar de um índice desatualizado ou
depois de uma mudança em massa.

```json
{ "full": false, "budget_ms": 2000 }
```

Com `full: true` o índice é reconstruído por completo. A resposta traz adicionados, modificados,
removidos, ignorados e falhas.

### `context_decide`

Decisão de roteamento com escopo delimitado, para quando as regras locais não bastam.

```json
{
  "task": "evidence_sufficient",
  "state": "exact_symbol confirmed, 1 resultado",
  "options": ["yes", "no"]
}
```

Tarefas aceitas: `complexity`, `retrieval_strategy`, `need_more_context`, `evidence_sufficient`,
`next_source`, `escalate`, `semantic_fallback`, `workflow`.

A política de privacidade é aplicada: um estado que parece conter código-fonte é recusado com
`PERMISSION_DENIED` enquanto `privacy.allow_remote_source` estiver desligado. Sem provedor
externo configurado, a decisão vem das regras locais e o provedor é reportado como
`deterministic`.

## Códigos de erro

| Código | Significado |
| --- | --- |
| `SYMBOL_NOT_FOUND` | Nenhum símbolo corresponde à consulta |
| `AMBIGUOUS_MATCH` | A consulta corresponde a mais de um candidato |
| `STALE_INDEX` | O índice não reflete o estado atual do arquivo |
| `PERMISSION_DENIED` | Operação bloqueada pela política de privacidade |
| `UNSUPPORTED_LANGUAGE` | Linguagem sem gramática e sem fallback aplicável |
| `INVALID_PATCH` | Revisão obsoleta, faixa inválida ou caminho fora do workspace |
| `PROVIDER_UNAVAILABLE` | Provedor de decisão externo indisponível |
| `RESOURCE_LIMIT_EXCEEDED` | Limite configurado excedido |
| `PATH_ESCAPE` | Caminho fora do workspace |

As respostas de erro usam `isError: true` e trazem o mesmo código no texto e no
`structuredContent`.

## Permissão das ferramentas

O Claude Code rejeita uma ferramenta MCP que não tem regra de permissão: a chamada volta com
`you haven't granted it yet`, o agente segue adiante e o índice nunca é consultado. A regra
`mcp__devzymer__*` na lista `permissions.allow` é o que libera o uso.

O instalador concede a permissão no escopo em que o servidor é registrado:

| Escopo | Arquivo | Regra |
| --- | --- | --- |
| Projeto | `<projeto>/.claude/settings.json` | `mcp__devzymer__*` |
| Usuário | `~/.claude/settings.json` | `mcp__devzymer__*` |

Verifique com `devzymer status`, que informa se a permissão está presente. `devzymer uninstall`
remove a regra junto com o servidor.

## Limites do host

O Claude Code avisa quando a saída de uma ferramenta MCP ultrapassa 10.000 tokens e limita a
25.000 por padrão (`MAX_MCP_OUTPUT_TOKENS`). As respostas do devzymer são deliberadamente
curtas: a expansão é solicitada, não automática. O tool schema também tem custo de contexto, e
por isso a superfície é de seis ferramentas, não dezenas.

## Guarda de busca

Com centenas de ferramentas MCP carregadas, o agente decide sozinho se usa o índice, e com
frequência escolhe varrer o repositório. A regra de texto no `CLAUDE.md` reduz a decisão, mas
não a elimina: a escolha é do modelo, e ela varia entre sessões.

A guarda `hook guard-search` remove a variabilidade. É um hook `PreToolUse` que nega a
varredura **apenas quando o índice sabe responder**, e indica a ferramenta que responde:

| Ferramenta | Negada quando | Indicação devolvida |
| --- | --- | --- |
| `Grep` | o padrão contém um identificador que existe no índice | `context_resolve` com aquele termo |
| `Read` | o arquivo está indexado | `context_read` com o caminho, que aceita faixa de linhas ou símbolo |
| `Glob` | o padrão não tem curinga e o arquivo existe | `context_resolve` com o nome do arquivo |

Uma busca por texto que não é código, um arquivo fora do índice e um padrão com curinga
passam sem interferência. A guarda não é um bloqueador: ela redireciona para a resposta
melhor e mais barata.

O comportamento é configurável:

```bash
devzymer config set context.guard_search false
```

Sem índice, ou com a guarda desligada, o hook não nega nada.

## Verificação

```bash
python scripts/mcp_smoke.py
```

O script sobe o servidor real, executa o handshake, lista as ferramentas, chama cada uma contra o
índice vivo e confere que uma consulta inexistente produz erro. Não simula o protocolo: fala
JSON-RPC por stdio com o binário em execução.

```bash
python scripts/sync_check.py /caminho/do/projeto
```

O script reproduz a consulta que nomeia um arquivo, altera um arquivo de código, confirma que a
mudança ficou pesquisável sem comando manual, restaura o arquivo e confere o hash original.
