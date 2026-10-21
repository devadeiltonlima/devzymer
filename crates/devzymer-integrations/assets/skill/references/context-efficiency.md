# Eficiência de contexto

Meta: o contexto necessário para a próxima decisão correta, sem mais nem menos. Economia nunca
justifica mudar o resultado: se o recorte pequeno não sustenta a conclusão, leia mais.

## Regras práticas

1. **Símbolo antes de arquivo.** `context_read` com `symbol` traz a função; arquivo inteiro só
   para editar um arquivo curto ou quando a estrutura do arquivo é o assunto.
2. **Leia a função completa uma vez**, em vez de três recortes que se sobrepõem. Se precisa do
   contexto ao redor, peça uma faixa maior de uma só vez.
3. **Respeite a continuação.** Se a resposta traz `truncated` e `next_start_line`, continue dali
   com uma nova leitura; não releia o início.
4. **Estreite a busca.** `path` e `language` em `context_resolve` cortam candidatos. `limit`
   pequeno (3 a 5) quando você sabe que quer o melhor resultado.
5. **Não repita.** Uma leitura que você já fez e cujo `sha` não mudou não precisa ser refeita.
   O cache de `context_resolve` poupa o banco, não tokens: repetir a consulta gasta o mesmo.
6. **Poucos status.** `context_status` uma vez no início de uma investigação, e de novo só
   depois de um sinal concreto (resultado vazio inesperado, caminho que não existe).
7. **Chamadas independentes em paralelo.** Duas leituras de símbolos diferentes não dependem
   uma da outra.
8. **Impacto em camadas.** `depth: 1` primeiro; aumente só com motivo.
9. **Relatório curto.** Cite `arquivo:linha` e a conclusão; não cole funções inteiras na
   resposta se o usuário só precisa saber onde está.

## Orçamento de resposta

As respostas do devzymer já são compactas e a expansão é pedida, não automática. Se uma
chamada volta truncada, a pergunta certa é "preciso do resto?", não "reexecuto maior". Se
`RESOURCE_LIMIT_EXCEEDED` aparecer, reduza `limit`, `max_bytes` ou restrinja `path`.

## Exemplo

Incorreto, 4 chamadas e metade redundante:

```
context_resolve("calcularTotal")      context_resolve("calcularTotal")   # repetida
context_read(path, 10, 40)            context_read(path, 30, 70)         # sobrepostas
```

Correto, 2 chamadas:

```
context_resolve("calcularTotal", path="pricing")
context_read(path="pricing/total.ts", symbol="calcularTotal")
```

## Sobre economia de tokens

Não afirme quantos tokens foram poupados sem medição. Diga o que fez ("li só a função
`calcularTotal`, 38 linhas") e deixe o número para quem mede.
