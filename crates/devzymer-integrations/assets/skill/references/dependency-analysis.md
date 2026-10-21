# Dependências e impacto

## Workflow 4: o que quebra se esta função mudar

1. **Localizar o símbolo exato.** `context_resolve` com o nome qualificado. Se vier
   `ambiguidade:`, resolva antes de analisar impacto: o grafo de outro símbolo de mesmo nome
   responde a pergunta errada.
2. **Consultar as relações.** `context_impact` com `symbol` e `depth: 1`. Aumente para 2 só
   se os chamadores diretos não bastarem; profundidade grande devolve ruído.
3. **Separar verificado de heurístico.** Cada relação vem de análise estática. Trate como
   *verificada* só a que você confirmou lendo a linha da chamada; o resto é *candidata*.
4. **Chamadores diretos e contratos.** Leia o ponto de chamada de cada chamador direto: que
   argumentos passa, o que faz com o retorno, o que supõe (formato, nulidade, ordem, erros). Se
   a mudança altera assinatura, tipo de retorno, exceções ou efeitos colaterais, esses são os
   contratos em risco.
5. **Limitar a profundidade.** Declare até onde foi ("diretos e um nível indireto") em vez de
   sugerir cobertura total.
6. **Relatar só o impacto sustentado.** Liste: chamadores confirmados (com `arquivo:linha`),
   candidatos não confirmados, e o que o grafo não enxerga.

Não afirme que o grafo está completo a menos que consiga demonstrar. Uma checagem barata de
completude: busca nativa pelo nome do símbolo (`rg -w nome`) e comparar com os chamadores que o
grafo listou; cada diferença explica uma lacuna ou descarta um falso positivo.

## O que o grafo não enxerga

Despacho dinâmico e callbacks, injeção de dependência, registro por string (rotas, eventos,
nomes de handler), reflexão, decoradores e metaprogramação, código gerado, templates, arquivos
de configuração que referenciam símbolos, chamadas entre repositórios e testes que importam por
caminho. Nesses casos, aresta ausente **não** prova ausência de chamador.

## Por arquivo

`context_impact` com `path` mostra quem importa o arquivo e o que ele importa. Serve para
mudanças de módulo (mover, renomear, dividir), não para mudanças de uma função.

## Exemplo: verificação do grafo

```
context_impact(symbol="PaymentGateway.charge", depth=1)
→ checkout/service.ts:88   CheckoutService.pay        calls   heuristic
→ billing/retry.ts:41      RetryJob.run               calls   heuristic
```

Leitura: `checkout/service.ts:88` passa `{amount, currency}` e trata `ChargeError`; confirmado.
`billing/retry.ts:41` chama `charge` dentro de um laço e ignora o retorno: confirmado, e é onde
uma mudança de retorno quebraria. A busca `rg -w charge` mostra um terceiro uso em
`admin/refund.ts:12` que o grafo não listou (chamada via `gateway['charge']`). Relato: dois
chamadores confirmados, um adicional encontrado por busca nativa, grafo incompleto para
acesso por string.

## Modelo de relatório

```
Alvo: PaymentGateway.charge (payments/gateway.ts:30-71)
Confirmados: checkout/service.ts:88, billing/retry.ts:41
Candidatos não confirmados: admin/refund.ts:12 (acesso por string)
Contratos em risco: retorno {ok, id}; ChargeError para recusas
Limite da análise: profundidade 1; testes não inspecionados
```
