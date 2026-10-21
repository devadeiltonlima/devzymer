# devzymer no npm

Wrapper de distribuição multiplataforma. O pacote não contém o motor: ele instala o binário
nativo correspondente à plataforma e cria os lançadores.

## Estado da publicação

**O pacote não foi publicado no npm.** O `package.json` e o instalador existem e são funcionais
localmente, mas nenhum registro foi criado, nenhuma assinatura foi feita e nenhum artefato de
release foi publicado. Qualquer afirmação de disponibilidade no registry seria falsa.

## O que o instalador faz

1. Procura um binário local em `../target/release/devzymer` ou `../target/debug/devzymer`.
2. Na ausência dele, baixa o arquivo de release de `DEVZYMER_RELEASE_BASE`
   (`https://github.com/devadeiltonlima/devzymer/releases/latest/download` por padrão).
3. Copia o binário para `bin/devzymer-bin` e cria os lançadores POSIX e Windows.

Quando nenhuma das duas fontes está disponível, o instalador explica como compilar a partir do
código e sai com código 1, sem deixar o pacote em estado quebrado.

## Uso local

```bash
cd npm
python install.py
./bin/devzymer --version
```

Ou, a partir da raiz do repositório:

```bash
cargo build --release
cd npm && python install.py
```

## Instalação por consumidor

```bash
npx devzymer init
```

O comando exige que o binário nativo esteja disponível para a plataforma, o que o passo
`postinstall` resolve.

## Publicação

Publicar exige, nesta ordem:

1. criar a tag de release e os artefatos com os nomes usados por `archive_name()`;
2. conferir que `cargo build --release` e os testes passam no commit da tag;
3. `npm publish` a partir de `npm/`, com as credenciais do registry;
4. registrar a versão publicada e a assinatura do pacote.

Nenhum desses passos foi executado.
