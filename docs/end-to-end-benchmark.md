# Benchmark ponta a ponta com o Claude Code

O benchmark de recuperação (`devzymer benchmark`) mede **tokens estimados de contexto** e
**precisão da busca**. Ele não mede o consumo real do Claude Code.

Esta página descreve o procedimento para medir o consumo real. Ele é separado do benchmark de
recuperação porque depende do host, não do devzymer.

## Por que a separação

| Métrica | Benchmark de recuperação | Benchmark ponta a ponta |
| --- | --- | --- |
| Fonte dos tokens | estimador local (o200k) | contagem do host |
| Entrada e saída do modelo | não mede | mede |
| Cache de tokens do host | não mede | mede |
| Custo por tarefa | não mede | mede |
| Sucesso da tarefa | não mede | mede |
| Regressão de qualidade | indício | verificação |

Publicar um percentual de economia de tokens derivado do benchmark de recuperação como se fosse
economia do Claude Code é incorreto. O benchmark de recuperação mede a ponte; este mede o
resultado.

## Procedimento

### 1. Preparar o conjunto de tarefas

Escreva um conjunto de tarefas que descrevam comportamento, não implementação. Cada tarefa precisa
de:

- um prompt que um engenheiro faria;
- um critério objetivo de conclusão (o arquivo foi alterado, o teste passa, a resposta cites a
  linha certa);
- o estado inicial do repositório.

Use o mesmo conjunto de tarefas nos dois braços. Versione o conjunto junto com o repositório.

### 2. Fixar as condições

Para que a comparação valha, os dois braços precisam diferir apenas na presença do devzymer:

- mesmo modelo e mesma versão do modelo;
- mesmo nível de esforço;
- mesmo contexto inicial (as mesmas instruções, o mesmo `CLAUDE.md`);
- mesmas permissões de ferramenta;
- mesma temperature, quando configurável;
- mesma máquina, executados em horários próximos, para não misturar latência de rede;
- a mesma revisão do repositório no início de cada execução.

### 3. Executar cada braço

```bash
# Braço A: com devzymer
claude -p "<tarefa>" --output-format stream-json --verbose > A.jsonl

# Braço B: sem devzymer
#   desative o servidor MCP do devzymer antes de iniciar a sessão
claude -p "<tarefa>" --output-format stream-json --verbose > B.jsonl
```

Repita cada tarefa em ambos os braços pelo menos cinco vezes, alternando a ordem entre execuções,
para separar o efeito da ferramenta do efeito da variação entre execuções.

### 4. Extrair as métricas

O formato `stream-json` expõe, por requisição:

| Métrica | Origem |
| --- | --- |
| Tokens de entrada | `usage.input_tokens` |
| Tokens de cache lidos | `usage.cache_read_input_tokens` |
| Tokens de cache escritos | `usage.cache_creation_input_tokens` |
| Tokens de saída | `usage.output_tokens` |
| Tokens de raciocínio | `usage` do host, quando exposto |
| Chamadas de ferramenta | eventos `tool_use` |
| Leituras de arquivo inteiro | eventos `tool_use` com `Read` sem faixa |
| Leituras direcionadas | eventos `tool_use` com `Read` por faixa |
| Consultas ao índice | eventos `tool_use` com `mcp__devzymer__*` |
| Latência por requisição | carimbo de tempo do evento |
| Custo | tokens multiplicados pelo preço do modelo |

Some as métricas por tarefa e depois por execução. Reporte a média e o intervalo de confiança
das repetições, não apenas a média.

### 5. Custo total

O custo de uma tarefa inclui tudo:

```
custo_total = custo_do_modelo
             + custo_das_chamadas_de_decisao
             + custo_de_infraestrutura
```

As chamadas de decisão usam o Jev quando habilitado. Some o `usage.input_tokens` devolvido pelo
provedor de decisão, que o `context_decide` expõe no resultado estruturado.

### 6. Sucesso da tarefa

Sem uma verificação de sucesso, a economia de tokens é um número sem significado: a execução
pode ter economizado tokens e não ter feito o trabalho.

Para cada tarefa, verifique objetivamente:

- a alteração esperada está presente no arquivo esperado;
- o comportamento verificável foi preservado (testes, lint, build);
- a resposta do agente cita os caminhos e linhas corretos.

Uma tarefa que falhou conta como falha, mesmo que tenha custado menos tokens.

### 7. Reportar

Para cada braço, reporte:

| Métrica | Valor |
| --- | --- |
| Tarefas | N |
| Tarefas concluídas | N (taxa de sucesso) |
| Tokens de entrada por tarefa concluída | média e intervalo |
| Tokens de saída por tarefa concluída | média e intervalo |
| Leituras de arquivo inteiro por tarefa | média |
| Consultas ao índice por tarefa | média |
| Latência por tarefa | p50, p95 |
| Custo por tarefa concluída | médio |

A métrica principal é o **custo por tarefa concluída**, não o custo por tarefa. As duas divergem
quando a taxa de sucesso muda.

## Limitações conhecidas desta medição

- A variação entre execuções do mesmo modelo é relevante. Poucas repetições não separam o
  efeito da ferramenta do ruído.
- O preço do modelo e o do provedor de decisão mudam. Registre o preço usado.
- O cache de tokens do host é contabilizado à parte, e o preço do cache lido é diferente do
  preço da entrada comum. Tratar os dois como iguais superestima a economia.
- A ordem das execuções pode vazar aprendizado entre repetições. Alternar a ordem reduz o
  efeito, mas não o elimina.
- Se o provedor de decisão estiver habilitado em apenas um dos braços, a diferença inclui o
  custo e o efeito das chamadas de decisão, e não apenas o do índice.
