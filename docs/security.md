# Segurança e privacidade

## Modelo de ameaça

O devzymer processa conteúdo não confiável: código do repositório, comentários, documentação,
saídas de ferramentas e respostas de modelos externos. Nada disso é tratado como instrução.
Comentários que dizem "ignore as regras anteriores" são texto indexado, nunca uma ordem.

O controle confiável vem da configuração e dos argumentos de linha de comando, não do conteúdo
indexado.

## Fronteiras

### Workspace

Toda leitura e escrita é confinada à raiz do workspace.

- Caminhos são normalizados antes da comparação; `..` é resolvido lexicalmente.
- O destino final é verificado contra a raiz canônica, o que impede escape por link simbólico.
- Uma tentativa de escape produz `PATH_ESCAPE` e não executa nada.

O mesmo vale para o patch: um caminho fora do workspace é rejeitado antes de qualquer leitura.

### Rede

O devzymer não faz requisições de rede durante a indexação, a busca ou o patch. A única
dependência de rede é o provedor de decisão externo, e ele só é acionado quando habilitado,
configurado e necessário.

- O índice nunca é enviado.
- O conteúdo do repositório não é enviado por padrão: `privacy.allow_remote_source` é `false`.
- O estado enviado ao provedor é um resumo delimitado por `decisions.max_state_chars`, e um
  estado que parece conter código-fonte é recusado.
- Timeout e retentativa são finitos; o circuit breaker impede laço de falhas.

### Credenciais

- A chave do provedor é lida de variável de ambiente, nunca de arquivo.
- A chave não aparece em log, cache, relatório de benchmark ou dump de configuração.
- `logging.redact_secrets` remove de logs valores com aparência de credencial (chaves de AWS,
  GitHub, Slack, Google, JWTs e cabeçalhos PEM).

### Banco de dados

- Todas as consultas são parametrizadas. Não há concatenação de entrada em SQL.
- O conteúdo do repositório é tratado como dado, nunca como comando.
- A configuração é gravada de forma atômica, com arquivo temporário e renomeação.

### Sistema de arquivos

- A instalação altera apenas arquivos do projeto, sempre com backup antes da escrita.
- A desinstalação remove apenas o que o devzymer criou.
- `.gitignore`, `.ignore` e `.devzymerignore` são respeitados na varredura.
- A varredura não segue links simbólicos para fora do workspace.

### Entrada de modelos externos

- A resposta do provedor é validada contra o esquema esperado; uma resposta inesperada vira
  `PROVIDER_UNAVAILABLE`, nunca um valor adivinhado.
- Jev não é tratado como gerador de texto nem como substituto de um modelo de raciocínio.

## Permissões de arquivos de teste

O hook `guard-test-files` bloqueia a criação de arquivos `.test`, `.spec`, `__tests__` e afins
sem aprovação explícita. Ele cobre `Write`, `Edit`, `MultiEdit`, `NotebookEdit` e comandos de
terminal que criam arquivos.

**Limitação declarada:** o hook é uma barreira de melhor esforço, não um limite de segurança
absoluto. Um script executado fora das ferramentas do Claude Code pode criar arquivos sem passar
pelo hook. O bloqueio é responsabilidade do ambiente de execução, não do devzymer.

Para liberar:

```bash
devzymer config set tests.allow_files true
# ou, por projeto:
touch .devzymer/tests.approved
```

## Hooks de roteamento

Os hooks e o plugin de roteamento rodam a cada chamada de ferramenta do agente, então a
superfície é pequena e fechada:

- Não alteram arquivos-fonte e **abrem o índice somente leitura** (`SQLITE_OPEN_READ_ONLY`): não
  criam, migram, adotam identidade nem reparam. Nunca disparam indexação.
- Só agem em um workspace que já tem `.devzymer/`, achado subindo a partir do diretório da
  chamada e parando antes da home. Fora dele, não fazem nada.
- Não leem o corpo de arquivos. Classificam o comando ou o padrão, consultam o índice por nome e
  descartam o texto.
- Qualquer erro interno resulta em permitir: um hook com defeito não bloqueia o trabalho.
- A telemetria (`.devzymer/routing/events.jsonl`) guarda só contagens e categorias; nunca prompt,
  comando, padrão de busca, código ou credencial. A sessão é um hash de 12 hex.
- O estado de sessão tem orçamento: no máximo uma negação por busca e 6 por sessão.

## Compressão de saída de ferramentas

O hook `post-tool-use` comprime a saída grande do Bash. A compressão é determinística e
preserva: as primeiras linhas, as últimas linhas, linhas com marcadores de erro, a presença de
stderr e todos os demais campos do objeto de resposta. Quando há stderr, o texto comprimido
declara explicitamente que a operação pode ter falhado. Uma falha nunca é apresentada como
sucesso.

## O que o devzymer não faz

- Não modifica o Claude Code internamente; usa apenas mecanismos de extensão suportados
  (registro de servidor MCP, regras, hooks em arquivo de settings).
- Não desinstala outras ferramentas de indexação sem aprovação explícita e por escopo.
- Não remove servidores MCP, hooks ou regras que não pertencem ao devzymer.
- Não envia código do repositório sem autorização explícita.
- Não executa nada lido do repositório: o conteúdo indexado é dado, jamais instrução.

## Escrita concorrente

O `~/.claude.json` é reescrito pelo Claude Code durante a sessão. O devzymer relê o arquivo
antes de gravar e reaplica a mudança se o conteúdo mudou no meio do caminho, até cinco
tentativas. Toda alteração passa por um backup em `.devzymer/backups/`.
