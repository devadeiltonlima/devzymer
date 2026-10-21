# Metodologia do benchmark

> **Visualizador animado:** [benchmark animado](https://devadeiltonlima.github.io/devzymer/docs/benchmark.html) mostra os resultados em animação —
> contexto entregue antes e depois do devzymer, precisão por repositório e o caso real que motivou o
> motor v2.1. Abre direto no navegador, sem servidor e sem dependências externas.

O benchmark mede **recuperação de código**: dada uma consulta, o quanto de contexto é
necessário para localizar a implementação correta, com que precisão e em que latência.

Ele **não** mede tokens de ponta a ponta do Claude Code. Essa medição exige instrumentação do
host e está descrita em [end-to-end-benchmark.md](end-to-end-benchmark.md).

## Cenários

Dez cenários independentes, cada um com uma expectativa própria.

| Cenário | Consulta | O que é medido |
| --- | --- | --- |
| `exact_symbol` | nome simples de um símbolo **único** no repositório | o símbolo esperado é o primeiro resultado |
| `qualified_symbol` | nome qualificado (`Container.metodo`) | o símbolo qualificado resolve na primeira posição |
| `filename_discovery` | nome do arquivo sem extensão | o arquivo é identificado e seus símbolos são devolvidos |
| `natural_language` | descrição comportamental | o símbolo descrito é recuperado |
| `ambiguous_symbol` | nome simples com várias definições | a ambiguidade é reportada, e não escondida |
| `dependency_traversal` | caminho de um módulo | os importadores são relacionados |
| `repeated_search` | a mesma consulta duas vezes | o cache de recuperação é acionado |
| `missing_symbol` | nome inexistente | nada é resolvido |
| `stale_reference` | nome qualificado | a faixa de linhas continua apontando para o símbolo |
| `index_freshness` | caminho de um arquivo | o índice reflete o arquivo no disco |

## Como as consultas são produzidas

**Consulta de identificador.** O nome do símbolo, como o usuário escreveria.

**Consulta de documentação.** O texto de documentação escrito **no arquivo de origem** é usado como
descrição. O identificador do símbolo é removido do texto, para que a consulta não revele a
resposta. Símbolos cuja documentação não existe, ou cuja documentação cita o próprio
identificador, são descartados. A origem fica registrada como `source_documentation`.

**Consulta curada.** Perguntas escritas à mão que descrevem comportamento sem nomear o símbolo.
A origem fica registrada como `curated`. As perguntas curadas de um repositório de produção estão em
`crates/devzymer-bench/src/tasks.rs` e só entram no relatório quando o arquivo e o símbolo
existem no repositório avaliado.

**Nunca** se converte um identificador em minúsculas e se chama isso de linguagem natural. Uma
consulta derivada de identificador é marcada como `identifier` e não conta como cenário de
compreensão semântica.

## Verdade de base independente

A expectativa de cada tarefa é validada contra o **arquivo no disco**, nunca contra o índice
avaliado:

1. o arquivo precisa existir;
2. o identificador esperado precisa aparecer no arquivo como palavra inteira;
3. quando há linha esperada, a linha e as três linhas vizinhas precisam conter o identificador.

Uma tarefa que não passa na validação é marcada como `unverified`, com o motivo, e **não entra
nas métricas**. O relatório mostra quantas foram verificadas e quantas não foram. Isso impede
que o benchmark confirme o próprio erro: um parser que produz a linha errada não valida a tarefa
que ele criou.

## Seleção reproduzível

A seleção usa um gerador pseudoaleatório determinístico com semente explícita
(`--seed`, padrão `20260319`). A semente, a versão do benchmark, o estimador de tokens e a
revisão do índice entram no relatório. Duas execuções com a mesma semente e o mesmo índice
selecionam as mesmas tarefas.

O relatório separa **tarefas únicas** de **execuções**: a repetição de uma consulta é uma
execução a mais, não uma evidência independente.

## Métricas

| Métrica | Definição |
| --- | --- |
| Acerto no top K | O símbolo esperado está entre os K primeiros candidatos |
| Recall@1 | O símbolo esperado é o primeiro candidato |
| Recall@5 | O símbolo esperado está entre os cinco primeiros |
| MRR | Média do inverso da posição do primeiro acerto |
| Latência | Tempo da chamada de resolução, com percentis |
| Tokens de contexto | Estimativa dos tokens do material entregue |
| Tokens da linha de base | Estimativa dos tokens do **arquivo inteiro** que contém a resposta |
| Redução | Com sinal: `(linha de base - contexto) / linha de base` |

A janela de candidatos é de 50, maior que o top K. Isso permite distinguir "não foi recuperado"
de "recuperado na posição 8". A posição real e a lista de candidatos entram no relatório de cada
tarefa, para que uma falha seja diagnosticável de forma independente.

## Aritmética da redução

A redução é calculada em ponto flutuante com sinal:

```rust
if baseline_tokens == 0 {
    return None;                       // sem denominador, sem porcentagem
}
let value = (baseline_tokens as f64 - context_tokens as f64) / baseline_tokens as f64 * 100.0;
if !value.is_finite() { return None; }
Some(value)
```

Não existe subtração não sinalizada em nenhum nível. Um valor negativo é legítimo: significa
que o contexto devolvido é maior que a leitura do arquivo inteiro. Isso aparece como
`-22,30%`, e não como `1,3271e19%`.

Tarefas sem linha de base são contadas em `tasks_without_baseline` e excluídas da porcentagem
agregada, que fica sem valor quando o total de linha de base é zero.

## Cache de recuperação

A resolução é memorizada por revisão do índice. O relatório traz acertos, falhas, e a latência
média de cada caso.

O cache **não** economiza tokens: um acerto devolve exatamente o mesmo contexto de uma falha. O
relatório registra `token_saving_from_cache_pct` como `0,00%` e explica isso nas notas. Confundir
cache computacional com economia de tokens seria uma leitura incorreta do resultado.

A invalidação é conservadora: qualquer alteração no índice incrementa a revisão e descarta todas
as entradas de uma vez. O cache nunca serve um resultado de uma revisão anterior.

A chave do cache carrega também a **versão do algoritmo de recuperação**. Uma mudança no
resolver descarta as entradas antigas, porque a revisão do índice não muda quando só o código
muda. Sem essa chave, um resultado produzido pela versão anterior do algoritmo continuaria
sendo servido, e a correção de um defeito de ranqueamento não apareceria.

## Estimador de tokens

O estimador é local e aproximado: `heuristic-chars-v1`, quatro caracteres por token com um
ajuste por palavra. Ele **não** é o tokenizador do Claude.

As contagens são estimativas de contexto entregue, e o relatório diz isso no campo
`token_estimator`. Nenhuma delas é consumo real do modelo.

## Resultado medido

Repositório real (TypeScript e Supabase Edge Functions), 807 arquivos indexados, 24.664
símbolos, semente `20260319`, 20 tarefas por cenário.

| Métrica | Valor |
| --- | --- |
| Tarefas únicas / execuções | 131 / 131 |
| Verificadas / não verificadas | 106 / 0 |
| Acerto no top 5 | 102 (96,2%) |
| Recall@1 | 64,2% |
| Recall@5 | 66,0% |
| MRR | 0,649 |
| Latência média / p50 / p95 / p99 | 1,7 ms / 1,0 ms / 6,0 ms / 9,0 ms |
| Tokens de contexto | 62.912 |
| Tokens da linha de base | 379.861 |
| Redução estimada | +83,44% |
| Tarefas com redução negativa | 4 |
| Tarefas sem linha de base | 20 |
| Cache: latência fria / quente | 0,00 ms / 0,00 ms |
| Cache: economia de tokens | 0,00% |

Por cenário:

| Cenário | Tarefas | Acertos | Recall@1 | Recall@5 | Redução |
| --- | --- | --- | --- | --- | --- |
| `exact_symbol` | 20 | 20 | 95,0% | 100,0% | +94,31% |
| `qualified_symbol` | 20 | 20 | 100,0% | 100,0% | +74,12% |
| `natural_language` | 5 | 1 | 0,0% | 20,0% | +49,47% |
| `ambiguous_symbol` | 20 | 20 | n/a | n/a | n/a |
| `dependency_traversal` | 20 | 20 | n/a | n/a | +92,41% |
| `repeated_search` | 1 | 1 | 100,0% | 100,0% | +98,55% |
| `missing_symbol` | 5 | 5 | n/a | n/a | n/a |
| `stale_reference` | 20 | 20 | 100,0% | 100,0% | +74,14% |
| `index_freshness` | 20 | 20 | n/a | n/a | +97,20% |

### Leitura do resultado

**O `exact_symbol` passou de 18/20 para 20/20** porque o cenário passou a escolher nomes únicos.
A versão anterior media o mesmo símbolo em duas categorias, com a consulta feita pelo nome simples
de um símbolo que tinha dez definições no repositório. O alvo esperado estava na posição 8, fora
do corte top-5. A tarefa agora é bem definida, e a ambiguidade tem cenário próprio.

**O `natural_language` é o ponto fraco, com honestidade.** Uma das cinco perguntas curadas
resolve; as outras quatro ficam fora do top-5 ou não são recuperadas. A recuperação lexical
compara termos; uma descrição em prosa usa palavras diferentes das do código
("registro" contra `Record`, "limitador" contra `limitar`). Essa é a fronteira em que a busca
por embeddings ajudaria, e ela está declarada como limitação, não disfarçada.

**O cache economiza trabalho, não tokens.** Com os índices corrigidos, a recuperação a frio e a
quente têm latência equivalente, e a economia de tokens do cache é 0,00%: o mesmo contexto é
devolvido. Isso é registrado no relatório.

**Nove tarefas têm redução negativa** e vinte não têm linha de base. As negativas significam
que o contexto devolvido era maior que a leitura do arquivo inteiro, o que é um resultado
legítimo de uma consulta ampla. As vinte sem linha de base são as que não têm um arquivo
associado, e ficam fora da porcentagem agregada.

## O que o benchmark não prova

- Não prova economia de tokens do Claude Code.
- Não prova taxa de sucesso de tarefa.
- Não mede o custo do provedor de decisão.
- Não mede a qualidade da alteração de código, apenas a capacidade de localizar o código
  certo.
