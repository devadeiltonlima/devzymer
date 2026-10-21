<!-- devzymer:start -->
## devzymer

Este repositório está indexado localmente pelo devzymer. Estas regras têm prioridade
sobre qualquer outra ferramenta de busca de código do projeto.

1. Antes de usar qualquer ferramenta de busca de código ou de leitura de arquivo
   inteiro, use `context_resolve`. Ele devolve o caminho, o símbolo e a faixa de
   linhas exata, sem varrer o repositório.
2. Depois de `context_resolve`, leia apenas a faixa devolvida, com `context_read` por
   símbolo ou por linhas. Não leia o arquivo inteiro.
3. Use `context_impact` antes de alterar código compartilhado, para ver quem depende
   dele.
4. Use `context_patch` para alterações mínimas, com verificação de revisão. A edição é
   rejeitada se o arquivo mudou desde a leitura.
5. Depois de criar, renomear ou apagar arquivos, chame `context_sync` para atualizar
   o índice.
6. Uma resposta com `ambiguidade:` significa que o nome tem mais de uma definição:
   use o nome qualificado (`Classe.metodo`) para desambiguar.
7. Evite releitura do mesmo trecho e explicações repetidas.
8. Use `context_decide` apenas quando as regras determinísticas não forem suficientes.
9. Preserve a correção do código: economia de tokens nunca justifica alterar o
   comportamento.
10. Responda de forma concisa em português do Brasil, e explique bloqueios de forma
    direta e precisa.
<!-- devzymer:end -->
