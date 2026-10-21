# Limitações conhecidas

Esta é a lista honesta do que o devzymer **não** faz nesta fase. Cada item aqui é uma
restrição real, verificada, e não uma projeção.

## Análise estática

**O grafo de chamadas é heurístico em linguagens dinâmicas.** As arestas `calls` são estabelecidas
por correspondência de nome entre o corpo de um símbolo e os símbolos indexados. Um método
chamado por um nome dinâmico, por reflexão ou por variável não aparece. A confiança dessas
arestas é `heuristic`, nunca `confirmed`.

**A herança é resolvida por nome.** `extends`, `implements` e `impl ... for` casam o nome do tipo
base com os símbolos indexados. A confiança também é `heuristic`.

**A resolução de importações é incompleta por natureza.** O grafo resolve importações relativas
em TypeScript e JavaScript, pacotes em Python e caminhos `crate::` em Rust. Importações de
pacotes externos, aliases de barrel e indexações dinâmicas ficam sem resolução, marcadas como
não resolvidas.

**Não há análise de LSP.** A resolução semântica vem da estrutura sintática, não de serviços de
linguagem. Identidade semântica perfeita em código genérico ou com tipagem implícita não é
alcançada por essa via.

## Cobertura de linguagem

**Seis linguagens têm parser**: TypeScript, TSX, JavaScript, JSX, Python e Rust.

**Dez linguagens têm fallback por expressão regular**, com posição exata mas estrutura
aproximada. A extração de fallback não distingue, por exemplo, um método de um construtor em
Java, e não resolve aninhamento de classe.

**Outras linguagens recebem apenas metadados.** O arquivo é indexado e visível, mas não produz
símbolos. O devzymer não afirma cobertura que não tem.

## Decisões

**O provedor externo é opcional e desligado por padrão.** Sem credencial, o roteamento usa
regras locais, que são mais grosseiras: classificam por palavras-chave e contagem de evidência,
não por semântica.

**As decisões externas não são calibrate.** O provedor devolve confiança, e o valor é repassado
como veio, sem ajuste próprio.

**Não há laço de decisão.** Uma decisão nunca dispara nova consulta. Isso evita custo
descontrolado, mas significa que decisões realmente interdependentes precisam ser compostas
fora do devzymer.

## Patch

**O patch substitui uma faixa de linhas.** Não há aplicação de diff com contexto. Um patch
preciso é a operação mais comum; um patch que reordena blocos exige reescrever a faixa
inteira.

**Não há reversão automática.** O backup existe, e a operação é atômica, mas o devzymer não
mantém um histórico de alterações para desfazer.

**A verificação de revisão é por arquivo.** `expected_sha` protege contra o arquivo ter mudado
inteiramente entre a leitura e a escrita. Não há granularidade por linha.

## Hooks

**O hook de guarda de arquivos de teste é uma barreira de melhor esforço.** Ele cobre as
ferramentas do Claude Code e comandos de terminal, mas um script executado fora dessas
ferramentas cria arquivos sem passar pelo hook.

**A compressão de saída cobre a ferramenta Bash.** A substituição de saída usa
`updatedToolOutput`, que exige que o valor substituto tenha o mesmo formato do resultado
original. Para outras ferramentas, o hook não substitui nada, para não produzir um formato
inválido.

**A instalação dos hooks é no arquivo de settings do projeto.** Uma configuração de hooks
definida em nível superior ou gerenciada pela organização continua valendo.

## Tokens

**As contagens de token são estimativas.** O estimador local usa o tokenizador o200k. O Claude
usa um tokenizador próprio, então os números absolutos não são os do Claude.

**Não há medição de ponta a ponta com Claude Code publicada.** Ver [benchmarks.md](benchmarks.md)
para o que seria necessário. Qualquer percentual de economia de token em tarefa completa de
programação, hoje, seria uma estimativa sem base.

## Escala

**A varredura é rápida, o parsing é o gargalo.** Repositórios muito grandes têm o custo
dominado pelo parsing sintático, que é proporcional ao número de arquivos, não ao tamanho do
repositório.

**O grafo é reconstruído por completo no caminho explícito.** A sincronização automática recalcula
apenas as relações dos arquivos que mudaram. O `devzymer sync` e o `context_sync` reconstruem o
grafo inteiro, o que é aceitável para a escala testada e pode ser caro em repositórios muito
grandes.

**A sincronização automática depende do servidor MCP vivo.** O observador de filesystem roda no
processo do servidor. Mudanças feitas com o servidor parado são recuperadas pela reconciliação na
abertura da próxima sessão, o que significa que o primeiro `context_resolve` após a abertura pode
acertar o índice. A verificação de frescor por arquivo (`mtime` e tamanho) fecha esse intervalo
para os resultados devolvidos.

**O resolvedor não é exato quando o nome é ambíguo.** `useVoiceCall` casa com `useVoiceCall.ts` e
também pode casar com símbolos que começam com o mesmo prefixo. O resultado carrega a origem da
correspondência (`file_match`, `exact_symbol`, `prefix`) para que o agente possa distinguir.

**As arestas de chamada em linguagens dinâmicas são heurísticas.** Um método chamado por nome
dinâmico, reflexão ou variável não aparece. A confiança dessas arestas é `heuristic`.

**O índice cresce com o repositório.** O conteúdo textual dos símbolos é armazenado para a
busca lexical. Em projetos muito grandes, o tamanho do banco precisa ser acompanhado.

## Escopo

**A invalidação do cache de recuperação é global.** A chave do cache inclui a revisão do índice, e
qualquer alteração em qualquer arquivo incrementa a revisão. O cache é descartado por completo,
mesmo para consultas que não têm relação com o arquivo alterado. A invalidação é conservadora e
correta, apenas mais grossa do que o necessário.

**As consultas em linguagem natural dependem de documentação.** O cenário usa o comentário de
documentação escrito no arquivo de origem. Símbolos sem comentário, ou cujo comentário cita o
próprio identificador, são descartados da geração. Um repositório sem documentação tem menos
tarefas nesse cenário, e o relatório mostra a contagem.

**A validação da verdade de base exige o identificador no arquivo.** Um símbolo alcançado apenas
por reexportação pode não ter o identificador no arquivo esperado. A tarefa é marcada como não
verificada e excluída das métricas, em vez de ser contada como acerto ou falha.

**O Claude Code não controla o raciocínio interno do Claude.** As regras instaladas orientam o
comportamento observável, mas não interceptam o que o modelo considera entre as mensagens.

**O devzymer não controla o raciocínio interno do Claude.** As regras instaladas orientam o
comportamento observável, mas não interceptam o que o modelo considera entre as mensagens.

**A instalação não é totalmente reversível.** As regras são reinstaláveis, mas um agente pode
ter criado arquivos adicionais durante a sessão, que a desinstalação não remove.

**O nível de esforço do Claude Code não é alterado.** O devzymer não modifica preferências
globais do usuário.

## embeddings

**Não há busca por embeddings.** A recuperação é estrutural, lexical e por grafo. A busca
semântica por conceito é um cenário sem cobertura, e a arquitetura foi mantida simples de
proposito: adicionar embeddings exige evidência de benchmark que demonstre vantagem mensurável
para a recuperação natural-language, e essa evidência ainda não foi produzida.
