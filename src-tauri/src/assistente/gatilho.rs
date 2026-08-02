//! Quando o assistente resolve falar.
//!
//! Não há entrada: ninguém digita nem fala com ele. Então o que substitui o
//! pedido do usuário é o acontecimento — o carro ligou, uma rota foi traçada, o
//! motor esquentou, chegamos. Cada acontecimento vira uma mensagem curta que
//! serve de pedido; o resto o modelo busca sozinho pelas ferramentas.
//!
//! O [`Detector`] observa o barramento e só devolve gatilho em **transição**.
//! O OBD publica três vezes por segundo; reagir a valor, e não a mudança de
//! valor, seria uma chamada de API a cada leitura.

use std::collections::HashSet;

use eclipse_core::StateEnvelope;
use eclipse_ia::Modelo;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Gatilho {
    /// O painel acabou de subir.
    Ignicao,
    /// Um destino novo foi traçado no mapa.
    RotaDefinida,
    /// A telemetria cruzou um limiar que merece aviso.
    AlertaCarro,
    /// Viagem longa em curso.
    Periodico,
    /// Chegamos.
    Chegada,
}

impl Gatilho {
    /// Qual modelo atende.
    ///
    /// O critério é quanto trabalho de pesquisa o gatilho pede, não a
    /// importância dele. Saudação e comentário de estrada são resumo de coisas
    /// que as ferramentas já entregam prontas — Haiku dá conta. Destino novo e
    /// alerta do carro exigem cruzar telemetria, clima e caminho, e aí a
    /// qualidade da leitura paga o preço do Opus.
    pub fn modelo(self) -> Modelo {
        match self {
            Self::Ignicao | Self::Periodico => Modelo::Haiku,
            Self::RotaDefinida | Self::AlertaCarro | Self::Chegada => Modelo::Opus,
        }
    }

    /// Quanto tempo tem que passar antes de este gatilho poder disparar de novo.
    pub fn descanso_min(self) -> i64 {
        match self {
            // Parar para abastecer e voltar não é ligar o carro de novo.
            Self::Ignicao => 6 * 60,
            Self::RotaDefinida => 5,
            // O rearme de verdade é a histerese em [`Alerta`]; este descanso só
            // impede dois alertas diferentes se atropelarem.
            Self::AlertaCarro => 10,
            Self::Periodico => 20,
            Self::Chegada => 30,
        }
    }

    pub fn como_texto(self) -> &'static str {
        match self {
            Self::Ignicao => "ignicao",
            Self::RotaDefinida => "rotaDefinida",
            Self::AlertaCarro => "alertaCarro",
            Self::Periodico => "periodico",
            Self::Chegada => "chegada",
        }
    }
}

/// O que o assistente recebe no lugar de um pedido do usuário.
#[derive(Clone, Debug, PartialEq)]
pub struct Acionamento {
    pub gatilho: Gatilho,
    pub pedido: String,
}

impl Acionamento {
    pub fn ignicao() -> Self {
        Self {
            gatilho: Gatilho::Ignicao,
            pedido: "O painel acabou de ligar. Veja que dia e que hora são, como está o \
                     carro, e diga o que for útil para agora — o tempo, o que está \
                     acontecendo por aqui, o que a pessoa provavelmente vai querer saber \
                     antes de sair."
                .into(),
        }
    }

    pub fn periodico() -> Self {
        Self {
            gatilho: Gatilho::Periodico,
            pedido: "A viagem está durando. Traga alguma coisa do caminho ou do destino que \
                     ainda não foi dita — como está o trânsito daqui pra frente, o tempo lá \
                     na frente, o que tem por perto. Se não houver novidade que valha, não \
                     pinte nada."
                .into(),
        }
    }

    pub fn rota(destino: &str) -> Self {
        Self {
            gatilho: Gatilho::RotaDefinida,
            pedido: format!(
                "Foi traçada uma rota para {destino}. Pesquise sobre o destino e o caminho: \
                 como está o tempo lá, como está o trânsito no trajeto, o que existe por lá \
                 que valha mencionar. Diga também se o carro está em condição de fazer essa \
                 distância."
            ),
        }
    }

    pub fn chegada(destino: &str) -> Self {
        Self {
            gatilho: Gatilho::Chegada,
            pedido: format!(
                "Chegamos em {destino}. Feche a viagem: o que vale saber de agora em diante \
                 aqui — o tempo, o que tem por perto, como está o carro depois do trajeto."
            ),
        }
    }

    pub fn alerta(alerta: Alerta, valor: f64) -> Self {
        Self {
            gatilho: Gatilho::AlertaCarro,
            pedido: format!(
                "A telemetria do carro cruzou um limiar: {}. Leitura atual: {valor:.1}. \
                 Confira o resto da telemetria e diga, em um cartão de tom `alerta` e uma \
                 frase, o que fazer agora.",
                alerta.descricao()
            ),
        }
    }
}

/// Uma condição do carro que merece aviso.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Alerta {
    #[allow(dead_code)] // fora dos testes, só o `ECLIPSE_IA_GATILHO` constrói direto
    TemperaturaAlta,
    CombustivelBaixo,
    TensaoBaixa,
}

impl Alerta {
    const TODOS: [Alerta; 3] = [
        Self::TemperaturaAlta,
        Self::CombustivelBaixo,
        Self::TensaoBaixa,
    ];

    fn descricao(self) -> &'static str {
        match self {
            Self::TemperaturaAlta => "a temperatura do motor está subindo demais",
            Self::CombustivelBaixo => "o combustível está acabando",
            Self::TensaoBaixa => "a tensão da bateria está baixa",
        }
    }

    /// O valor deste alerta na leitura, se o PID já tiver voltado.
    fn valor(self, obd: &Value) -> Option<f64> {
        match self {
            Self::TemperaturaAlta => obd["coolantC"].as_f64(),
            // O `tanque.pct` estimado antes do `fuelPct` cru: é ele que casa
            // com a barra que o motorista vê, e ele existe mesmo em carro que
            // não responde o PID de nível. O cru fica de reserva.
            Self::CombustivelBaixo => obd["tanque"]["pct"]
                .as_f64()
                .or_else(|| obd["fuelPct"].as_f64()),
            Self::TensaoBaixa => obd["voltage"].as_f64(),
        }
    }

    /// Este alerta está ativo?
    ///
    /// Dois limiares: um para acender, outro mais folgado para apagar. Os
    /// limiares de acender são os mesmos de `src/core/telemetria.ts`, para o
    /// alerta escrito e o mostrador vermelho concordarem.
    ///
    /// Sem a folga, um valor tremendo em cima do limiar — que é exatamente o que
    /// acontece com temperatura de motor — acenderia e apagaria a cada leitura,
    /// e cada acender é uma chamada de API paga.
    fn ativo(self, valor: f64, ja_ativo: bool) -> bool {
        match (self, ja_ativo) {
            (Self::TemperaturaAlta, false) => valor > 105.0,
            (Self::TemperaturaAlta, true) => valor > 98.0,
            (Self::CombustivelBaixo, false) => valor < 15.0,
            (Self::CombustivelBaixo, true) => valor < 25.0,
            (Self::TensaoBaixa, false) => valor < 11.8,
            (Self::TensaoBaixa, true) => valor < 12.4,
        }
    }
}

/// Observa o barramento e aponta as transições que merecem uma fala.
#[derive(Default)]
pub struct Detector {
    destino: Option<String>,
    chegou: bool,
    alertas: HashSet<Alerta>,
}

impl Detector {
    pub fn novo() -> Self {
        Self::default()
    }

    /// Há rota ativa? É o que decide se o gatilho periódico faz sentido.
    pub fn em_viagem(&self) -> bool {
        self.destino.is_some() && !self.chegou
    }

    pub fn observar(&mut self, envelope: &StateEnvelope) -> Option<Acionamento> {
        match envelope.module.as_str() {
            "nav" => self.observar_nav(envelope.data.as_deref()?),
            "obd" => self.observar_obd(envelope.data.as_deref()?),
            _ => None,
        }
    }

    fn observar_nav(&mut self, nav: &Value) -> Option<Acionamento> {
        let destino = nav["rota"]["destino"].as_str().map(str::to_string);
        let chegou = nav["progresso"]["chegou"].as_bool().unwrap_or(false);

        // Destino novo (ou trocado) enquanto ainda não chegamos.
        if destino.is_some() && destino != self.destino {
            self.destino = destino.clone();
            self.chegou = false;
            return Some(Acionamento::rota(destino.as_deref().unwrap_or("o destino")));
        }

        // Rota cancelada: esquece, sem falar nada.
        if destino.is_none() && self.destino.is_some() {
            self.destino = None;
            self.chegou = false;
            return None;
        }

        // Chegar só vale uma vez por rota.
        if chegou && !self.chegou {
            self.chegou = true;
            return Some(Acionamento::chegada(
                self.destino.as_deref().unwrap_or("o destino"),
            ));
        }

        None
    }

    fn observar_obd(&mut self, obd: &Value) -> Option<Acionamento> {
        let mut acionamento = None;

        for alerta in Alerta::TODOS {
            // PID que ainda não voltou não acende nem apaga nada: sem leitura
            // não há o que afirmar, e apagar um alerta por falta de dado seria
            // dizer que o problema passou.
            let Some(valor) = alerta.valor(obd) else {
                continue;
            };

            let estava = self.alertas.contains(&alerta);
            let esta = alerta.ativo(valor, estava);

            match (estava, esta) {
                (false, true) => {
                    self.alertas.insert(alerta);
                    // Um acionamento por rodada. Se dois alertas acenderem
                    // juntos, o segundo dispara na leitura seguinte — e o modelo
                    // vê os dois de qualquer jeito, porque consulta a telemetria
                    // inteira antes de escrever.
                    acionamento.get_or_insert_with(|| Acionamento::alerta(alerta, valor));
                }
                (true, false) => {
                    self.alertas.remove(&alerta);
                }
                _ => {}
            }
        }

        acionamento
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eclipse_core::{ModuleId, Status};
    use serde_json::json;

    fn env(modulo: &'static str, data: Value) -> StateEnvelope {
        StateEnvelope {
            module: ModuleId::new(modulo),
            seq: 0,
            status: Status::Ready,
            data: Some(std::sync::Arc::new(data)),
            reason: None,
        }
    }

    fn nav(destino: Option<&str>, chegou: bool) -> StateEnvelope {
        env(
            "nav",
            json!({
                "rota": destino.map(|d| json!({ "destino": d })),
                "progresso": { "chegou": chegou },
            }),
        )
    }

    /// `fuel` entra pelo `tanque.pct`, que é a fonte que o detector prefere.
    fn obd(temp: Option<f64>, fuel: Option<f64>, volt: Option<f64>) -> StateEnvelope {
        env(
            "obd",
            json!({
                "coolantC": temp, "voltage": volt, "fuelPct": null,
                "tanque": { "pct": fuel }
            }),
        )
    }

    #[test]
    fn rota_nova_dispara_uma_vez_e_carrega_o_destino() {
        let mut d = Detector::novo();

        let a = d.observar(&nav(Some("Campos do Jordão"), false)).unwrap();
        assert_eq!(a.gatilho, Gatilho::RotaDefinida);
        assert!(a.pedido.contains("Campos do Jordão"));

        assert!(
            d.observar(&nav(Some("Campos do Jordão"), false)).is_none(),
            "a mesma rota não pode disparar de novo a cada posição"
        );
    }

    #[test]
    fn trocar_de_destino_dispara_de_novo() {
        let mut d = Detector::novo();
        d.observar(&nav(Some("Santos"), false));

        let a = d.observar(&nav(Some("Ubatuba"), false)).unwrap();
        assert_eq!(a.gatilho, Gatilho::RotaDefinida);
        assert!(a.pedido.contains("Ubatuba"));
    }

    #[test]
    fn chegar_dispara_uma_vez_so() {
        let mut d = Detector::novo();
        d.observar(&nav(Some("Santos"), false));

        let a = d.observar(&nav(Some("Santos"), true)).unwrap();
        assert_eq!(a.gatilho, Gatilho::Chegada);
        assert!(d.observar(&nav(Some("Santos"), true)).is_none());
    }

    #[test]
    fn cancelar_rota_nao_fala_nada() {
        let mut d = Detector::novo();
        d.observar(&nav(Some("Santos"), false));

        assert!(d.observar(&nav(None, false)).is_none());
        assert!(!d.em_viagem());
    }

    #[test]
    fn em_viagem_so_enquanto_ha_rota_e_nao_chegamos() {
        let mut d = Detector::novo();
        assert!(!d.em_viagem());

        d.observar(&nav(Some("Santos"), false));
        assert!(d.em_viagem());

        d.observar(&nav(Some("Santos"), true));
        assert!(!d.em_viagem());
    }

    #[test]
    fn temperatura_alta_dispara_ao_cruzar_o_limiar() {
        let mut d = Detector::novo();
        assert!(d.observar(&obd(Some(90.0), None, None)).is_none());

        let a = d.observar(&obd(Some(106.0), None, None)).unwrap();
        assert_eq!(a.gatilho, Gatilho::AlertaCarro);
        assert!(a.pedido.contains("temperatura"));
    }

    /// O caso que justifica a histerese inteira: o OBD publica ~3x por segundo,
    /// e temperatura de motor treme. Sem a folga de rearme, cada tremida em
    /// cima do limiar seria uma chamada de API paga.
    #[test]
    fn valor_tremendo_no_limiar_nao_dispara_a_cada_leitura() {
        let mut d = Detector::novo();
        assert!(d.observar(&obd(Some(106.0), None, None)).is_some());

        for temperatura in [104.0, 106.5, 103.0, 107.0, 99.5, 106.0] {
            assert!(
                d.observar(&obd(Some(temperatura), None, None)).is_none(),
                "disparou de novo a {temperatura}°C sem ter esfriado antes"
            );
        }
    }

    #[test]
    fn alerta_rearma_depois_de_normalizar_de_verdade() {
        let mut d = Detector::novo();
        d.observar(&obd(Some(106.0), None, None));

        // Abaixo do limiar de apagar: o alerta some.
        assert!(d.observar(&obd(Some(95.0), None, None)).is_none());
        // E agora pode acender de novo.
        assert!(d.observar(&obd(Some(106.0), None, None)).is_some());
    }

    /// Carro que não responde o PID de nível ainda tem a estimativa do tanque —
    /// e carro que responde tem as duas. O detector prefere a estimativa,
    /// porque é ela que casa com a barra da tela.
    #[test]
    fn combustivel_cai_para_o_pid_cru_quando_nao_ha_estimativa() {
        let mut d = Detector::novo();
        let so_cru = env("obd", json!({ "fuelPct": 12.0, "tanque": { "pct": null } }));
        assert!(d.observar(&so_cru).is_some());
    }

    #[test]
    fn combustivel_e_tensao_tambem_alertam() {
        let mut d = Detector::novo();
        assert!(d.observar(&obd(None, Some(12.0), None)).is_some());

        let mut d = Detector::novo();
        let a = d.observar(&obd(None, None, Some(11.2))).unwrap();
        assert!(a.pedido.contains("tensão"));
    }

    /// PID que ainda não voltou não é "o problema passou".
    #[test]
    fn pid_ausente_nao_apaga_alerta_aceso() {
        let mut d = Detector::novo();
        d.observar(&obd(Some(106.0), None, None));

        assert!(d.observar(&obd(None, None, None)).is_none());
        // Continua aceso: voltar a 106 não redispara.
        assert!(d.observar(&obd(Some(106.0), None, None)).is_none());
    }

    #[test]
    fn dois_alertas_juntos_saem_um_por_rodada() {
        let mut d = Detector::novo();
        let primeiro = d.observar(&obd(Some(120.0), Some(5.0), None));
        assert!(primeiro.is_some());

        // O segundo já está registrado como aceso, então não redispara.
        assert!(d.observar(&obd(Some(120.0), Some(5.0), None)).is_none());
    }

    #[test]
    fn gatilho_barato_usa_haiku_e_o_que_pesquisa_usa_opus() {
        assert_eq!(Gatilho::Ignicao.modelo(), Modelo::Haiku);
        assert_eq!(Gatilho::Periodico.modelo(), Modelo::Haiku);
        assert_eq!(Gatilho::RotaDefinida.modelo(), Modelo::Opus);
        assert_eq!(Gatilho::AlertaCarro.modelo(), Modelo::Opus);
    }

    #[test]
    fn modulo_que_nao_interessa_e_ignorado() {
        let mut d = Detector::novo();
        assert!(d.observar(&env("music", json!({ "nowPlaying": null }))).is_none());
    }
}
