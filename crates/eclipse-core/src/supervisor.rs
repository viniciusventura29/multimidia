//! O supervisor: mantém cada módulo vivo sem deixar que a queda de um derrube os outros.

use std::any::Any;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::module::{Bus, ModuleCommand, ModuleCtx, ModuleFactory};
use crate::state::{StateEnvelope, Status};

const MIN_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Uma tentativa que durou pelo menos isso é considerada saudável, e zera o backoff.
const HEALTHY_AFTER: Duration = Duration::from_secs(30);

/// Backoff exponencial entre reinícios, limitado no teto.
struct Backoff {
    next: Duration,
}

impl Backoff {
    fn new() -> Self {
        Self { next: MIN_BACKOFF }
    }

    fn reset(&mut self) {
        self.next = MIN_BACKOFF;
    }

    fn take(&mut self) -> Duration {
        let current = self.next;
        self.next = (self.next * 2).min(MAX_BACKOFF);
        current
    }
}

/// Extrai a mensagem de um pânico, que chega como `Box<dyn Any>`.
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "pânico sem mensagem".to_string()
}

/// Governa o ciclo de vida de todos os módulos.
///
/// Cada módulo roda na própria task. Se ela devolve erro **ou entra em pânico**,
/// o supervisor marca o módulo como degradado, avisa a UI e reagenda com backoff.
/// Os demais módulos não percebem nada.
pub struct Supervisor {
    bus: Bus,
    tasks: Vec<JoinHandle<()>>,
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            bus: Bus::new(256),
            tasks: Vec::new(),
        }
    }

    /// Ouve as mudanças de estado de todos os módulos.
    pub fn subscribe(&self) -> broadcast::Receiver<StateEnvelope> {
        self.bus.subscribe_events()
    }

    /// O estado atual de todos os módulos, ordenado por id.
    ///
    /// Um assinante novo não recebe o que já passou pelo barramento, então a UI
    /// chama isso ao montar para pintar a tela antes do primeiro evento.
    pub fn snapshot(&self) -> Vec<StateEnvelope> {
        self.bus.snapshot()
    }

    pub fn dispatch(&self, command: ModuleCommand) {
        self.bus.dispatch(command);
    }

    /// Coloca um módulo no ar e passa a supervisioná-lo.
    pub fn spawn<F: ModuleFactory>(&mut self, factory: F) {
        let bus = self.bus.clone();
        let id = factory.id();

        let handle = tokio::spawn(async move {
            let mut backoff = Backoff::new();

            loop {
                let ctx = ModuleCtx::new(id.clone(), bus.clone());
                ctx.loading();

                let mut module = factory.create();
                let started = Instant::now();
                let outcome = tokio::spawn(async move { module.run(ctx).await }).await;

                let reason = match outcome {
                    Ok(Ok(())) => {
                        tracing::info!(module = %id, "módulo encerrou por conta própria");
                        return;
                    }
                    Ok(Err(err)) => err.to_string(),
                    Err(join_err) if join_err.is_panic() => {
                        format!("pânico: {}", panic_message(join_err.into_panic()))
                    }
                    // Task cancelada: desligamento, não falha.
                    Err(_) => return,
                };

                tracing::error!(module = %id, reason, "módulo caiu, vai reiniciar");
                bus.publish(&id, Status::Degraded, None, Some(reason));

                if started.elapsed() >= HEALTHY_AFTER {
                    backoff.reset();
                }
                tokio::time::sleep(backoff.take()).await;
            }
        });

        self.tasks.push(handle);
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{factory, Module, ModuleCtx, ModuleResult};
    use crate::state::ModuleId;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Publica um valor e então entra em pânico.
    struct PublicaEQuebra {
        valor: i64,
    }

    #[async_trait]
    impl Module for PublicaEQuebra {
        async fn run(&mut self, ctx: ModuleCtx) -> ModuleResult {
            ctx.ready(&self.valor);
            panic!("o adaptador sumiu");
        }
    }

    /// Devolve erro sem publicar nada.
    struct FalhaComErro;

    #[async_trait]
    impl Module for FalhaComErro {
        async fn run(&mut self, _ctx: ModuleCtx) -> ModuleResult {
            Err("sem rede".into())
        }
    }

    /// Conta pra sempre, publicando a cada 100ms.
    struct Contador {
        tick: Arc<AtomicU32>,
    }

    #[async_trait]
    impl Module for Contador {
        async fn run(&mut self, ctx: ModuleCtx) -> ModuleResult {
            loop {
                let n = self.tick.fetch_add(1, Ordering::Relaxed);
                ctx.ready(&n);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    /// Lê eventos até `pred` aceitar um, ou estourar o limite de eventos.
    async fn espera_por(
        rx: &mut broadcast::Receiver<StateEnvelope>,
        pred: impl Fn(&StateEnvelope) -> bool,
    ) -> StateEnvelope {
        for _ in 0..500 {
            let env = rx.recv().await.expect("barramento fechou");
            if pred(&env) {
                return env;
            }
        }
        panic!("evento esperado não chegou");
    }

    #[tokio::test(start_paused = true)]
    async fn erro_do_modulo_vira_degraded() {
        let mut sup = Supervisor::new();
        let mut rx = sup.subscribe();
        sup.spawn(factory(ModuleId::new("falho"), || FalhaComErro));

        let env = espera_por(&mut rx, |e| e.is_degraded()).await;
        assert_eq!(env.module.as_str(), "falho");
        assert_eq!(env.reason.as_deref(), Some("sem rede"));
    }

    #[tokio::test(start_paused = true)]
    async fn panico_vira_degraded_com_a_mensagem() {
        let mut sup = Supervisor::new();
        let mut rx = sup.subscribe();
        sup.spawn(factory(ModuleId::new("obd"), || PublicaEQuebra { valor: 2500 }));

        let env = espera_por(&mut rx, |e| e.is_degraded()).await;
        let reason = env.reason.expect("degradação precisa dizer o motivo");
        assert!(
            reason.contains("pânico") && reason.contains("o adaptador sumiu"),
            "motivo inesperado: {reason}"
        );
    }

    /// O tile tem que continuar mostrando o último número lido, não piscar pra vazio.
    #[tokio::test(start_paused = true)]
    async fn degradacao_preserva_o_ultimo_valor_bom() {
        let mut sup = Supervisor::new();
        let mut rx = sup.subscribe();
        sup.spawn(factory(ModuleId::new("obd"), || PublicaEQuebra { valor: 2500 }));

        let env = espera_por(&mut rx, |e| e.is_degraded()).await;
        assert_eq!(
            env.data,
            Some(serde_json::json!(2500)),
            "o último valor bom precisa sobreviver à queda"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn modulo_que_quebra_e_reiniciado() {
        let mut sup = Supervisor::new();
        let mut rx = sup.subscribe();
        sup.spawn(factory(ModuleId::new("obd"), || PublicaEQuebra { valor: 1 }));

        // Duas degradações seguidas só acontecem se ele voltou a rodar no meio.
        espera_por(&mut rx, |e| e.is_degraded()).await;
        espera_por(&mut rx, |e| e.is_degraded()).await;
    }

    /// O requisito central do projeto: um módulo caindo não pode calar os outros.
    #[tokio::test(start_paused = true)]
    async fn queda_de_um_modulo_nao_afeta_os_outros() {
        let mut sup = Supervisor::new();
        let mut rx = sup.subscribe();

        let ticks = Arc::new(AtomicU32::new(0));
        let ticks_mod = ticks.clone();

        sup.spawn(factory(ModuleId::new("quebrado"), || PublicaEQuebra {
            valor: 7,
        }));
        sup.spawn(factory(ModuleId::new("saudavel"), move || Contador {
            tick: ticks_mod.clone(),
        }));

        // Espera o módulo quebrado degradar...
        espera_por(&mut rx, |e| {
            e.module.as_str() == "quebrado" && e.is_degraded()
        })
        .await;

        // ...e confirma que o saudável seguiu publicando valores novos depois disso.
        let primeiro = espera_por(&mut rx, |e| {
            e.module.as_str() == "saudavel" && e.status == Status::Ready
        })
        .await;
        let segundo = espera_por(&mut rx, |e| {
            e.module.as_str() == "saudavel" && e.status == Status::Ready && e.data != primeiro.data
        })
        .await;

        assert!(segundo.seq > primeiro.seq);
        assert!(ticks.load(Ordering::Relaxed) >= 2);
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_traz_o_estado_de_todos_os_modulos() {
        let mut sup = Supervisor::new();
        let mut rx = sup.subscribe();

        sup.spawn(factory(ModuleId::new("a"), || PublicaEQuebra { valor: 10 }));
        sup.spawn(factory(ModuleId::new("b"), || PublicaEQuebra { valor: 20 }));

        espera_por(&mut rx, |e| e.module.as_str() == "a" && e.is_degraded()).await;
        espera_por(&mut rx, |e| e.module.as_str() == "b" && e.is_degraded()).await;

        let snapshot = sup.snapshot();
        let ids: Vec<_> = snapshot.iter().map(|e| e.module.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"], "snapshot vem ordenado por id");
        assert_eq!(snapshot[0].data, Some(serde_json::json!(10)));
        assert_eq!(snapshot[1].data, Some(serde_json::json!(20)));
    }

    /// Um `broadcast` não entrega o que passou. Sem o supervisor guardar o
    /// perfil, um módulo que sobe depois do anúncio — no boot, ou renascendo de
    /// um pânico — passaria a vida sem saber de quem são as contas.
    #[tokio::test(start_paused = true)]
    async fn modulo_que_sobe_depois_ainda_recebe_o_perfil_ativo() {
        /// Publica o nome de quem está dirigindo, ou nada se não souber.
        struct QuemDirige;

        #[async_trait]
        impl Module for QuemDirige {
            async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
                while let Some(comando) = ctx.next_command().await {
                    if let ModuleCommand::ProfileChanged(perfil) = comando {
                        ctx.ready(&perfil.name);
                    }
                }
                Ok(())
            }
        }

        let mut sup = Supervisor::new();
        let mut rx = sup.subscribe();

        // O perfil é anunciado ANTES de o módulo existir.
        sup.dispatch(ModuleCommand::ProfileChanged(Arc::new(
            crate::Profile::new("Vinicius", "#3ddc97"),
        )));
        sup.spawn(factory(ModuleId::new("quem"), || QuemDirige));

        let env = espera_por(&mut rx, |e| e.status == Status::Ready).await;
        assert_eq!(env.data, Some(serde_json::json!("Vinicius")));
    }

    #[test]
    fn backoff_cresce_ate_o_teto_e_zera_no_reset() {
        let mut b = Backoff::new();
        assert_eq!(b.take(), Duration::from_millis(500));
        assert_eq!(b.take(), Duration::from_secs(1));
        assert_eq!(b.take(), Duration::from_secs(2));

        for _ in 0..20 {
            b.take();
        }
        assert_eq!(b.take(), MAX_BACKOFF, "não pode passar do teto");

        b.reset();
        assert_eq!(b.take(), MIN_BACKOFF);
    }
}
