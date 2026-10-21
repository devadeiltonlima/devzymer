# Edição segura

## Antes de editar

1. **Entenda a implementação.** Leia o símbolo inteiro com `context_read`, não um recorte.
2. **Contratos relacionados.** Se a assinatura, o retorno ou os efeitos mudam, rode
   `context_impact` e leia os chamadores diretos.
3. **Revisão atual.** O `sha` vem da própria leitura. Se o arquivo pode ter mudado desde então
   (você ou o usuário editaram, rodou formatador), leia de novo.
4. **Dentro do workspace.** Edite só arquivos do projeto em que trabalha e que o usuário
   autorizou. Não toque em segredos, arquivos de configuração do agente nem no índice.
5. **Só edite o que foi pedido.** Uma sugestão de busca ("isto parece um bug") não é pedido de
   mudança.

## `context_patch`

Contrato, tal como o servidor aplica:

- `expected_sha` é obrigatório e vem de `context_read`; o patch é rejeitado se o arquivo mudou.
- O intervalo precisa estar **dentro do trecho que foi lido**. Para editar uma função, leia a
  função (por `symbol` ou pela faixa) e edite dentro dela.
- Substituir o arquivo inteiro exige `allow_whole_file`, o que raramente é o certo: uma edição
  mínima é mais fácil de revisar.
- A escrita é atômica e preserva codificação e quebras de linha.
- `dry_run: true` valida sem escrever; use em mudanças não triviais.

Se for rejeitado, **leia de novo** e refaça o patch com o novo `sha`. Não force a barra com
`allow_whole_file` para passar pela verificação, e não edite por outro caminho para driblar a
proteção. Depois de um patch bem-sucedido o `sha` muda; para outra edição no mesmo arquivo,
leia novamente.

## Quando usar a edição nativa

O devzymer não é obrigatório para escrever. Prefira a ferramenta nativa quando: o arquivo é
novo; a mudança toca vários arquivos de forma mecânica (renomear); o arquivo não está no índice
(gerado, ignorado); o patch não consegue cumprir o contrato; ou o host pede outro fluxo. Nesses
casos, leia o arquivo como o host exige antes de editar.

Após criar, renomear ou apagar arquivos, chame `context_sync` para o índice acompanhar.

## Testes

Hooks do projeto podem impedir a criação de arquivos de teste novos sem aprovação. Estenda os
testes existentes quando houver; se for preciso um arquivo novo, peça ao usuário.

## Validar

Rode o que o projeto oferece, com as ferramentas nativas: testes do módulo, tipagem, lint,
build. Inclua o caso que motivou a mudança. Se nada foi executado, diga isso em vez de supor
que passou.

## Exemplo

```
context_read(path="billing/retry.ts", symbol="RetryJob.run")  → sha 9f0c1a77, linhas 30-72
context_patch(path="billing/retry.ts", start_line=41, end_line=41,
              replacement="    const result = await gateway.charge(order);\n    if (!result.ok) { await park(order); continue; }",
              expected_sha="9f0c1a77", dry_run=true)   → OK (dry run)
context_patch(... dry_run=false)                       → OK | sha 9f0c1a77 -> 31ab04de
```

Depois: rodar o teste de `billing`, relatar o comportamento novo (pedido recusado fica
estacionado em vez de ser descartado) e o que não foi coberto.
