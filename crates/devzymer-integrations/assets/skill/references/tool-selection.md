# Seleção de ferramentas

## Intenção → primeira chamada

| Intenção do usuário | Primeira chamada | Depois | Parar quando |
| --- | --- | --- | --- |
| Onde fica `X` / qual arquivo | `context_resolve` | `context_read` da faixa | o caminho e a linha foram confirmados na leitura |
| Como `X` funciona | `context_resolve` | `context_read` do símbolo, `context_impact` (profundidade 1) | você consegue descrever o fluxo com linhas citadas |
| Quem usa `X` / o que quebra | `context_resolve` (fixa o símbolo) | `context_impact` | os chamadores diretos foram lidos |
| Ache um bug | `context_status` uma vez, `context_resolve` por caminho de risco | leitura + hipótese | hipótese refutada ou confirmada |
| Altere `X` | `context_resolve` + `context_read` | `context_impact`, `context_patch` | a validação passou |
| Criei, renomeei ou apaguei arquivos | `context_sync` | nova consulta | `context_status` sem pendências |
| Rode testes, build, git, veja logs | ferramenta nativa | — | — |

`context_decide` só serve quando as regras acima não resolvem uma decisão de roteamento; não
é necessário no fluxo comum.

## Contrato de cada ferramenta

- **`context_resolve`** `{query, path?, language?, limit?}`: só `query` é obrigatório. Aceita
  nome de símbolo, nome qualificado (`Classe.metodo`), nome de arquivo, caminho ou descrição em
  linguagem natural. Devolve caminho, símbolo, faixa de linhas e confiança (`confirmed` ou
  `heuristic`). `path` restringe a um arquivo ou diretório; `language` filtra por linguagem.
- **`context_read`** `{path, symbol? | start_line+end_line?, max_bytes?}`: lê a faixa e devolve
  o `sha` do arquivo, necessário para o patch. Sem faixa nem símbolo, lê o arquivo inteiro dentro
  dos limites: evite, a menos que vá editá-lo.
- **`context_impact`** `{symbol | path, depth?}`: com `symbol`, quem o chama direta e
  indiretamente; com `path`, quem importa o arquivo e o que ele importa.
- **`context_patch`**: edição mínima com `expected_sha` obtido em `context_read`; o intervalo
  precisa estar dentro do trecho lido.
- **`context_sync`** `{full?, budget_ms?}`: reconcilia o índice com o disco. `full: true` reconstrói
  tudo; use só quando o sync normal não resolver.
- **`context_status`**: frescor, cobertura, integridade, arquivos ignorados e falhas.

No OpenCode as ferramentas aparecem com o prefixo `devzymer_` (`devzymer_context_resolve`); no
Claude Code, `mcp__devzymer__context_resolve`.

## Formular a consulta

1. Comece pelo pedido do usuário, na linguagem dele, sem traduzir para um nome inventado.
2. Se o resultado vier amplo, acrescente uma restrição que você *aprendeu* (diretório, linguagem,
   nome de arquivo do melhor candidato), em vez de reescrever a frase.
3. Se vier `ambiguidade:`, use o nome qualificado ou `arquivo.ext nome`.
4. Se um identificador exato é conhecido (vem do usuário, de um stack trace ou do código lido),
   consulte-o literalmente.

## Roteamento correto e incorreto

| Pedido | Correto | Incorreto |
| --- | --- | --- |
| "onde se calcula o frete?" | `context_resolve("onde se calcula o frete")`, ler o melhor candidato | `grep -r frete` e ler cinco arquivos inteiros |
| "quem chama `charge()`?" | `context_resolve("charge")` e `context_impact` | afirmar quem chama a partir só do nome |
| "o teste falha com timeout" | rodar o teste (nativo), depois `context_resolve` no símbolo do stack trace | procurar o erro no índice sem ter a saída |
| "mude o limite para 50" | ler a faixa, `context_patch` com o sha | reescrever o arquivo inteiro |
| o usuário diz "use grep" | usar grep | insistir no devzymer |

## Quando a ferramenta nativa é a escolha certa

Logs e saída de comandos; arquivos gerados, minificados ou ignorados; conteúdo mais novo do que
o índice; um único arquivo que você já sabe qual é e vai editar por inteiro; texto que não é
código; e qualquer caso em que o usuário pediu busca nativa. Dizer isso ao usuário basta; não
é preciso justificar longamente.
