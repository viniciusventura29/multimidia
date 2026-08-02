//! O estado que um módulo expõe, e a forma serializada que chega na UI.

use std::borrow::Cow;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Identidade estável de um módulo. É a chave no barramento e na UI.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModuleId(pub Cow<'static, str>);

impl ModuleId {
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// O estado de um módulo.
///
/// `Degraded` carrega o último valor bom de propósito: um tile de RPM continua
/// mostrando o último número lido, esmaecido, em vez de piscar pra vazio quando
/// o módulo cai.
#[derive(Clone, Debug, PartialEq)]
pub enum ModuleState<T> {
    Loading,
    Ready(T),
    Degraded { last: Option<T>, reason: String },
}

impl<T> ModuleState<T> {
    /// O último valor conhecido, esteja o módulo saudável ou degradado.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Loading => None,
            Self::Ready(v) => Some(v),
            Self::Degraded { last, .. } => last.as_ref(),
        }
    }

    pub fn is_degraded(&self) -> bool {
        matches!(self, Self::Degraded { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Loading,
    Ready,
    Degraded,
}

/// O que atravessa o barramento e chega no React.
///
/// Deliberadamente plano em vez de um enum tagueado: o TypeScript do outro lado
/// consome `status` + `data` + `reason` sem narrowing de união discriminada.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateEnvelope {
    pub module: ModuleId,
    /// Contador global e monotônico — deixa a UI descartar eventos fora de ordem.
    pub seq: u64,
    pub status: Status,
    /// O último valor bom. Sobrevive à degradação.
    ///
    /// `Arc` porque o envelope é clonado a cada publicação (para o `latest` e
    /// para o broadcast) e a cada snapshot — na cadência do OBD isso era um
    /// deep-clone de JSON por leitura de PID. O JSON serializado não muda em
    /// nada (`serde` com feature `rc` atravessa o `Arc`).
    pub data: Option<Arc<Value>>,
    /// Preenchido só quando `status` é `degraded`.
    pub reason: Option<String>,
}

impl StateEnvelope {
    pub fn is_degraded(&self) -> bool {
        self.status == Status::Degraded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_preserva_o_ultimo_valor() {
        let s = ModuleState::Degraded {
            last: Some(42),
            reason: "sem rede".into(),
        };
        assert_eq!(s.value(), Some(&42));
        assert!(s.is_degraded());
    }

    #[test]
    fn envelope_serializa_plano_para_a_ui() {
        let env = StateEnvelope {
            module: ModuleId::new("obd"),
            seq: 7,
            status: Status::Degraded,
            data: Some(Arc::new(serde_json::json!({ "rpm": 2500 }))),
            reason: Some("adaptador desconectado".into()),
        };
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["module"], "obd");
        assert_eq!(json["status"], "degraded");
        assert_eq!(json["data"]["rpm"], 2500);
        assert_eq!(json["reason"], "adaptador desconectado");
    }
}
