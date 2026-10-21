# Configuração

## Camadas

A configuração efetiva é resolvida nesta ordem, do menos para o mais específico:

1. padrões embutidos
2. `~/.config/devzymer/config.toml` (usuário)
3. `.devzymer/config.toml` (projeto)

O projeto sobrescreve o usuário. Arquivos ausentes são ignorados. Chaves desconhecidas geram
erro de validação, para que um erro de digitação não passe silenciosamente.

```bash
devzymer config list
devzymer config get <chave>
devzymer config set <chave> <valor>
devzymer config path
```

`devzymer config set` grava sempre no arquivo do projeto.

## Referência

### `[index]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `exclude` | lista de diretórios comuns de build e dependência | Padrões aplicados sobre a varredura. Um nome simples exclui em qualquer nível; um caminho com `/` é ancorado na raiz |
| `max_file_size_bytes` | `2000000` | Arquivos maiores são ignorados |
| `follow_symlinks` | `false` | Quando verdadeiro, links simbólicos são seguidos, desde que o destino permaneça no workspace |
| `respect_gitignore` | `true` | Aplica `.gitignore`, `.ignore`, `.git/info/exclude` e `.devzymerignore` |
| `include_hidden` | `false` | Inclui arquivos e diretórios ocultos |
| `languages` | `[]` | Lista de linguagens a indexar. Vazio significa todas |
| `max_workers` | `0` | Número de threads de parsing. `0` usa a capacidade paralela da máquina |
| `memory_budget_mb` | `512` | Alvo de memória para o processamento em blocos |

Exemplo:

```toml
[index]
exclude = ["node_modules", "dist", "target", "vendor"]
max_file_size_bytes = 4000000
max_workers = 8
```

### `[search]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `max_results` | `20` | Limite de resultados por consulta |
| `default_token_budget` | `2000` | Orçamento de tokens padrão de um contexto |
| `snippet_max_lines` | `40` | Linhas máximas por trecho lido |
| `max_fragment_bytes` | `24000` | Bytes máximos por trecho |
| `min_bm25_score` | `0.18` | Corte mínimo do resultado lexical normalizado. Abaixo dele a correspondência é tratada como ruído |

### `[context]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `default_level` | `2` | Nível de divulgação inicial: 1 localização, 2 evidência, 3 implementação, 4 dependência, 5 expandido |
| `max_total_bytes` | `64000` | Teto de bytes por conjunto de contexto |
| `dedupe` | `true` | Remove fragmentos idênticos |
| `cache_enabled` | `true` | Habilita o cache de recuperação |
| `cache_ttl_secs` | `900` | Validade do cache |

### `[decisions]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `enabled` | `false` | Habilita o provedor de decisão externo |
| `provider` | `"typesafe"` | `typesafe` usa a API System One |
| `model` | `"typesafe/jev-1.14"` | Modelo. O prefixo do fornecedor é removido para a API System One |
| `base_url` | `"https://api.typesafe.ai"` | Endpoint do provedor |
| `api_key_env` | `"TYPESAFE_API_KEY"` | Variável de ambiente com a credencial |
| `timeout_secs` | `10` | Tempo limite por requisição |
| `max_retries` | `1` | Retentativas após falha |
| `cache_ttl_secs` | `3600` | Validade do cache de decisão |
| `circuit_breaker_threshold` | `3` | Falhas consecutivas que abrem o circuito |
| `circuit_breaker_cooldown_secs` | `60` | Tempo de repouso do circuito |
| `max_state_chars` | `6000` | Tamanho máximo do resumo enviado ao provedor |
| `min_confidence` | `0.55` | Confiança mínima para aceitar uma decisão externa |

### `[mcp]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `server_name` | `"devzymer"` | Nome do servidor registrado no Claude Code |
| `protocol_version` | `"2025-06-18"` | Versão do protocolo MCP anunciada |
| `install_scope` | `"project"` | `project`, `local` ou `user` |
| `max_output_tokens` | `4000` | Teto de tokens por resposta de ferramenta |

### `[privacy]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `allow_remote_source` | `false` | Permite enviar conteúdo de origem a provedores externos. Mantenha desligado |
| `redact_secrets` | `true` | Redige material que pareça credencial em logs |
| `exclude_secret_files` | `true` | Exclui arquivos com nomes típicos de segredo da indexação |

### `[logging]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `level` | `"info"` | `trace`, `debug`, `info`, `warn`, `error`, `off` |
| `file` | nenhum | Caminho de arquivo de log adicional |

### `[tests]`

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `allow_files` | `false` | Libera a criação de arquivos de teste dedicados sem aprovação |
| `approval_marker` | `".devzymer/tests.approved"` | Arquivo que concede aprovação no projeto |

## Exemplo completo

```toml
[index]
exclude = ["node_modules", "dist", "target"]
max_file_size_bytes = 3000000
max_workers = 8

[search]
max_results = 25
default_token_budget = 3000

[context]
default_level = 2

[decisions]
enabled = false
model = "typesafe/jev-1.14"

[mcp]
install_scope = "project"

[privacy]
allow_remote_source = false
```
