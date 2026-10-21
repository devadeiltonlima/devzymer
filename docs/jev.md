# Jev: roteamento de decisão estruturado

## O que o Jev é

Jev é o modelo de decisão estruturada da TypeSafe, o primeiro|System One. Ele não gera texto:
avalia perguntas tipadas contra um estado e devolve valores tipados com distribuição de
probabilidade e, em alguns casos, confiança. A API usada é a **System One**, não o chat
completions.

Endpoint: `POST https://api.typesafe.ai/v1/systemone`

```json
{
  "model": "jev-1.14",
  "state": "resumo curto e não sensível da evidência atual",
  "questions": {
    "evidence_sufficient": {
      "type": "choice",
      "instructions": "A evidência coletada é suficiente para agir?",
      "criteria": { "yes": "...", "no": "..." }
    }
  }
}
```

Resposta:

```json
{
  "model": "jev-1.14",
  "answers": {
    "evidence_sufficient": {
      "type": "choice",
      "choice": "yes",
      "confidence": 0.82,
      "probabilities": { "yes": 0.79, "no": 0.21 }
    }
  },
  "usage": { "input_tokens": 328, "output_tokens": 34 }
}
```

Perguntas do tipo `noul` respondem sim/não com a probabilidade de "sim", sem lista de opções.

## Responsabilidades

O devzymer delega ao Jev apenas classificações delimitadas:

| Tarefa | Pergunta |
| --- | --- |
| `complexity` | Quão complexa é a tarefa? |
| `retrieval_strategy` | Qual estratégia de recuperação serve? |
| `need_more_context` | Falta contexto de origem? |
| `evidence_sufficient` | A evidência basta para agir? |
| `next_source` | Qual fonte consultar agora? |
| `escalate` | A decisão deve escalar para o agente? |
| `semantic_fallback` | Vale o custo de um fallback semântico? |
| `workflow` | Qual caminho de trabalho serve? |

O Jev não recebe decisões de arquitetura, não recebe o código do repositório e não concede
permissão para operação destrutiva.

## Hierarquia de decisão

**1. Regras determinísticas.** Se a decisão pode ser tomada com regra explícita ou evidência
indexada, o Jev não é chamado. Na configuração padrão, `decisions.enabled` é `false` e isso é o
comportamento.

**2. Jev.** Consultado apenas quando a regra local não é decisiva (confiança abaixo de 0,7) e o
provedor está configurado.

**3. Agente.** Questões abertas, arquitetura, segurança e incerteza de correção sobem para o
Claude Code, nunca para o modelo de decisão.

## Onde colocar a chave

A chave **nunca** vai para arquivo de configuração, repositório ou log. Ela vive em uma variável
de ambiente, e o devzymer só a lê dali.

### Caminho nativo: TypeSafe System One

É o caminho documentado do Jev, e o que o provedor usa por padrão.

```powershell
# Sessão atual
$env:TYPESAFE_API_KEY = "sua-chave"

# Permanente, para o seu usuário
setx TYPESAFE_API_KEY "sua-chave"
```

```bash
# Linux e macOS
export TYPESAFE_API_KEY="sua-chave"
```

Depois, ligue o provedor:

```bash
devzymer config set decisions.enabled true
```

Verifique com `devzymer doctor`: a linha `decisões` informa se a chave foi encontrada.

### Caminho alternativo: OpenRouter

O mesmo modelo, `typesafe/jev-1.14`, também é publicado no OpenRouter. Lá o endpoint é o
`chat/completions` comum, e o modelo **não declara** o parâmetro de structured output. O
adaptador do devzymer pede a decisão em um esquema JSON e interpreta a resposta de forma
tolerante: um objeto JSON válido é usado como está, e uma resposta em texto livre que cite uma
das opções é aceita.

```powershell
setx OPENROUTER_API_KEY "sua-chave"
```

```bash
devzymer config set decisions.provider openrouter
devzymer config set decisions.base_url https://openrouter.ai/api/v1
devzymer config set decisions.api_key_env OPENROUTER_API_KEY
devzymer config set decisions.enabled true
```

Depois de `setx`, **abra um terminal novo**: a variável só existe em processos novos.

### Configuração

```toml
[decisions]
enabled = true
provider = "typesafe"          # ou "openrouter"
model = "typesafe/jev-1.14"
base_url = "https://api.typesafe.ai"
api_key_env = "TYPESAFE_API_KEY"
timeout_secs = 10
max_retries = 1
min_confidence = 0.55
```

A credencial vem da variável de ambiente `api_key_env` e nunca é gravada em arquivo, log, cache
ou relatório. O prefixo do fornecedor no nome do modelo é removido automaticamente para a API
System One e mantido para o OpenRouter.

### Sem chave

Sem a variável definida, o provedor externo não é construído. O roteamento continua funcionando
com as regras locais e a resposta vem com `provider=deterministic`. Nada quebra.

## Salvaguardas

- **Sem laço recursivo.** O provedor é consultado uma vez por decisão; a resposta nunca dispara
  nova consulta.
- **Estado delimitado.** O resumo enviado é limitado por `decisions.max_state_chars`.
- **Privacidade.** Um estado que parece conter código-fonte é recusado com `PERMISSION_DENIED`
  enquanto `privacy.allow_remote_source` estiver desligado.
- **Sem escalonamento automático de permissão.** Uma decisão do Jev nunca autoriza uma operação
  destrutiva.
- **Sem atribuição inventada.** A origem e a confiança da decisão são reportadas como vieram.
- **Cache.** Decisões repetidas com o mesmo estado são reaproveitadas por
  `decisions.cache_ttl_secs`.
- **Circuit breaker.** Após `circuit_breaker_threshold` falhas consecutivas, as chamadas param
  por `circuit_breaker_cooldown_secs` e o sistema volta ao fallback local.
- **Timeout e retentativa limitada.** `timeout_secs` e `max_retries` são finitos.

## Degradação

O provedor pode estar offline, a credencial pode faltar, a rede pode falhar. Em todos esses casos
o devzymer continua utilizável: a decisão vem das regras locais e a resposta reporta
`provider=deterministic`. Desligar o Jev não remove nenhuma capacidade de recuperação.

```bash
devzymer doctor
```

A linha `decisões` informa se o provedor está habilitado e se a credencial está presente.
