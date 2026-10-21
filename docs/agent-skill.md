# Skill e instruções do agente

O `devzymer init` deixa no projeto três coisas que ensinam um agente a usar o índice:

1. um bloco curto em `CLAUDE.md` (Claude Code) e em `AGENTS.md` (Codex e OpenCode);
2. a skill `devzymer`, em `.claude/skills/devzymer/` e `.agents/skills/devzymer/`;
3. o registro do servidor MCP, que é o que de fato responde (veja [mcp.md](mcp.md)).

Cada peça tem uma função. O **MCP** devolve a informação do repositório. A **skill** diz qual
ferramenta escolher e como transformar o resultado em evidência. Os **hooks** de roteamento
decidem se uma busca nativa passa (veja [routing.md](routing.md)). A skill não substitui os
hooks e não depende deles: sem hooks, ou com o roteamento em `off`, ela funciona do mesmo jeito.

O texto do projeto não se coloca acima do usuário nem do host. Pedidos explícitos e as permissões
do agente valem mais do que estas preferências, e o próprio conteúdo diz isso.

## O que o agente aprende

O ciclo é *entender → localizar → inspecionar → verificar → agir → validar → relatar*, aplicado
só até onde a tarefa exige:

- identificar o objetivo antes de escolher a ferramenta, e usar o MCP quando ele ajuda, não para
  parecer que o usou;
- recuperar antes de explorar: `context_resolve` com a intenção do usuário, `context_read` só da
  faixa necessária, `context_impact` quando as relações importam;
- buscar comportamento, não só nomes, e conferir se o candidato realmente o implementa;
- separar fonte verificada, inferência estrutural, inferência léxica e suposição (aresta
  `heuristic` não é dependência confirmada, aresta ausente não prova ausência de chamador);
- editar com o menor trecho e o `expected_sha` da leitura, sem contornar a proteção;
- cair para a busca nativa com honestidade quando o índice não dá evidência confiável, e nunca
  reparar um índice por conta própria;
- inicializar um projeto sem índice só quando a raiz é a do projeto em que trabalha, o pedido e as
  permissões autorizam, e uma única vez.

Seis fluxos têm roteiro próprio: descobrir uma implementação, procurar um bug sem pista,
corrigir um bug, medir impacto, se orientar em um repositório novo e pesquisar com fallback nativo.

## Organização

```
devzymer/
  SKILL.md                          # curto: quando usar, ciclo, protocolo, índice das referências
  references/
    tool-selection.md               # intenção → ferramenta, contrato de cada uma, exemplos
    repository-discovery.md         # fluxos 1 e 5
    bug-investigation.md            # fluxos 2 e 3
    dependency-analysis.md          # fluxo 4, limites do grafo
    safe-editing.md                 # contrato do patch, quando usar a edição nativa
    fallback-and-recovery.md        # erros, fluxo 6, projeto sem índice, índice danificado
    context-efficiency.md           # orçamento de contexto
  .devzymer-manifest.json           # propriedade e versão (não é lido pelo agente)
```

O `SKILL.md` é carregado quando a skill é escolhida; cada referência só quando a tarefa pede, e
nenhuma aponta para outra. O frontmatter tem apenas `name` e `description`, que são os campos
portáveis entre Claude Code, Codex e OpenCode.

## Versão e propriedade

A versão da skill (`SKILL_VERSION`, hoje `1.0.0`) é **independente** do esquema do SQLite, da
versão do parser, do resolvedor e do binário. Ela sobe quando o conteúdo do modelo muda.

O manifesto grava a versão, o protocolo MCP contra o qual a skill foi escrita, as ferramentas
que ela cita, a menor versão do devzymer compatível e o hash de cada arquivo **como o devzymer o
escreveu**. Os hashes ignoram a diferença entre CRLF e LF, para que `core.autocrlf` não faça um
arquivo intacto parecer alterado.

A cada `init` (ou `integrations repair`), cada arquivo cai em um destes estados:

| Estado | Significa | O que acontece |
| --- | --- | --- |
| ausente | não existe | é gravado |
| atual | igual ao modelo | nada |
| desatualizado | o devzymer o escreveu, de um modelo anterior, e você não o tocou | é atualizado |
| alterado | o devzymer o escreveu e você o editou depois | **mantido**, e o conflito é apontado |
| de terceiros | existe e o devzymer não o escreveu | **mantido**, e o conflito é apontado |

Arquivos que você acrescentou à pasta (por exemplo `references/meu-fluxo.md`) nunca são tocados.
Uma skill instalada por um devzymer mais novo não é rebaixada. Para receber a versão atual de um
arquivo que você alterou, mova-o ou remova-o e rode `devzymer init --repair`.

`devzymer uninstall` remove o bloco gerenciado e apenas os arquivos da skill que ainda são os do
devzymer.

## Verificação

```bash
devzymer doctor     # bloco presente, skill atual, frontmatter válido, estágio da inicialização
devzymer integrations list
```

O `doctor` confirma que os **arquivos** estão corretos. Se o agente de fato carrega as instruções
e a skill é uma propriedade da sessão do agente, e não é verificada por ele: abra uma sessão nova
e confira (no Claude Code, a skill `devzymer` aparece entre as skills disponíveis; no Codex,
`$devzymer` ou `/skills`; no OpenCode, a ferramenta `skill`). Se o agente escolhe a skill sozinho
depende do modelo e de quantas outras skills competem com ela; a invocação explícita é o caminho
garantido.

## Melhorar ao longo do tempo

A skill melhora por versões revisadas do modelo: fluxos novos, exemplos verificados, descrições
de ferramenta mais claras. O modelo não é treinado e a skill não é reescrita a partir de prompts
do usuário nem de comentários no código: o conteúdo do repositório e a saída das ferramentas são
dados da tarefa, não autorização para mudar a política do agente. Qualquer geração adaptativa
futura precisaria de validação e do controle do usuário.
