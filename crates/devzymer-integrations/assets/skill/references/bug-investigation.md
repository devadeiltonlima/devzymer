# Investigação e correção de bugs

## Workflow 2: procurar um bug sem pista

Pedido: "Ache um bug real neste repositório." O objetivo é um defeito verificável, nunca uma
lista de suspeitas.

1. **Saúde do workspace.** `context_status` uma vez. Se o índice está defasado ou incompleto,
   `context_sync` antes de tirar conclusões; se o sync não resolver, trate como índice defasado e use a busca nativa.
2. **Caminhos de risco.** Priorize onde um erro custa caro e onde erros costumam morar:
   autenticação e autorização, entrada externa (parsing, validação), dinheiro e contagem,
   datas e fusos, concorrência e estado compartilhado, E/S e liberação de recursos, tratamento
   de erro que engole exceções, montagem de SQL/comando/caminho, migrações.
3. **Recuperar candidatos.** `context_resolve` por área ("validação do token de sessão",
   "cálculo de total do pedido") e leia as funções inteiras.
4. **Traçar o fluxo.** Siga a entrada até o efeito: quais condições levam à linha suspeita? O
   que o chamador garante? `context_impact` mostra quem alimenta a função.
5. **Hipótese testável.** Escreva: "Com entrada E, a linha L produz R em vez de R'." Se não dá
   para escrever assim, ainda não é um bug, é uma suspeita.
6. **Tente refutar.** Procure o que impede o caso: validação a montante, tipo que não admite o
   valor, teste existente, documentação de contrato. Um bug que sobrevive à refutação é forte.
7. **Validar com segurança.** Se autorizado, reproduza com um teste existente, um script
   descartável ou raciocínio sobre entradas concretas. Não altere dados reais, não rode contra
   produção, não crie arquivos de teste novos sem aprovação do usuário.
8. **Classificar.** *Confirmado* (reproduzido ou demonstrado linha a linha), *provável* (fluxo
   leva ao defeito, sem execução), *inconclusivo* (há dúvida que você não consegue fechar).

Não fabrique defeitos. "Nenhum bug confirmado nesta área; verifiquei X e Y" é um resultado
válido e mais útil do que um falso positivo.

### Exemplo de diagnóstico

```
Hipótese: calcularTotal(itens) soma arredondando cada linha; com 3 itens de 0,335 o total
vira 1,00 em vez de 1,01.
Evidência: pricing/total.ts:22 usa Math.round(item.preco*100)/100 dentro do reduce.
Refutação tentada: o contrato em pricing/types.ts:8 diz "preço com 2 casas"; o checkout
(checkout.ts:55) passa preços do catálogo, que vêm com 3 casas em promoções (catalog.ts:130).
Resultado: provável. Não executei; um teste com [0.335, 0.335, 0.335] confirmaria.
```

## Workflow 3: corrigir um bug identificado

1. **Reproduzir com segurança.** Um teste que falha ou um passo a passo determinístico. Sem
   reprodução, diga que a correção é baseada em raciocínio.
2. **Traçar o caminho real de execução.** Leia a função, seus chamadores diretos
   (`context_impact`) e o contrato (tipos, interfaces, documentação).
3. **Restrições e testes existentes.** Procure testes do módulo e invariantes que a correção
   precisa preservar (segurança, concorrência, compatibilidade de formato).
4. **Menor correção correta.** Mude o necessário. Não aproveite para refatorar. Para o patch,
   siga o protocolo de edição segura do `SKILL.md` (item 6).
5. **Preservar garantias.** Verificações de autorização, travas, transações e limites ficam
   como estavam, a menos que o bug esteja justamente nelas.
6. **Rodar as verificações relevantes.** Testes do módulo, tipagem, lint, com as ferramentas
   nativas. Inclua o caso que falhava.
7. **Relatar.** O que mudou no comportamento, o que foi validado, o que não foi, e riscos
   residuais (chamadores indiretos, dados já gravados com o comportamento antigo).
