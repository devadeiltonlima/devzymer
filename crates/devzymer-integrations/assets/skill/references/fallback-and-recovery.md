# Falhas, fallback e recuperação

Princípio: se o devzymer não dá evidência confiável, diga por quê, siga com a busca nativa e
volte quando fizer sentido. Não tente de novo às cegas, não esconda um índice ruim e não repare
nada sem autorização.

## Resposta da ferramenta → ação

| Sinal | Significa | Faça |
| --- | --- | --- |
| `nao_encontrado:` / `SYMBOL_NOT_FOUND` | nenhum símbolo casou (o servidor já sincronizou e repetiu a busca) | não conclua "não existe"; confirme com busca nativa e reformule só com o que aprendeu |
| `ambiguidade:` / `AMBIGUOUS_MATCH` | o nome tem várias definições | use o nome qualificado ou `arquivo nome`; não escolha ao acaso |
| `STALE_INDEX` | o arquivo mudou desde a indexação | `context_sync`, repita a leitura |
| `INVALID_PATCH` | `sha` velho, faixa fora do trecho lido, caminho fora do workspace | releia e refaça o patch com o `sha` novo, sem `allow_whole_file` para contornar |
| `PERMISSION_DENIED` | política de privacidade ou permissão | respeite; não contorne; use outro caminho legítimo |
| `PATH_ESCAPE` | caminho fora do workspace | corrija o caminho; não saia da raiz |
| `RESOURCE_LIMIT_EXCEEDED` | limite de resposta | peça menos (`limit`, `max_bytes`, `path`) |
| `UNSUPPORTED_LANGUAGE` | sem gramática | leia com a ferramenta nativa |
| ferramenta `context_*` ausente / MCP não conectado | servidor indisponível | busca nativa; informe que o MCP não está ativo |
| `context_status` com falhas, `integrity` diferente de `ok`, ou "outro workspace" | índice corrompido ou de outro projeto | veja "Índice corrompido" |

Uma falha não justifica repetir a mesma chamada. Se `context_sync` não resolver, a busca nativa
é o caminho, e você diz isso.

## Workflow 6: pesquisa com fallback nativo

Use a busca nativa quando: o índice está defasado e o sync não resolveu; o arquivo é
intencionalmente excluído (gerado, ignorado, segredo); a consulta é sobre logs ou saída de
comandos; o MCP está indisponível; ou o usuário pediu busca nativa. Registre a razão em uma
frase. Depois, se o índice voltar a responder (`context_status` saudável), retome o devzymer.

## Hooks de roteamento

Em `guard`, o hook pode redirecionar **uma** busca nativa para o devzymer e libera a repetição.
Se o devzymer estiver indisponível, defasado ou tiver falhado, a busca nativa passa. Se uma
busca foi redirecionada e o devzymer não ajuda, repita a busca nativa: ela passa. Não fique em
laço; duas tentativas bastam para concluir que o caminho certo é o nativo.

## Projeto sem índice: inicialização autorizada

1. **Detecte.** As ferramentas `context_*` ausentes, ou `context_status` informando que não há
   índice, ou o aviso de que o índice não existe.
2. **Confirme a raiz.** O diretório atual é a raiz do projeto em que você trabalha (tem `.git`
   ou um manifesto). Nunca rode a partir da pasta pessoal do usuário e nunca inicialize um
   projeto enquanto opera em outro. Se estiver em dúvida, use `--root <diretório>` explícito.
3. **Confirme a autorização.** O pedido do usuário e as permissões do host cobrem rodar
   `devzymer init`? Se não, informe a falta e use as ferramentas nativas.
4. **Execute uma vez.** `devzymer init`. Não rode `init` dentro do próprio init, nem repita se
   já houver índice.
5. **Verifique.** `context_status` (ou `devzymer status`) com cobertura e integridade `ok`.
6. **Continue a tarefa original.** Se o servidor MCP só carrega no início da sessão, avise que
   o agente precisa ser reiniciado para ver as ferramentas e use a busca nativa até lá.

## Índice corrompido ou de outro workspace

Não rode `init`, `sync --full` nem apague `.devzymer/` por conta própria. Mostre o
diagnóstico (`devzymer doctor`), explique que a busca nativa será usada, e ofereça o reparo:
`devzymer init --repair` guarda o índice danificado ao lado (não o apaga) e reconstrói. Só
execute com autorização do usuário.

## Depois de uma falha

Retome o devzymer quando `context_status` indicar índice íntegro e sem pendências, ou depois
de um `context_sync` bem-sucedido. Não faça isso por reflexo a cada passo.
