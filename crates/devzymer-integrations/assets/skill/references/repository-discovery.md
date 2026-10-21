# Descoberta de código

## Workflow 1: achar a implementação de um comportamento

Pedido: "Ache a implementação que transfere uma chamada ativa da IA para um atendente humano
disponível."

1. **Resolver o comportamento.** `context_resolve` com a frase do usuário. A ação é
   *transferir*; os objetos são *chamada ativa*, *IA*, *atendente humano disponível*; a
   restrição de domínio é *disponível*. Não invente `transferToHuman`.
2. **Inspecionar o melhor candidato.** `context_read` do símbolo. Descarte candidatos que só
   compartilham palavras: um componente de UI chamado `TransferButton` não transfere nada.
3. **Verificar o comportamento real.** Procure no código lido: a chamada ao provedor de voz ou
   telefonia, a escolha do atendente (como "disponível" é determinado), o que acontece com a
   chamada da IA (encerra, mantém, repassa) e o tratamento de falha.
4. **Explorar chamadores e contratos.** `context_impact` com profundidade 1 para saber quem
   dispara a transferência; leia o tipo ou a interface que define o contrato do provedor.
5. **Relatar.** Dê `arquivo:linha` da função que executa a transferência, de onde se decide a
   disponibilidade e de quem a chama. Diga o que ficou sem verificar (por exemplo, o provedor
   externo).

Sucesso é identificar a evidência da implementação, não componentes de nome parecido.

### Exemplo: recuperação bem-sucedida

```
context_resolve("transferir chamada ativa da IA para atendente humano disponível")
→ services/voice/handoff.ts:40-96 | HandoffService.transferToAgent | method | heuristic
→ ui/TransferButton.tsx:5-30      | TransferButton                 | function | heuristic
```

O primeiro chama `provider.bridge(callId, agentId)` e escolhe o agente com `pickAvailableAgent`;
o segundo só dispara um evento. Cite o primeiro, mencione o segundo como ponto de entrada da UI.

### Exemplo: recuperação ambígua

```
ambiguidade: 'handle' tem 31 definições; use o nome qualificado para desambiguar
```

Não escolha a primeira. Releia o pedido, descubra o módulo (`handoff`), consulte
`handoff.ts handle` ou `HandoffService.handle`.

## Workflow 5: repositório desconhecido

1. **Verificar índice e raiz.** `context_status` uma vez: cobertura, linguagens, falhas. Confirme
   que a raiz é o projeto em que você trabalha.
2. **Entradas e módulos.** Manifestos (`package.json`, `Cargo.toml`, `pyproject.toml`), arquivos
   `main`/`index`/`app`/`server`/`cli` e rotas: `context_resolve` com o nome do arquivo.
3. **Símbolos de alto nível.** Leia os símbolos representativos de cada módulo principal, não a
   lista de milhares de símbolos.
4. **Relações seletivas.** `context_impact` por `path` nas entradas, profundidade 1, para ver o
   que cada módulo importa.
5. **Mapa mental para a tarefa.** Escreva 5 a 10 linhas: entrada → camadas → persistência →
   integrações externas. Atualize quando a tarefa pedir outro canto do sistema.

Evite enumerar tudo "para conhecer". O mapa serve à tarefa atual.

## Critérios de parada

- Você leu a função que implementa o comportamento e sabe quem a chama: pare.
- Dois candidatos plausíveis e nenhum leu: leia os dois antes de concluir.
- Três consultas sem aprendizado novo: mude de estratégia (outro termo do domínio, outra
  pasta, ou busca nativa) em vez de repetir.
