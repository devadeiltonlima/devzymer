# Migração e desinstalação

## Detecção de CodeGraph

`devzymer init` e `devzymer migrate` procuram instalações do CodeGraph em:

- diretório `.codegraph/` na raiz do projeto
- registro de servidor MCP em `.mcp.json` do projeto
- registro de servidor MCP em `~/.claude.json`, no escopo do projeto e no escopo do usuário
- arquivo `settings.json` do Claude Code, no projeto e no usuário
- executável `codegraph` no `PATH`

A detecção é somente leitura. Ela informa o escopo, o arquivo e o nome do servidor MCP
encontrado.

```bash
devzymer migrate --dry-run
```

```
plano de migração
  codegraph detectado: true
  índice do codegraph: /caminho/projeto/.codegraph
  ação: registrar o servidor MCP do devzymer no projeto
  ação: instalar as regras de uso do devzymer
  ação: instalar os hooks de compressão e de guarda de arquivos de teste
  ação: desativar a entrada MCP do CodeGraph: codegraph
  aviso: a desativação do CodeGraph é local: o pacote continua instalado e pode ser reativado

nenhuma alteração aplicada. Use `devzymer migrate --confirm` para executar.
```

## Ordem de execução

A migração segue a ordem exigida, e nenhuma ferramenta é removida antes de a substituta passar
na verificação:

1. detectar o CodeGraph e identificar o escopo da instalação;
2. criar backup dos arquivos afetados;
3. instalar o devzymer (servidor MCP, regras e hooks);
4. construir o índice inicial;
5. verificar a conectividade MCP e a recuperação funcional;
6. desativar a entrada MCP do CodeGraph no escopo escolhido;
7. verificar que a entrada foi removida;
8. em caso de falha, restaurar as configurações.

## Escopo e consentimento

- Sem `--confirm` e com `--dry-run`, nada é alterado.
- A desativação do CodeGraph é **local**: a entrada MCP é removida do arquivo de configuração
  escopado. O pacote continua instalado e a ferramenta pode ser reativada.
- A desinstalação global do pacote CodeGraph exige permissão separada e não é feita pelo
  devzymer.
- Outras ferramentas de indexação não são removidas, mesmo com função semelhante.

## Backup e restauração

Todo arquivo modificado é copiado para `.devzymer/backups/<timestamp>/` antes da escrita. Os
backups são preservados até que sejam removidos manualmente.

```bash
ls .devzymer/backups/
```

## Desinstalação

```bash
devzymer uninstall
devzymer uninstall --purge
devzymer uninstall --scope user
```

O que é removido:

- a entrada do servidor MCP `devzymer` no escopo escolhido
- o bloco de regras entre os marcadores `<!-- devzymer:start -->` e `<!-- devzymer:end -->`
- os hooks que apontam para o executável do devzymer

O que é preservado:

- outros servidores MCP, incluindo o CodeGraph
- hooks e regras que não pertencem ao devzymer
- o restante do conteúdo do `CLAUDE.md` e do `settings.json`
- o índice local, a menos que `--purge` seja usado

A desinstalação é idempotente: executá-la duas vezes não causa erro nem efeito adicional.

## Instalação em escopo de usuário

```bash
devzymer config set mcp.install_scope user
devzymer init
```

O escopo `user` grava a entrada em `~/.claude.json` e a torna disponível em todos os projetos
da máquina. O padrão é `project`, que grava em `.mcp.json` e compartilha a configuração com a
equipe pelo controle de versão.

O escopo `local` grava em `~/.claude.json` sob o caminho do projeto, o que torna o servidor
disponível apenas naquele projeto e fora do controle de versão.
