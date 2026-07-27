//! O contrato de um módulo e o canal por onde ele fala com o resto do sistema.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::profile::Profile;
use crate::state::{ModuleId, StateEnvelope, Status};

/// Qualquer erro que um módulo queira propagar. Aceita `?` sobre qualquer
/// `std::error::Error`, então módulos não precisam de um tipo de erro próprio.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type ModuleResult = Result<(), BoxError>;

/// Ordens vindas de fora do módulo: troca de perfil e ações do usuário.
#[derive(Clone, Debug)]
pub enum ModuleCommand {
    ProfileChanged(Arc<Profile>),
    Action { target: ModuleId, payload: Value },
}

/// Estado compartilhado entre o supervisor e os módulos que ele governa.
#[derive(Clone)]
pub(crate) struct Bus {
    events: broadcast::Sender<StateEnvelope>,
    commands: broadcast::Sender<ModuleCommand>,
    latest: Arc<Mutex<HashMap<ModuleId, StateEnvelope>>>,
    seq: Arc<AtomicU64>,
}

impl Bus {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            events: broadcast::Sender::new(capacity),
            commands: broadcast::Sender::new(capacity),
            latest: Arc::new(Mutex::new(HashMap::new())),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Publica um novo estado.
    ///
    /// Quando `data` é `None` — degradação, ou volta pra `Loading` — o último
    /// valor bom conhecido é herdado. É isso que mantém o número no tile em vez
    /// de piscar pra vazio quando o módulo cai.
    pub(crate) fn publish(
        &self,
        module: &ModuleId,
        status: Status,
        data: Option<Value>,
        reason: Option<String>,
    ) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());

        let data = data.or_else(|| latest.get(module).and_then(|prev| prev.data.clone()));
        let envelope = StateEnvelope {
            module: module.clone(),
            seq,
            status,
            data,
            reason,
        };

        latest.insert(module.clone(), envelope.clone());
        drop(latest);

        // Sem assinantes não é erro: o app pode estar sem janela aberta ainda.
        let _ = self.events.send(envelope);
    }

    pub(crate) fn subscribe_events(&self) -> broadcast::Receiver<StateEnvelope> {
        self.events.subscribe()
    }

    pub(crate) fn subscribe_commands(&self) -> broadcast::Receiver<ModuleCommand> {
        self.commands.subscribe()
    }

    pub(crate) fn dispatch(&self, command: ModuleCommand) {
        let _ = self.commands.send(command);
    }

    pub(crate) fn snapshot(&self) -> Vec<StateEnvelope> {
        let latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        let mut all: Vec<_> = latest.values().cloned().collect();
        all.sort_by(|a, b| a.module.cmp(&b.module));
        all
    }
}

/// O canal de um módulo com o mundo: por onde ele publica estado e recebe ordens.
pub struct ModuleCtx {
    id: ModuleId,
    bus: Bus,
    commands: broadcast::Receiver<ModuleCommand>,
}

impl ModuleCtx {
    pub(crate) fn new(id: ModuleId, bus: Bus) -> Self {
        let commands = bus.subscribe_commands();
        Self { id, bus, commands }
    }

    pub fn id(&self) -> &ModuleId {
        &self.id
    }

    pub fn loading(&self) {
        self.bus.publish(&self.id, Status::Loading, None, None);
    }

    /// Publica um novo valor bom.
    ///
    /// Se a serialização falhar, o módulo é degradado em vez de entrar em pânico —
    /// um bug de `Serialize` num módulo não deve derrubar o painel.
    pub fn ready<T: Serialize>(&self, value: &T) {
        match serde_json::to_value(value) {
            Ok(data) => self
                .bus
                .publish(&self.id, Status::Ready, Some(data), None),
            Err(err) => self.degraded(format!("falha ao serializar o estado: {err}")),
        }
    }

    pub fn degraded(&self, reason: impl Into<String>) {
        self.bus
            .publish(&self.id, Status::Degraded, None, Some(reason.into()));
    }

    /// Espera a próxima ordem endereçada a este módulo.
    ///
    /// Ações para outros módulos são ignoradas aqui. Se o módulo ficar lento e
    /// perder mensagens, o canal pula as antigas em vez de travar o barramento.
    pub async fn next_command(&mut self) -> Option<ModuleCommand> {
        loop {
            match self.commands.recv().await {
                Ok(ModuleCommand::Action { target, .. }) if target != self.id => continue,
                Ok(command) => return Some(command),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(module = %self.id, skipped, "módulo perdeu comandos");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Uma funcionalidade do painel.
///
/// `run` deve laçar até ser cancelado. Voltar `Ok(())` significa "terminei de
/// propósito" e o supervisor para de reiniciar; voltar `Err` ou entrar em pânico
/// significa falha, e o supervisor degrada e tenta de novo.
#[async_trait]
pub trait Module: Send {
    async fn run(&mut self, ctx: ModuleCtx) -> ModuleResult;
}

/// Constrói instâncias de um módulo.
///
/// O supervisor cria uma instância nova a cada tentativa em vez de reaproveitar
/// a que quebrou — depois de um pânico, o estado interno do módulo não é confiável.
pub trait ModuleFactory: Send + Sync + 'static {
    fn id(&self) -> ModuleId;
    fn create(&self) -> Box<dyn Module>;
}

/// Adapta uma closure a [`ModuleFactory`].
pub struct FnFactory<F> {
    id: ModuleId,
    build: F,
}

/// Registra um módulo a partir de uma closure que o constrói.
pub fn factory<F, M>(id: ModuleId, build: F) -> FnFactory<F>
where
    F: Fn() -> M + Send + Sync + 'static,
    M: Module + 'static,
{
    FnFactory { id, build }
}

impl<F, M> ModuleFactory for FnFactory<F>
where
    F: Fn() -> M + Send + Sync + 'static,
    M: Module + 'static,
{
    fn id(&self) -> ModuleId {
        self.id.clone()
    }

    fn create(&self) -> Box<dyn Module> {
        Box::new((self.build)())
    }
}
