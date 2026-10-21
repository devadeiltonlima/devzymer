# Diagnóstico

Comece por:

```bash
devzymer doctor
```

A saída lista cada verificação com `ok`, `aviso` ou `falha`. O comando sai com código 1 quando
há falha.

## Problemas frequentes

### `índice não inicializado`

```
Error: índice não encontrado em .devzymer/index.sqlite3
```

Execute `devzymer init` na raiz do projeto.

### `índice não encontrado` após trocar de diretório

O devzymer sobe a árvore procurando `.git` ou `.devzymer`. Se o projeto não for um repositório
git e não tiver a pasta `.devzymer`, ele para no diretório atual. Use `--root`:

```bash
devzymer status --root /caminho/do/projeto
```

### A busca não encontra nada

```bash
devzymer status
```

Verifique o campo `atualidade`. Se houver arquivos alterados, sincronize:

```bash
devzymer sync
```

Se o símbolo existir mas não for encontrado, confira o resultado do parser:

```bash
devzymer resolve "nomeParcial"
```

Símbolos de uma linha só, como o resultado de uma atribuição dentro de um laço, e símbolos de
padrão de desestruturação não são indexados, por não descreverem um alvo de recuperação útil.

### `SYMBOL_NOT_FOUND` em uma consulta em linguagem natural

A consulta é quebrada em tokens, e tokens muito curtos ou palavras de parada são descartados.
"valida sessão" vira os tokens `valida` e `sessao`. Se a implementação se chamar
`validateSession`, use o nome do símbolo ou uma palavra mais específica:

```bash
devzymer resolve "validateSession"
```

### O Claude Code não enxerga o servidor MCP

```bash
claude mcp list
claude mcp get devzymer
```

Causas frequentes:

- **Servidor de escopo de projeto aguardando aprovação.** Execute `claude` interativamente no
  projeto e aprove. Em sessão `-p` a aprovação não é exibida.
- **Pasta não confiável.** O Claude Code ignora `.mcp.json` em pasta ainda não confiável.
- **Executável movido.** O `.mcp.json` guarda o caminho absoluto do binário. Se o binário mudou
  de lugar, rode `devzymer init` de novo para reescrever a entrada.
- **Servidor falhando ao iniciar.** Rode o comando manualmente para ver o erro:

  ```bash
  devzymer serve --root .
  ```

### A instalação não substitui o que já existe

Por desenho. O devzymer só acrescenta:

- o `.mcp.json` existente é lido, a entrada `devzymer` é inserida ou atualizada e as demais
  entradas ficam intactas;
- o `CLAUDE.md` existente recebe apenas o bloco delimitado por marcadores;
- o `settings.json` existente recebe apenas os grupos de hook do devzymer.

Toda alteração tem backup em `.devzymer/backups/`.

### `codegraph detectado` no doctor

O CodeGraph está registrado no escopo do usuário (`~/.claude.json`) ou no projeto. Isso é
informação, não erro. Para migrar:

```bash
devzymer migrate --dry-run
devzymer migrate --confirm
```

### Falhas de compilação

```bash
cargo build 2>&1 | Select-Object -First 40
```

No Windows, o erro típico é `linker link.exe not found`, que indica ausência das Build Tools com
o componente de C++. Reinstale com o workload "Desktop development with C++".

### Banco corrompido

```bash
devzymer doctor
```

A verificação `banco` mostra o resultado de `PRAGMA integrity_check`. Se não for `ok`, o índice
pode ser reconstruído:

```bash
devzymer uninstall --purge
devzymer init --full
```

### Decisões externas indisponíveis

```
[aviso] decisões: provedor externo habilitado, mas TYPESAFE_API_KEY não está definida
```

O sistema continua funcional: as decisões passam a ser resolvidas por regras locais. Defina a
variável ou desligue o provedor:

```bash
devzymer config set decisions.enabled false
```

## Logs

```bash
devzymer --verbose status
```

O nível de log vem de `logging.level` e de `RUST_LOG`. Saída para arquivo:

```toml
[logging]
level = "debug"
file = ".devzymer/devzymer.log"
```

## Verificação automatizada

```bash
python scripts/mcp_smoke.py
python scripts/patch_check.py
```

O primeiro sobe o servidor MCP real e exercita o protocolo. O segundo verifica o patch atômico, a
recusa de revisão obsoleta e o limite de workspace. Ambos falam com o binário em execução.

## Agentes e integrações

| Sintoma | Causa | Ação |
| --- | --- | --- |
| O agente ignora o devzymer | A chamada da ferramenta é rejeitada por falta de permissão | `devzymer doctor` mostra `permissao das ferramentas MCP`; `devzymer integrations repair` regrava a regra `mcp__devzymer__*` |
| `you haven't granted it yet` | A regra de permissão não existe no escopo do agente | `devzymer init --agents <agente>` de novo |
| A interface não abre | O terminal não é interativo, ou o comando roda em script | `devzymer init --agents claude,codex,opencode` |
| A interface não abre com terminal | O terminal não suporta modo cru | Use `--agents`; a escolha é a mesma |
| O Codex não encontra o servidor | O executável registrado não existe mais | `devzymer integrations repair` |
| O Codex não lê as regras | O `AGENTS.md` do projeto não foi criado | `devzymer integrations repair` |
| O agente mostra `Connection closed` para o devzymer | O comando gravado no registro (ou no hook) não existe mais, porque o binário foi movido, renomeado ou atualizado em outro lugar; o Codex só lê `.codex/config.toml` de projetos confiáveis | `devzymer doctor` aponta o comando ausente e a versão do hook; `devzymer integrations repair` regrava o caminho atual. Reinicie o agente depois |
| O OpenCode não carrega o servidor | `opencode.json` ficou com JSON inválido | `devzymer doctor` e o arquivo; o backup está em `.devzymer/backups/` |
| As regras do Codex e do OpenCode colidem | Os dois leem `AGENTS.md` no projeto | O bloco é o mesmo e é escrito uma única vez, com marcadores |
| O agente consulta outro repositório | O servidor foi iniciado fora do projeto | `devzymer doctor` mostra a raiz; use `--root` |
| `init` recusa: "é a pasta do usuário" | O diretório atual é a home (ou um ancestral); o `init` não escolhe isso por descoberta automática | Entre na pasta do projeto, ou `devzymer init --root <pasta>` |
| `init` recusa: "o índice existente não pôde ser usado" | Banco ilegível, FTS5 inconsistente ou índice de outro workspace. Nada foi alterado | `devzymer init --repair` guarda o arquivo ao lado (`index.sqlite3.corrupt-<carimbo>`) e reconstrói; nada é apagado |
| `init` avisa "pode conflitar com o devzymer" | Uma instrução sua em `CLAUDE.md`/`AGENTS.md` (por exemplo "nunca use o devzymer") vai contra o bloco gerenciado | O texto é mantido e só apontado. Decida qual vale e ajuste o seu |
| `init` avisa "marcadores malformados" | Falta o `<!-- devzymer:start -->` ou o `<!-- devzymer:end -->` no arquivo | O arquivo não foi tocado. Corrija ou remova o marcador e rode `devzymer init` |
| Skill "foi alterado depois da instalação" | Você editou um arquivo da skill; ele é seu e não é substituído | Para receber a versão atual, mova ou remova o arquivo e rode `devzymer init --repair` |
| `doctor` diz que a inicialização está parcial | O `init` foi interrompido (estágio em `.devzymer/init-state.json`) | `devzymer init --repair` conclui as etapas que faltam |
| O agente não usa a skill | A skill existe, mas o carregamento é da sessão do agente | Abra uma sessão nova; confira que `devzymer` aparece nas skills do agente; invoque-a pelo nome |
| Codex: "MCP tool call requires approval" em `codex exec` | A execução não interativa tem política de aprovação `never` | Em sessão interativa o Codex pergunta; em automação, configure a aprovação do servidor no seu `config.toml` |
