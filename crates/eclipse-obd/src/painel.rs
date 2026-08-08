//! O que o painel do carro mostra, montado num lugar só.
//!
//! Junta a varredura ([`Poller`]) com a conta de consumo ([`Medidor`]) e entrega o
//! pacote que a tela recebe. Existe para o `src-tauri` continuar sendo só fiação:
//! conectar o Bluetooth, laçar, publicar, gravar — sem regra nenhuma.
//!
//! O tempo entra por parâmetro em [`Painel::step`]. É o que permite provar a
//! integração de litros com um relógio de mentira, em vez de dirigindo.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::capacidades::Capacidades;
use crate::consumo::{Consumo, Medidor, Tanque, Viagem};
use crate::pid::Readings;
use crate::poller::Poller;
use crate::source::{ObdError, ObdSource};
use crate::veiculo::{EstadoTanque, Veiculo};

/// Tudo o que a tela recebe do módulo `obd`.
///
/// As leituras vêm **achatadas** no topo de propósito: o header lê `voltage` e
/// `fuelPct` direto do estado do módulo, fora do sistema de quadros, e aninhá-las
/// quebraria isso sem ganho nenhum.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetria<'a> {
    #[serde(flatten)]
    pub leituras: &'a Readings,
    pub consumo: Consumo,
    pub tanque: Tanque,
    pub viagem: Viagem,
    pub capacidades: Capacidades,
    /// Como o adaptador descreve o barramento que negociou. Só para o diagnóstico.
    pub protocolo: Option<&'a str>,
}

/// O que o usuário pode mandar o painel fazer.
///
/// Vem do toque na tela como `{ "acao": "abasteci", "litros": 12.5 }`. É o serde que
/// valida: uma ação com nome errado ou campo faltando não vira `Acao` nenhuma, em vez
/// de virar meia ação silenciosa.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(
    tag = "acao",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Acao {
    /// Enchi o tanque: o nível vira a capacidade e a média do tanque recomeça.
    Enchi,
    /// Coloquei N litros.
    Abasteci {
        litros: f32,
    },
    /// O nível é este — correção à mão, sem ter abastecido.
    Nivel {
        litros: f32,
    },
    /// O tanque deste carro é de N litros.
    Tanque {
        capacidade_l: f32,
    },
    /// A calibração do consumo é esta (1,0 = sem correção).
    Calibrar {
        fator: f32,
    },
    ZerarViagem,
}

impl Acao {
    /// Mexe na configuração do carro, e não só no estado do tanque?
    ///
    /// Separa o que vai para `veiculo.json` do que vai para `tanque.json`: ajuste é
    /// raro e estado é constante, e juntá-los faria cada gota de gasolina reescrever
    /// a configuração.
    pub fn muda_o_veiculo(self) -> bool {
        matches!(self, Self::Tanque { .. } | Self::Calibrar { .. })
    }
}

/// A varredura, a conta e o estado do tanque, juntos.
pub struct Painel<S> {
    poller: Poller<S>,
    medidor: Medidor,
    protocolo: Option<String>,
    /// Quando foi a leitura anterior, para medir o `Δt` de verdade.
    ///
    /// `None` na primeira: sem intervalo conhecido não se integra nada — é o mesmo
    /// cuidado que descarta o buraco de uma reconexão.
    ultima: Option<Instant>,
}

impl<S: ObdSource> Painel<S> {
    pub fn novo(
        poller: Poller<S>,
        veiculo: Veiculo,
        estado: EstadoTanque,
        protocolo: Option<String>,
    ) -> Self {
        let medidor = Medidor::novo(veiculo, estado, poller.capacidades());
        Self {
            poller,
            medidor,
            protocolo,
            ultima: None,
        }
    }

    /// Lê o próximo PID e atualiza a conta.
    pub async fn step(&mut self, agora: Instant) -> Result<(), ObdError> {
        self.poller.step().await?;

        // A varredura pode ter se remontado sozinha (um PID parou de responder), e aí
        // a fonte de vazão desce um degrau da cascata junto — e o nível de
        // combustível pode ter saído da roda.
        if self.poller.replanejou() {
            self.medidor
                .atualizar_capacidades(self.poller.capacidades());
        }

        let dt = match self.ultima.replace(agora) {
            Some(antes) => agora.saturating_duration_since(antes),
            None => Duration::ZERO,
        };
        self.medidor.amostrar(self.poller.readings(), dt);
        Ok(())
    }

    pub fn telemetria(&self) -> Telemetria<'_> {
        let leituras = self.poller.readings();
        Telemetria {
            leituras,
            consumo: self.medidor.consumo(leituras),
            tanque: self.medidor.tanque(),
            viagem: self.medidor.viagem(),
            capacidades: self.poller.capacidades(),
            protocolo: self.protocolo.as_deref(),
        }
    }

    /// Aplica uma ação do usuário. Devolve `true` quando há algo novo para gravar.
    pub fn aplicar(&mut self, acao: Acao) -> bool {
        match acao {
            Acao::Enchi => self.medidor.encheu(),
            Acao::Abasteci { litros } => self.medidor.abasteceu(litros),
            Acao::Nivel { litros } => self.medidor.corrigiu_nivel(litros),
            Acao::ZerarViagem => self.medidor.zerar_viagem(),
            Acao::Tanque { capacidade_l } => {
                let veiculo = Veiculo {
                    capacidade_l,
                    ..self.medidor.veiculo()
                };
                self.medidor.ajustar(veiculo);
            }
            Acao::Calibrar { fator } => {
                let veiculo = Veiculo {
                    calibracao: fator,
                    ..self.medidor.veiculo()
                };
                self.medidor.ajustar(veiculo);
            }
        }
        true
    }

    pub fn estado(&self) -> EstadoTanque {
        self.medidor.estado()
    }

    pub fn veiculo(&self) -> Veiculo {
        self.medidor.veiculo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::Pid;
    use async_trait::async_trait;
    use serde_json::json;

    /// Um carro de mentira que responde sempre o mesmo para cada PID.
    struct CarroFixo;

    #[async_trait]
    impl ObdSource for CarroFixo {
        async fn read(&mut self, pid: Pid) -> Result<f32, ObdError> {
            Ok(match pid {
                Pid::Rpm => 2_500.0,
                Pid::Speed => 100.0,
                Pid::Maf => 22.0,
                Pid::Coolant => 88.0,
                Pid::Voltage => 14.1,
                // Este carro não informa o nível de combustível — o caso que a
                // estimativa integrada existe para cobrir.
                Pid::Fuel => return Err(ObdError::Unsupported),
                _ => 0.0,
            })
        }
    }

    fn painel() -> Painel<CarroFixo> {
        let mut cap = Capacidades::otimista();
        for base in [0x00, 0x20, 0x40] {
            cap.juntar(base, &[0, 0, 0, 0]);
        }
        for pid in [0x05, 0x0C, 0x0D, 0x10] {
            cap.marcar(pid);
        }
        Painel::novo(
            Poller::com_capacidades(CarroFixo, cap),
            Veiculo::default(),
            EstadoTanque::default(),
            Some("ISO 9141-2".into()),
        )
    }

    /// Roda `voltas` leituras espaçadas de `ms`.
    async fn rodar(p: &mut Painel<CarroFixo>, ms: u64, voltas: u32) {
        let mut t = Instant::now();
        for _ in 0..voltas {
            t += Duration::from_millis(ms);
            p.step(t).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_telemetria_sai_com_as_leituras_achatadas() {
        let mut p = painel();
        p.aplicar(Acao::Enchi);
        rodar(&mut p, 300, 20).await;

        let json = serde_json::to_value(p.telemetria()).unwrap();
        // O header lê estes dois direto da raiz: aninhar quebraria a barra de status.
        assert!(json["voltage"].is_number(), "{json}");
        assert!(json.get("fuelPct").is_some());
        assert!(json["consumo"]["litrosHora"].is_number());
        assert!(json["tanque"]["autonomiaKm"].is_number());
        assert!(json["viagem"]["distanciaKm"].is_number());
        assert_eq!(json["protocolo"], "ISO 9141-2");
    }

    #[tokio::test]
    async fn a_primeira_leitura_nao_integra_nada() {
        let mut p = painel();
        p.aplicar(Acao::Enchi);
        p.step(Instant::now()).await.unwrap();

        // Sem intervalo anterior não há como saber quanto tempo passou — integrar
        // com um Δt inventado é o começo de um tanque que anda para trás sozinho.
        assert_eq!(p.telemetria().viagem.litros, 0.0);
    }

    #[tokio::test]
    async fn rodando_a_cem_por_hora_o_tanque_baixa_e_a_media_aparece() {
        let mut p = painel();
        p.aplicar(Acao::Enchi);
        // 10 min de estrada, uma leitura a cada 300 ms como o barramento entrega.
        rodar(&mut p, 300, 2_000).await;

        let t = p.telemetria();
        assert!(t.viagem.distancia_km > 15.0, "km {}", t.viagem.distancia_km);
        assert!(t.consumo.instantaneo_km_l.unwrap() > 5.0);
        assert!(t.tanque.litros.unwrap() < 61.0, "gastou gasolina");
        assert!(t.tanque.autonomia_km.unwrap() > 0.0);
        assert!(!t.tanque.medido, "sem PID de nível, o tanque é estimativa");
    }

    #[tokio::test]
    async fn as_acoes_chegam_como_json_da_tela() {
        let mut p = painel();

        let acao: Acao = serde_json::from_value(json!({ "acao": "enchi" })).unwrap();
        p.aplicar(acao);
        assert_eq!(p.telemetria().tanque.litros, Some(61.0));

        let acao: Acao =
            serde_json::from_value(json!({ "acao": "tanque", "capacidadeL": 45.0 })).unwrap();
        assert!(acao.muda_o_veiculo());
        p.aplicar(acao);
        assert_eq!(p.veiculo().capacidade_l, 45.0);

        let acao: Acao =
            serde_json::from_value(json!({ "acao": "calibrar", "fator": 1.1 })).unwrap();
        p.aplicar(acao);
        assert!((p.veiculo().calibracao - 1.1).abs() < 0.001);

        let acao: Acao = serde_json::from_value(json!({ "acao": "zerarViagem" })).unwrap();
        assert!(!acao.muda_o_veiculo());
        p.aplicar(acao);
        assert_eq!(p.telemetria().viagem.distancia_km, 0.0);
    }

    #[test]
    fn acao_desconhecida_nao_vira_meia_acao() {
        // Um toque que chegue com o nome errado tem que falhar aqui, e não aplicar
        // metade de algo: é dinheiro de gasolina na tela.
        assert!(serde_json::from_value::<Acao>(json!({ "acao": "explodir" })).is_err());
        assert!(serde_json::from_value::<Acao>(json!({ "acao": "abasteci" })).is_err());
    }

    #[tokio::test]
    async fn o_estado_do_tanque_sobrevive_a_reconstrucao_do_modulo() {
        let mut p = painel();
        p.aplicar(Acao::Enchi);
        rodar(&mut p, 300, 2_000).await;
        let salvo = p.estado();
        let config = p.veiculo();

        // É o que o supervisor faz a cada tranco no conector do adaptador: joga o
        // módulo fora e constrói outro. O tanque não pode virar mistério de novo.
        let mut de_novo = Painel::novo(
            Poller::com_capacidades(CarroFixo, Capacidades::otimista()),
            config,
            salvo,
            None,
        );
        rodar(&mut de_novo, 300, 4).await;

        let t = de_novo.telemetria();
        assert!((t.tanque.litros.unwrap() - salvo.litros.unwrap()).abs() < 0.1);
        assert!(
            t.viagem.distancia_km > 15.0,
            "a viagem continua de onde parou"
        );
    }
}
