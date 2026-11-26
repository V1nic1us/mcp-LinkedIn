# LinkedIn MCP em Rust — Design

## Intenção

Criar um servidor MCP em Rust para permitir que agentes de IA compatíveis com MCP preparem conteúdo para o LinkedIn e publiquem posts pessoais pela API oficial, além de ajudar o usuário a revisar e atualizar o próprio perfil sem automatizar a interface do LinkedIn.

## Objetivos

- Integrar com clientes de agentes que aceitem MCP (incluindo Codex, Claude e outros compatíveis).
- Usar OAuth 2.0 e apenas produtos, escopos e endpoints oficiais efetivamente habilitados para o app. Publicação pessoal requer `w_member_social`; obter o URN do autor requer `openid` via OpenID Connect.
- Oferecer ferramentas para preparar um post, revisar o conteúdo e publicar somente após confirmação explícita do usuário.
- Oferecer apoio à edição de perfil: gerar/revisar texto de campos com base em dados fornecidos pelo usuário e abrir a página oficial de edição; não editar o perfil via API ou navegador.
- Manter tokens fora de logs e protegidos em armazenamento local apropriado.

## Fora do escopo

- Scraping, automação de navegador, extensão/injeção de interface, simulação de cliques ou chamadas privadas.
- Convites, mensagens, curtidas, comentários ou ações automáticas de engajamento.
- Edição de campos do perfil por API até que o LinkedIn confirme que o app e o caso de uso têm acesso aprovado.
- Publicação sem uma confirmação afirmativa vinculada ao conteúdo exato que será enviado.

## Restrições e premissas

- A publicação pessoal usa o produto Share on LinkedIn (`w_member_social`) e o endpoint de consumo `POST https://api.linkedin.com/v2/ugcPosts` com `X-Restli-Protocol-Version: 2.0.0`. A requisição usa `ShareContent` com texto, categoria `NONE` e visibilidade explícita `PUBLIC`.
- O app também precisa de Sign In with LinkedIn using OpenID Connect (`openid`). O cliente usa o token OAuth em `GET https://api.linkedin.com/v2/userinfo`, lê somente `sub` e forma localmente `urn:li:person:{sub}`. Os escopos solicitados são exatamente `openid` e `w_member_social`.
- A documentação de People APIs menciona edição de perfil, mas não demonstra disponibilidade geral. O MVP não dependerá desse acesso.
- Clientes diferentes podem suportar transportes MCP distintos. Compatibilidade significa suporte ao transporte configurado pelo usuário; não será possível garantir integração universal com apps sem suporte MCP.
- Falhas da API, revogação de consentimento e limites de taxa devem ser retornados claramente, sem tentativas de contornar bloqueios.

## Abordagens consideradas

1. **MCP local por stdio (escolhida para o MVP):** o usuário instala/compila o binário Rust e configura cada cliente compatível para iniciá-lo. Simplifica autenticação local e evita hospedar um serviço com credenciais. A configuração precisa ser repetida nos clientes e cada um deve aceitar MCP por stdio.
2. **Servidor remoto por HTTP:** um serviço hospedado pode ser compartilhado por vários agentes e dispositivos. Exige hospedagem, autenticação do usuário no serviço, proteção de tokens em servidor e suporte do cliente ao transporte remoto; aumenta a superfície operacional.
3. **Automação da interface do LinkedIn:** descartada porque scraping e automação não autorizada são proibidos pelas regras do LinkedIn e elevam o risco de restrição da conta.

O MVP usa stdio, com separação interna entre protocolo MCP e integração LinkedIn para permitir avaliar transporte HTTP como etapa futura sem compromisso de implementação.

## Experiência e capacidades propostas

- `prepare_post`: recebe texto, valida limites e devolve uma prévia local.
- `publish_post`: recebe o texto exato e uma confirmação explícita no mesmo pedido, envia uma publicação textual via API oficial e devolve o identificador/link quando disponível. Sem confirmação, recusa a ação.
- `draft_profile_update`: recebe o campo e o texto atual/proposto fornecido pelo usuário, sugere uma versão editada e devolve um link para a página de edição do LinkedIn. Não lê ou altera a conta.
- `linkedin_connection_status`: informa a partir do cofre local se há uma credencial válida para os dois escopos permitidos e lista os escopos concedidos. O resultado contém apenas o estado da conexão e os escopos.

A publicação com mídia, leitura do perfil e publicação em páginas de organizações ficam para depois do MVP e exigem análise de escopos, aprovação e requisitos próprios.

## Segurança e privacidade

- OAuth 2.0 com PKCE nativo habilitado pelo suporte do LinkedIn e callback local com validação de `state` para proteção contra CSRF.
- Armazenar tokens no cofre do sistema operacional quando disponível; oferecer alternativa explícita e segura quando não houver cofre.
- Nunca registrar tokens, códigos OAuth ou conteúdo privado por padrão.
- Validar que o autor é o membro autenticado; não aceitar URNs arbitrárias do agente para publicar como outra pessoa.
- Confirmar exatamente o texto final antes do envio e não repetir automaticamente uma publicação após resultado ambíguo.
- Limpar/invalidar credenciais localmente e documentar revogação no LinkedIn.

## Decisões para o implementador

- Escolher crates Rust mantidos para MCP, OAuth, HTTP e armazenamento seguro após verificar compatibilidade e licença.
- Acompanhar mudanças no endpoint Share on LinkedIn e no protocolo Rest.li.
- Determinar como iniciar o callback OAuth de forma consistente nos sistemas operacionais suportados.
- Propor instalação e configuração documentadas para os clientes MCP inicialmente selecionados.

## Questões ainda em aberto

Não há questões que impeçam o plano. Decisões assumidas para o primeiro marco:

- Priorizar Linux, conforme o ambiente atual, e documentar configuração para Codex, Claude e outros clientes que aceitem MCP por stdio; suporte depende do transporte MCP que cada cliente oferece.
- Exigir os dois produtos e escopos para publicar. Credenciais locais do fluxo anterior conservam apenas `w_member_social` e precisam de nova autorização.
- A confirmação será uma etapa explícita ligada ao texto exato publicado.
- Começar com posts públicos de texto. Anexos ficam para uma etapa futura.

## Fontes consultadas

- [LinkedIn User Agreement](https://www.linkedin.com/legal/user-agreement)
- [Prohibited software and extensions](https://www.linkedin.com/help/linkedin/answer/a1341387/prohibited-software-and-extensions)
- [Getting Access to LinkedIn APIs](https://learn.microsoft.com/en-us/linkedin/shared/authentication/getting-access)
- [Share on LinkedIn](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/share-on-linkedin)
- [People APIs](https://learn.microsoft.com/en-us/linkedin/shared/integrations/people/overview)
- [Sign In with LinkedIn using OpenID Connect](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/sign-in-with-linkedin-v2)
