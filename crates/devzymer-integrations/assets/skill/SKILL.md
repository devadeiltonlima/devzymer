---
name: devzymer
description: >-
  Use when investigating or changing code in an indexed repository: locating files,
  functions, symbols or implementations, tracing callers and dependencies, hunting or
  fixing bugs, assessing what a change breaks, or getting oriented in an unfamiliar
  codebase. Drives the devzymer MCP (context_resolve, context_read, context_impact,
  context_patch, context_sync) to retrieve precise excerpts instead of repeated grep or
  whole-file reads. Not for running tests, builds, git, logs or command output.
---
<!-- devzymer-skill-version: 1.0.0 -->

# devzymer: inteligência de repositório

O devzymer indexa o repositório localmente e responde por MCP com caminho, símbolo e faixa de
linhas. Esta skill diz **quando** usar cada ferramenta e **como** transformar o resultado em
evidência. O pedido do usuário, as permissões do host e as regras do projeto valem mais do que
estas preferências.

## Quando usar e quando não usar

Use para localizar código, entender como algo funciona, achar a origem de um bug, medir o
impacto de uma mudança, preparar uma edição e se orientar em um repositório novo.

Não use para: rodar testes, builds ou git (ferramentas nativas), ler logs e saída de comandos,
inspecionar arquivos fora do índice (gerados, ignorados, binários), ou quando o usuário pediu
busca nativa. Não chame o MCP só para parecer que o usou.

## Ciclo

Entender → Localizar → Inspecionar → Verificar → Agir → Validar → Relatar.

Nem toda tarefa precisa de todas as etapas. Símbolo exato: `context_resolve` + `context_read` e
pare. Explicação: nada de edição. Bug em produção: inclua impacto e validação. Pare de buscar
quando a evidência já sustenta a próxima decisão; continue quando ela não sustenta.

## Protocolo

1. **Objetivo antes da ferramenta.** "Onde fica X" é descoberta; "como funciona" é localizar,
   inspecionar e traçar; "ache um bug" é investigar e tentar refutar; "corrija" acrescenta edição
   segura e validação; "o que quebra se mudar" é impacto; "rode os testes" é ferramenta nativa.
2. **Recuperar antes de explorar.** `context_resolve` com o pedido do usuário, depois
   `context_read` só da faixa necessária, depois `context_impact` quando as relações importam.
   Amplie o contexto só se a evidência estiver incompleta.
3. **Busque comportamento, não só nomes.** Comece pela intenção original (ação, objetos,
   restrições do domínio). Não invente nomes de função antes de recuperar. Confira se o
   candidato realmente implementa o comportamento. Não reformule a mesma consulta sem ter
   aprendido algo com o resultado anterior.
4. **Evidência acima de confiança.** Separe: fonte verificada (você leu o código), inferência
   estrutural (grafo), inferência léxica (texto casado) e suposição. `confirmed` diz que a
   ligação nome/caminho foi confirmada, não que o código está correto. Aresta `heuristic` não é
   dependência confirmada, e aresta ausente não prova que não há chamador.
5. **Menor contexto suficiente.** Símbolo antes de arquivo inteiro; uma leitura completa da
   função em vez de vários trechos sobrepostos; respeite `max_bytes` e `next_start_line`.
6. **Edição segura.** Entenda o alvo, leia o trecho, edite o mínimo, valide. `context_patch`
   exige o `expected_sha` da leitura; nunca contorne essa proteção. Detalhes em
   `references/safe-editing.md`.
7. **Fallback honesto.** Se o devzymer não der evidência confiável, diga por quê, use a busca
   nativa e volte ao devzymer quando fizer sentido. Não esconda índice corrompido ou
   desatualizado e não o repare por conta própria.

## Qual referência carregar

Carregue só a que a tarefa pede; cada uma é independente.

| Situação | Leia |
| --- | --- |
| Escolher ferramenta, formular consulta, combinar chamadas | `references/tool-selection.md` |
| Achar uma implementação ou se orientar em repositório novo | `references/repository-discovery.md` |
| Procurar, confirmar ou corrigir um bug | `references/bug-investigation.md` |
| Quem chama, de quem depende, o que quebra | `references/dependency-analysis.md` |
| Alterar código | `references/safe-editing.md` |
| Resultado vazio, ambíguo, defasado, erro de MCP, projeto sem índice | `references/fallback-and-recovery.md` |
| Respostas grandes, repetição, orçamento de contexto | `references/context-efficiency.md` |

## Exemplos de roteamento

Correto: "ache onde a sessão do usuário é validada" → `context_resolve` com essa frase → lê a
faixa do candidato → confere que ele valida mesmo a sessão → responde com `arquivo:linha`.

Incorreto: ler três arquivos inteiros "para entender" antes de qualquer `context_resolve`; ou
chamar `context_status` a cada passo numa sessão saudável; ou inventar `validateUserSession` e
buscar esse nome.

Ambíguo: `ambiguidade: 'parse' tem 12 definições` → não escolha uma ao acaso. Use o nome
qualificado (`Container.parse`) ou acrescente o arquivo à consulta (`config.ts parse`).

Fallback: `context_resolve` volta vazio para algo que o usuário garante existir → `context_sync`
uma vez; se continuar vazio, busca nativa, dizendo que o índice não cobre o trecho.

## Projeto sem índice

Se as ferramentas `context_*` não existem ou o índice não foi criado, o projeto pode não estar
inicializado. Só rode `devzymer init` se o diretório for a raiz do projeto em que você está
trabalhando (nunca a pasta do usuário nem outro projeto), o pedido do usuário e as permissões o
autorizarem, e o projeto ainda não estiver inicializado. Rode uma vez, confira com
`context_status` e siga a tarefa. Sem autorização, informe a falta e use as ferramentas
nativas. Índice corrompido não é motivo para init automático: veja
`references/fallback-and-recovery.md`.

## Relatório

Entregue achados com `arquivo:linha`, diga o que foi verificado e o que é inferência, e não
declare sucesso só porque uma ferramenta foi chamada. Registre limitações: índice não cobre,
grafo pode estar incompleto, teste não executado.
