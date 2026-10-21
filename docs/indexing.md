# Indexação e sincronização

## Visão geral da varredura

A varredura é feita pela biblioteca `ignore`, que aplica a semântica de `.gitignore`:

- `.gitignore` de cada diretório e dos diretórios ancestrais
- `.git/info/exclude`
- `.ignore`
- `.devzymerignore`, exclusivo do devzymer
- a lista `index.exclude` da configuração

A varredura é paralela, com o número de threads definido por `index.max_workers` ou pela
capacidade paralela da máquina. A travessia é estável: o resultado é ordenado por caminho.

## O que é ignorado

| Motivo | Regra |
| --- | --- |
| Ignorado por configuração | Casa com `index.exclude` |
| Grande demais | `index.max_file_size_bytes` |
| Binário | Byte nulo nos primeiros 8 KB |
| Oculto | `index.include_hidden` é `false` |
| Symlink fora do workspace | Rejeitado quando `follow_symlinks` é `true` |
| Linguagem filtrada | `index.languages` está definida e a linguagem não consta |
| Arquivo gerado conhecido | `package-lock.json`, `Cargo.lock`, `yarn.lock` e afins |

O relatório de varredura guarda o motivo de cada exclusão, disponível em `devzymer init
--json` e no relatório do indexador.

## Estágios

A indexação acontece em estágios, para que o repositório fique pesquisável antes de todo o
trabalho caro terminar:

1. **Descoberta de arquivos** — travessia e aplicação de filtros.
2. **Metadados** — tamanho, data de modificação, hash de conteúdo, linguagem.
3. **Parsing estrutural** — extração de símbolos por Tree-sitter, em blocos paralelos.
4. **Análise de dependências** — resolução de importações e construção do grafo.
5. **Análise semântica avançada** — reservada; hoje é o provedor de decisão opcional.

## Memória e escala

O índice nunca exige que o repositório inteiro caiba na memória.

- Os arquivos são processados em blocos de 256; o resultado de cada bloco é gravado e o bloco é
  descartado antes do próximo.
- A gravação acontece em transação, com commit ao fim de cada bloco.
- As consultas ao banco usam paginação e limites explícitos.
- O grafo é reconstruído a partir do índice e limitado por arquivo.

O alvo de memória é `index.memory_budget_mb`. O uso real depende do tamanho médio dos arquivos
e da complexidade da gramática, e não é garantido.

Limites reais e inevitáveis: capacidade de disco, limites do sistema de arquivos, permissões e
tempo de processamento. Não há garantia de escala ilimitada.

## Detecção de mudanças

Cada arquivo indexado guarda tamanho, data de modificação e hash de conteúdo (BLAKE3).

- **Data e tamanho iguais** — o arquivo é considerado inalterado e não é reparseado.
- **Data ou tamanho diferentes** — o arquivo é reparseado e o hash recalculado. Se o hash
  novo for igual ao anterior, o conteúdo não mudou de fato.
- **Arquivo ausente** — as entradas são removidas do índice e as relações do arquivo são
  apagadas.
- **Renomeação** — detectada quando o hash de um arquivo removido reaparece em um arquivo novo.

A sincronização incremental é o caminho normal. A reconstrução completa é necessária quando:

- o banco está corrompido;
- a versão do parser mudou;
- a configuração de exclusões mudou de forma incompatível;
- a estrutura do índice foi alterada por uma migração.

## Sincronização contínua

`devzymer watch` observa a árvore com `notify` e sincroniza após cada rajada de alterações, com
debounce de 800 ms. Após cada sincronização, o grafo é reconstruído.

O watcher trata overflow de eventos, eventos perdidos e trocas de branch do git de forma
conservadora: quando a contagem de arquivos pendentes diverge do esperado, o devzymer assume
pior caso e reindexa o que mudou, sem nunca devolver uma referência de linha obsoleta sem
aviso.

## Sincronização automática

Um `devzymer init` basta para o uso normal. A manutenção do índice é feita pelo servidor MCP,
que roda três mecanismos:

**1. Observador de filesystem.** Enquanto o servidor está vivo, um observador (`notify`) acompanha
a árvore. Cada evento de criação, alteração ou remoção é filtrado, agrupado com um debounce de
700 ms e processado. O processamento reindexa **apenas os arquivos que mudaram**, comparando o
hash de conteúdo: um arquivo cujo conteúdo não mudou tem apenas os metadados atualizados. As
relações do grafo são recalculadas para os arquivos afetados, e as arestas que apontam para
símbolos removidos são podadas.

**2. Reconciliação na abertura da sessão.** Quando o servidor MCP inicia, ele compara o índice com
uma varredura de metadados do disco. Isso recupera eventos perdidos enquanto o servidor estava
parado. A reconciliação roda em segundo plano e não bloqueia a primeira requisição.

**3. Verificação de frescor na recuperação.** Antes de devolver uma referência de linha, o
servidor confere o `mtime` e o tamanho do arquivo contra o índice. Se mudaram, o arquivo é
reindexado e a busca é refeita. Uma linha obsoleta nunca é devolvida como resultado verificado.

Quando uma busca falha, o servidor executa uma sincronização limitada, com orçamento de tempo
(`index.sync_budget_ms`), e repete a busca antes de responder `SYMBOL_NOT_FOUND`. O orçamento e um
intervalo mínimo entre sincronizações automáticas impedem que uma consulta ausente vire uma
varredura completa repetida.

O `context_sync` permanece disponível para recuperação explícita.

## Sinais de obsolescência

Uma referência só é devolvida sem ressalva quando o hash do arquivo confere. `context_read`
sempre devolve o hash, e `context_patch` aceita um `expected_sha` e rejeita a edição quando ele
não confere. É assim que uma linha obsoleta é detectada em vez de aplicada sobre um arquivo que
mudou.

## Remoção conservadora

Um arquivo é removido do índice apenas quando **não existe mais no disco**. Se ele existe mas
não apareceu na varredura, por estar sendo escrito no momento, ele permanece indexado e a
ocorrência é reportada em `skipped` com o motivo. Isso evita que uma falha transitória de leitura
apague símbolos válidos.

## Cache de recuperação

A resolução é memorizada na tabela `retrieval_cache`, com a chave derivada da consulta, do
caminho, da linguagem, do limite e da **revisão do índice**. Uma alteração em qualquer arquivo
incrementa a revisão, o que invalida todas as entradas de uma vez. A invalidação é
conservadora: o cache nunca serve um resultado de uma revisão anterior.

O cache economiza consultas ao banco, não tokens: um acerto devolve exatamente o mesmo contexto
que uma falha devolveria. Medir o cache como economia de tokens seria um erro de interpretação.

## Migração do esquema

O esquema do banco tem versão. A versão é gravada na tabela `meta`, junto com a versão do
parser e a data de criação. Ao abrir um banco com versão anterior, a migração correspondente é
aplicada na inicialização.

A migração para a versão 2 acrescenta a coluna `files.stem` (o nome do arquivo sem extensão,
minúsculo) com o índice correspondente, e preenche o valor dos arquivos já indexados. A coluna
existe para responder a consultas que nomeiam um arquivo sem extensão, sem varrer a tabela de
arquivos. Os dados indexados são preservados: nenhum símbolo é descartado na migração.
