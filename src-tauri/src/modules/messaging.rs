//! Mensagens.
//!
//! O módulo mantém a caixa de entrada e a publica inteira a cada mudança. É
//! barato: são poucas conversas com histórico limitado, e mandar o estado
//! completo evita a UI ter que reconstruir nada.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use eclipse_messaging::{Inbox, MessageSource, MockMessageSource};

pub const MESSAGING: ModuleId = ModuleId::new("messaging");

pub struct MessagingModule {
    fonte: Box<dyn MessageSource>,
}

impl MessagingModule {
    pub fn new(fonte: Box<dyn MessageSource>) -> Self {
        Self { fonte }
    }
}

impl Default for MessagingModule {
    fn default() -> Self {
        Self::new(Box::new(MockMessageSource::new(Duration::from_secs(12))))
    }
}

#[async_trait]
impl Module for MessagingModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        // Nasce vazia todo boot, de propósito: o Android só entrega o que virou
        // notificação enquanto o app estava ouvindo. Persistir daria a impressão
        // de um histórico que não existe.
        let mut inbox = Inbox::default();
        ctx.ready(&inbox);

        loop {
            tokio::select! {
                chegando = self.fonte.next_message() => {
                    let Some(msg) = chegando else {
                        // A fonte acabou (o roteiro do mock terminou). Não é
                        // falha: fica parada esperando ordens.
                        while ctx.next_command().await.is_some() {}
                        return Ok(());
                    };
                    inbox.recebeu(msg);
                    ctx.ready(&inbox);
                }

                comando = ctx.next_command() => match comando {
                    None => return Ok(()),

                    // WhatsApp é uma conta por aparelho: trocar de perfil não
                    // troca de conta. O que muda é de quem é a tela — então a
                    // caixa é limpa para não vazar conversa entre perfis.
                    Some(ModuleCommand::ProfileChanged(_)) => {
                        inbox = Inbox::default();
                        ctx.ready(&inbox);
                    }

                    Some(ModuleCommand::Action { payload, .. }) => {
                        let acao = payload.get("acao").and_then(|v| v.as_str());
                        let conversa = payload.get("conversa").and_then(|v| v.as_str());

                        match (acao, conversa) {
                            (Some("responder"), Some(conversa)) => {
                                let texto = payload
                                    .get("texto")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default();
                                if texto.is_empty() {
                                    continue;
                                }

                                // Só registra depois que saiu: a tela nunca
                                // mostra como enviada uma mensagem que falhou.
                                match self.fonte.reply(conversa, texto).await {
                                    Ok(()) => {
                                        inbox.respondeu(conversa, texto, Utc::now());
                                        ctx.ready(&inbox);
                                    }
                                    Err(err) => ctx.degraded(err.to_string()),
                                }
                            }
                            (Some("lida"), Some(conversa)) => {
                                inbox.marcou_lida(conversa);
                                ctx.ready(&inbox);
                            }
                            _ => continue,
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eclipse_core::{factory, StateEnvelope, Status, Supervisor};
    use eclipse_messaging::{IncomingMessage, MessagingError};
    use serde_json::json;
    use tokio::sync::broadcast::Receiver;

    /// Entrega uma mensagem e depois fica quieta; pode ser configurada para
    /// falhar no envio.
    struct FonteDeTeste {
        entregue: bool,
        falha_ao_responder: bool,
    }

    #[async_trait]
    impl MessageSource for FonteDeTeste {
        async fn next_message(&mut self) -> Option<IncomingMessage> {
            if self.entregue {
                // Nunca resolve: deixa o select! livre para os comandos.
                std::future::pending::<()>().await;
            }
            self.entregue = true;
            Some(IncomingMessage {
                conversation: "Ana".into(),
                sender: "Ana".into(),
                body: "oi".into(),
                at: Utc::now(),
                can_reply: true,
            })
        }

        async fn reply(&mut self, _c: &str, _t: &str) -> Result<(), MessagingError> {
            if self.falha_ao_responder {
                return Err(MessagingError::RespostaExpirada);
            }
            Ok(())
        }
    }

    fn subir(falha: bool) -> (Supervisor, Receiver<StateEnvelope>) {
        let mut supervisor = Supervisor::new();
        let rx = supervisor.subscribe();
        supervisor.spawn(factory(MESSAGING, move || {
            MessagingModule::new(Box::new(FonteDeTeste {
                entregue: false,
                falha_ao_responder: falha,
            }))
        }));
        (supervisor, rx)
    }

    async fn proximo(
        rx: &mut Receiver<StateEnvelope>,
        aceita: impl Fn(&StateEnvelope) -> bool,
    ) -> StateEnvelope {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let env = rx.recv().await.expect("barramento fechou");
                if env.module == MESSAGING && aceita(&env) {
                    return env;
                }
            }
        })
        .await
        .expect("estado não chegou")
    }

    #[tokio::test(start_paused = true)]
    async fn mensagem_recebida_aparece_na_caixa() {
        let (_sup, mut rx) = subir(false);

        let env = proximo(&mut rx, |e| {
            e.data
                .as_ref()
                .and_then(|d| d["conversations"].as_array())
                .is_some_and(|c| !c.is_empty())
        })
        .await;

        let conversas = env.data.unwrap()["conversations"].clone();
        assert_eq!(conversas[0]["name"], "Ana");
        assert_eq!(conversas[0]["unread"], 1);
    }

    #[tokio::test(start_paused = true)]
    async fn responder_registra_na_conversa_e_zera_nao_lidas() {
        let (supervisor, mut rx) = subir(false);
        proximo(&mut rx, |e| {
            e.data
                .as_ref()
                .and_then(|d| d["conversations"].as_array())
                .is_some_and(|c| !c.is_empty())
        })
        .await;

        supervisor.dispatch(ModuleCommand::Action {
            target: MESSAGING,
            payload: json!({ "acao": "responder", "conversa": "Ana", "texto": "chego em 10" }),
        });

        let env = proximo(&mut rx, |e| {
            e.data
                .as_ref()
                .and_then(|d| d["conversations"][0]["unread"].as_u64())
                == Some(0)
        })
        .await;

        let mensagens = env.data.unwrap()["conversations"][0]["messages"].clone();
        let ultima = mensagens.as_array().unwrap().last().unwrap();
        assert_eq!(ultima["autor"], "eu");
        assert_eq!(ultima["body"], "chego em 10");
    }

    /// Se o envio falha, a mensagem não pode aparecer como enviada.
    #[tokio::test(start_paused = true)]
    async fn falha_no_envio_degrada_em_vez_de_fingir_que_enviou() {
        let (supervisor, mut rx) = subir(true);
        proximo(&mut rx, |e| {
            e.data
                .as_ref()
                .and_then(|d| d["conversations"].as_array())
                .is_some_and(|c| !c.is_empty())
        })
        .await;

        supervisor.dispatch(ModuleCommand::Action {
            target: MESSAGING,
            payload: json!({ "acao": "responder", "conversa": "Ana", "texto": "oi" }),
        });

        let env = proximo(&mut rx, |e| e.status == Status::Degraded).await;
        assert!(env.reason.unwrap().contains("não aceita mais resposta"));

        let conversas = env.data.unwrap()["conversations"].clone();
        let mensagens = conversas[0]["messages"].as_array().unwrap().clone();
        assert_eq!(mensagens.len(), 1, "a resposta que falhou não pode entrar");
    }
}
