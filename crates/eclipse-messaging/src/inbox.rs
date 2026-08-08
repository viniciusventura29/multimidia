//! A caixa de entrada do painel.
//!
//! O formato aqui é ditado por como o Android entrega mensagem de WhatsApp, que
//! é pelo `NotificationListenerService` — o mesmo mecanismo que o Android Auto
//! usa. Isso impõe limites que não são escolha nossa:
//!
//! - **Não existe histórico.** Só se vê o que gerou notificação enquanto o app
//!   estava ouvindo. Abrir a conversa no celular dispensa a notificação e a
//!   mensagem nunca chega aqui. Por isso a caixa nasce vazia a cada boot e nada
//!   disto é persistido: guardar daria a impressão de um histórico que não existe.
//! - **Responder tem prazo.** A resposta vai pelo `RemoteInput` da notificação;
//!   se ela foi dispensada, não há mais para onde responder. Daí o `can_reply`.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Quantas mensagens guardar por conversa.
///
/// O painel fica horas ligado. Sem teto, um grupo movimentado comeria memória a
/// viagem inteira — e ninguém lê mensagem antiga dirigindo.
const HISTORICO_MAXIMO: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Autor {
    Eles,
    Eu,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub autor: Autor,
    /// Em grupo, quem falou. Em conversa de duas pessoas, igual ao nome dela.
    pub sender: String,
    pub body: String,
    pub at: DateTime<Utc>,
    /// Foto de quem mandou, quando a notificação do Android traz o ícone grande.
    /// `None` é comum — nem toda notificação tem foto — e a tela cai numa
    /// inicial nesses casos.
    pub avatar: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub name: String,
    pub messages: Vec<Message>,
    pub unread: usize,
    /// Se a notificação ainda está viva e aceita resposta.
    pub can_reply: bool,
}

/// Uma mensagem chegando.
#[derive(Clone, Debug, PartialEq)]
pub struct IncomingMessage {
    pub conversation: String,
    pub sender: String,
    pub body: String,
    pub at: DateTime<Utc>,
    pub can_reply: bool,
    /// Foto de quem mandou, se a notificação trouxe. `None` quando não há.
    pub avatar: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inbox {
    /// Da conversa mais recente para a mais antiga.
    pub conversations: Vec<Conversation>,
}

impl Inbox {
    /// Registra uma mensagem que chegou, subindo a conversa para o topo.
    pub fn recebeu(&mut self, msg: IncomingMessage) {
        let posicao = self
            .conversations
            .iter()
            .position(|c| c.name == msg.conversation);

        let mut conversa = match posicao {
            Some(i) => self.conversations.remove(i),
            None => Conversation {
                name: msg.conversation.clone(),
                messages: Vec::new(),
                unread: 0,
                can_reply: msg.can_reply,
            },
        };

        conversa.messages.push(Message {
            autor: Autor::Eles,
            sender: msg.sender,
            body: msg.body,
            at: msg.at,
            avatar: msg.avatar,
        });
        conversa.unread += 1;
        conversa.can_reply = msg.can_reply;

        if conversa.messages.len() > HISTORICO_MAXIMO {
            let excedente = conversa.messages.len() - HISTORICO_MAXIMO;
            conversa.messages.drain(..excedente);
        }

        self.conversations.insert(0, conversa);
    }

    /// Registra uma resposta enviada.
    ///
    /// Só é chamado depois que o envio deu certo: a tela nunca mostra uma
    /// mensagem como enviada antes de ela ter saído.
    pub fn respondeu(&mut self, conversa: &str, texto: &str, quando: DateTime<Utc>) -> bool {
        let Some(alvo) = self.conversations.iter_mut().find(|c| c.name == conversa) else {
            return false;
        };

        alvo.messages.push(Message {
            autor: Autor::Eu,
            sender: "eu".to_string(),
            body: texto.to_string(),
            at: quando,
            avatar: None,
        });
        // Responder é ler.
        alvo.unread = 0;

        if alvo.messages.len() > HISTORICO_MAXIMO {
            let excedente = alvo.messages.len() - HISTORICO_MAXIMO;
            alvo.messages.drain(..excedente);
        }

        true
    }

    pub fn marcou_lida(&mut self, conversa: &str) {
        if let Some(alvo) = self.conversations.iter_mut().find(|c| c.name == conversa) {
            alvo.unread = 0;
        }
    }

    pub fn nao_lidas(&self) -> usize {
        self.conversations.iter().map(|c| c.unread).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chegou(conversa: &str, corpo: &str) -> IncomingMessage {
        IncomingMessage {
            conversation: conversa.to_string(),
            sender: conversa.to_string(),
            body: corpo.to_string(),
            at: Utc::now(),
            can_reply: true,
            avatar: None,
        }
    }

    #[test]
    fn mensagem_nova_cria_a_conversa_e_conta_nao_lida() {
        let mut inbox = Inbox::default();
        inbox.recebeu(chegou("Ana", "oi"));

        assert_eq!(inbox.conversations.len(), 1);
        assert_eq!(inbox.conversations[0].name, "Ana");
        assert_eq!(inbox.nao_lidas(), 1);
        assert_eq!(inbox.conversations[0].messages[0].autor, Autor::Eles);
    }

    #[test]
    fn conversa_com_mensagem_nova_sobe_para_o_topo() {
        let mut inbox = Inbox::default();
        inbox.recebeu(chegou("Ana", "oi"));
        inbox.recebeu(chegou("Bruno", "e aí"));
        inbox.recebeu(chegou("Ana", "cadê você"));

        let nomes: Vec<_> = inbox
            .conversations
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(nomes, vec!["Ana", "Bruno"]);
        assert_eq!(
            inbox.conversations[0].messages.len(),
            2,
            "a segunda mensagem tem que agrupar, não criar conversa nova"
        );
        assert_eq!(inbox.nao_lidas(), 3);
    }

    #[test]
    fn responder_registra_como_minha_e_zera_nao_lidas() {
        let mut inbox = Inbox::default();
        inbox.recebeu(chegou("Ana", "oi"));

        assert!(inbox.respondeu("Ana", "chego em 10", Utc::now()));

        let conversa = &inbox.conversations[0];
        assert_eq!(conversa.unread, 0);
        assert_eq!(conversa.messages.last().unwrap().autor, Autor::Eu);
        assert_eq!(conversa.messages.last().unwrap().body, "chego em 10");
    }

    #[test]
    fn responder_conversa_desconhecida_falha_em_vez_de_criar_uma() {
        let mut inbox = Inbox::default();
        assert!(!inbox.respondeu("Fantasma", "oi", Utc::now()));
        assert!(inbox.conversations.is_empty());
    }

    /// O painel fica horas ligado; um grupo movimentado não pode crescer sem fim.
    #[test]
    fn historico_tem_teto_e_descarta_o_mais_antigo() {
        let mut inbox = Inbox::default();
        for i in 0..HISTORICO_MAXIMO + 5 {
            inbox.recebeu(chegou("Grupo", &format!("msg {i}")));
        }

        let conversa = &inbox.conversations[0];
        assert_eq!(conversa.messages.len(), HISTORICO_MAXIMO);
        assert_eq!(
            conversa.messages[0].body, "msg 5",
            "as mais antigas é que saem"
        );
        assert_eq!(
            conversa.unread,
            HISTORICO_MAXIMO + 5,
            "descartar do histórico não apaga que elas chegaram"
        );
    }

    /// A notificação some e o `RemoteInput` vai junto: aí não há mais como responder.
    #[test]
    fn conversa_sem_notificacao_viva_nao_aceita_resposta() {
        let mut inbox = Inbox::default();
        inbox.recebeu(IncomingMessage {
            can_reply: false,
            ..chegou("Ana", "oi")
        });

        assert!(!inbox.conversations[0].can_reply);
    }

    #[test]
    fn marcar_lida_zera_so_a_conversa_certa() {
        let mut inbox = Inbox::default();
        inbox.recebeu(chegou("Ana", "oi"));
        inbox.recebeu(chegou("Bruno", "opa"));

        inbox.marcou_lida("Ana");

        assert_eq!(inbox.nao_lidas(), 1);
        let ana = inbox
            .conversations
            .iter()
            .find(|c| c.name == "Ana")
            .unwrap();
        assert_eq!(ana.unread, 0);
    }
}
