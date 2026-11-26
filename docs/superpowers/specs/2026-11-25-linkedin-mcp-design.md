# LinkedIn MCP em Rust — Design

## Intenção

Criar um servidor MCP em Rust para permitir que agentes de IA compatíveis com MCP preparem conteúdo para o LinkedIn e publiquem posts pessoais pela API oficial, além de ajudar o usuário a revisar e atualizar o próprio perfil sem automatizar a interface do LinkedIn.

## Objetivos

- Integrar com clientes de agentes que aceitem MCP (incluindo Codex, Claude e outros compatíveis).
- Usar OAuth 2.0 e apenas produtos, escopos e endpoints oficiais efetivamente habilitados para o app.
- Oferecer ferramentas para preparar um post, revisar o conteúdo e publicar somente após confirmação explícita do usuário.
- Oferecer apoio à edição de perfil: gerar/revisar texto de campos com base em dados fornecidos pelo usuário e abrir a página oficial de edição; não editar o perfil via API ou navegador.
- Manter tokens fora de logs e protegidos em armazenamento local apropriado.

## Fora do escopo

- Scraping, automação de navegador, extensão/injeção de interface, simulação de cliques ou chamadas privadas.
- Convites, mensagens, curtidas, comentários ou ações automáticas de engajamento.
- Edição de campos do perfil por API até que o LinkedIn confirme que o app e o caso de uso têm acesso aprovado.
- Publicação sem uma confirmação afirmativa vinculada ao conteúdo exato que será enviado.

## Restrições e premissas

- A publicação pessoal pode usar a API oficial de posts com OAuth e `w_member_social`, se o produto Share on LinkedIn estiver habilitado no app. O acesso e os termos atuais precisam ser confirmados antes de liberar a função.
- A documentação de People APIs menciona edição de perfil, mas não demonstra disponibilidade geral. O MVP não dependerá desse acesso.
- Clientes diferentes podem suportar transportes MCP distintos. Compatibilidade significa suporte ao transporte configurado pelo usuário; não será possível garantir integração universal com apps sem suporte MCP.
- Falhas da API, revogação de consentimento e limites de taxa devem ser retornados claramente, sem tentativas de contornar bloqueios.

## Abordagens consideradas

1. **MCP local por stdio (escolhida para o MVP):** o usuário instala/compila o binário Rust e configura cada cliente compatível para iniciá-lo. Simplifica autenticação local e evita hospedar um serviço com credenciais. A configuração precisa ser repetida nos clientes e cada um deve aceitar MCP por stdio.
2. **Servidor remoto por HTTP:** um serviço hospedado pode ser compartilhado por vários agentes e dispositivos. Exige hospedagem, autenticação do usuário no serviço, proteção de tokens em servidor e suporte do cliente ao transporte remoto; aumenta a superfície operacional.
3. **Automação da interface do LinkedIn:** descartada porque scraping e automação não autorizada são proibidos pelas regras do LinkedIn e elevam o risco de restrição da conta.

O MVP usa stdio, com separação interna entre protocolo MCP e integração LinkedIn para permitir avaliar transporte HTTP como etapa futura sem compromisso de implementação.

## Experiência e capacidades propostas

- `prepare_post`: recebe texto (e, numa primeira versão, opcionalmente URL), valida limites e devolve uma prévia; não publica.
- `publish_post`: recebe o texto exato e uma confirmação explícita no mesmo pedido, envia uma publicação textual via API oficial e devolve o identificador/link quando disponível. Sem confirmação, recusa a ação.
- `draft_profile_update`: recebe o campo e o texto atual/proposto fornecido pelo usuário, sugere uma versão editada e devolve um link para a página de edição do LinkedIn. Não lê ou altera a conta.
- `linkedin_connection_status`: informa se OAuth está configurado e quais permissões foram concedidas, sem expor tokens.

A publicação com mídia, leitura do perfil e publicação em páginas de organizações ficam para depois do MVP e exigem análise de escopos, aprovação e requisitos próprios.

## Segurança e privacidade

- OAuth 2.0 com fluxo adequado a aplicativo instalado, callback local com validação de `state` e proteção contra CSRF.
- Armazenar tokens no cofre do sistema operacional quando disponível; oferecer alternativa explícita e segura quando não houver cofre.
- Nunca registrar tokens, códigos OAuth ou conteúdo privado por padrão.
- Validar que o autor é o membro autenticado; não aceitar URNs arbitrárias do agente para publicar como outra pessoa.
- Confirmar exatamente o texto final antes do envio e não repetir automaticamente uma publicação após resultado ambíguo.
- Limpar/invalidar credenciais localmente e documentar revogação no LinkedIn.

## Decisões para o implementador

- Escolher crates Rust mantidos para MCP, OAuth, HTTP e armazenamento seguro após verificar compatibilidade e licença.
- Definir a versão suportada do protocolo/API do LinkedIn e uma estratégia de atualização.
- Determinar como iniciar o callback OAuth de forma consistente nos sistemas operacionais suportados.
- Propor instalação e configuração documentadas para os clientes MCP inicialmente selecionados.

## Questões ainda em aberto

Não há questões que impeçam o plano. Decisões assumidas para o primeiro marco:

- Priorizar Linux, conforme o ambiente atual, e documentar configuração para Codex, Claude e outros clientes que aceitem MCP por stdio; suporte depende do transporte MCP que cada cliente oferece.
- Verificar o produto Share on LinkedIn e `w_member_social` no início. Se não estiverem habilitados, deixar a publicação bloqueada e incluir a solicitação/validação de acesso como pré-requisito, sem usar automação de interface.
- A confirmação será uma etapa explícita ligada ao texto exato publicado.
- Começar com posts de texto e links; mídia fica para etapa futura.

## Fontes consultadas

- [LinkedIn User Agreement](https://www.linkedin.com/legal/user-agreement)
- [Prohibited software and extensions](https://www.linkedin.com/help/linkedin/answer/a1341387/prohibited-software-and-extensions)
- [Getting Access to LinkedIn APIs](https://learn.microsoft.com/en-us/linkedin/shared/authentication/getting-access)
- [Share on LinkedIn](https://learn.microsoft.com/linkedin/consumer/integrations/self-serve/share-on-linkedin)
- [Posts API](https://learn.microsoft.com/en-us/linkedin/marketing/community-management/shares/posts-api)
- [People APIs](https://learn.microsoft.com/en-us/linkedin/shared/integrations/people/overview)
