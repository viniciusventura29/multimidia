//! O que este carro responde.
//!
//! O modo 01 tem uma máscara de suportados: `0100` devolve quatro bytes em que cada
//! bit diz se um PID de `01` a `20` existe, `0120` repete para `21` a `40` e `0140`
//! para `41` a `60`. O handshake já pedia `0100` para aquecer o protocolo e jogava
//! esses bytes fora — agora eles viram a lista de perguntas que vale a pena fazer,
//! num barramento em que cada pergunta custa 300 ms.
//!
//! A assimetria importante deste arquivo: **presença é fato, ausência é suspeita.**
//! Um `NO DATA` no primeiro segundo depois de ligar o carro pode ser a ECU ainda
//! acordando. Por isso "respondeu" pode ser guardado em disco e "não respondeu" não,
//! e por isso um bloco que nunca foi lido não vale como negativa.

use serde::{Serialize, Serializer};

/// Até onde a máscara vai: `01`–`60`, os três primeiros blocos.
///
/// Vai até o terceiro porque a vazão de combustível (`5E`) mora lá — sem ler esse
/// bloco não haveria como saber se o carro dispensa a estimativa de consumo.
const COBERTURA: u8 = 0x60;

/// A máscara de PIDs suportados do modo 01.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacidades {
    /// Bit `n-1` ligado = PID `n` existe.
    mascara: u128,
    /// Quais blocos de 32 PIDs já foram lidos (bit 0 = `01`–`20`, e assim por diante).
    ///
    /// Sem isto, ler só o primeiro bloco faria todo PID dos blocos seguintes parecer
    /// inexistente — e o nível de combustível (`2F`) e a vazão (`5E`) vivem lá.
    blocos: u8,
    /// Quem o carro se recusou a responder na prática, apesar da máscara.
    ///
    /// Mora só na memória, nunca em disco: um `NO DATA` transitório não deve
    /// condenar um sensor para sempre.
    recusados: u128,
}

impl Capacidades {
    /// O estado antes de qualquer resposta: tudo pode existir.
    pub fn otimista() -> Self {
        Self::default()
    }

    /// Alguma máscara já foi lida?
    pub fn descoberto(self) -> bool {
        self.blocos != 0
    }

    /// Vale a pena perguntar por este PID?
    pub fn suporta(self, pid: u8) -> bool {
        if pid == 0 {
            return false;
        }
        if pid <= COBERTURA && self.recusados & bit(pid) != 0 {
            return false;
        }
        match bloco_de(pid) {
            // Bloco lido: a máscara manda. Quem não anunciou e responder mesmo
            // assim entra depois por `marcar` — a prática vence a máscara.
            Some(b) if self.blocos & (1 << b) != 0 => self.mascara & bit(pid) != 0,
            // Bloco nunca lido: ninguém sabe, então pergunta.
            _ => true,
        }
    }

    /// Registra que um PID respondeu de verdade, mesmo que a máscara não o listasse.
    pub fn marcar(&mut self, pid: u8) {
        if pid > 0 && pid <= COBERTURA {
            self.mascara |= bit(pid);
            self.recusados &= !bit(pid);
        }
    }

    /// Registra que o carro não respondeu este PID na prática.
    pub fn recusar(&mut self, pid: u8) {
        if pid > 0 && pid <= COBERTURA {
            self.recusados |= bit(pid);
            self.mascara &= !bit(pid);
        }
    }

    /// Junta um bloco de máscara ao que já se sabe.
    ///
    /// `base` é o PID do pedido (`0x00` para `0100`, `0x20` para `0120`, `0x40` para
    /// `0140`) e `bytes` são os quatro bytes da resposta. Do primeiro byte, o bit mais
    /// significativo é `base+1`; do quarto, o menos significativo é `base+32`. Bloco
    /// curto é lido até onde deu — meia resposta ainda é melhor que nenhuma.
    pub fn juntar(&mut self, base: u8, bytes: &[u8]) {
        if let Some(b) = bloco_de(base + 1) {
            self.blocos |= 1 << b;
        }
        for (i, byte) in bytes.iter().take(4).enumerate() {
            for b in 0..8 {
                if byte & (0x80 >> b) != 0 {
                    self.marcar(base + (i as u8) * 8 + b + 1);
                }
            }
        }
    }

    /// Os PIDs suportados, em hex de dois dígitos — o que a tela do carro mostra.
    pub fn lista(self) -> Vec<String> {
        (1..=COBERTURA)
            .filter(|pid| self.mascara & bit(*pid) != 0)
            .map(|pid| format!("{pid:02X}"))
            .collect()
    }
}

fn bit(pid: u8) -> u128 {
    1u128 << (pid - 1)
}

/// Em qual bloco de 32 PIDs este PID cai, se estiver dentro da cobertura.
fn bloco_de(pid: u8) -> Option<u8> {
    (pid >= 1 && pid <= COBERTURA).then(|| (pid - 1) / 32)
}

/// A UI não quer uma máscara, quer saber o que o carro tem — e se já foi descoberto.
impl Serialize for Capacidades {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Fora {
            pids: Vec<String>,
            descoberto: bool,
        }

        Fora {
            pids: self.lista(),
            descoberto: self.descoberto(),
        }
        .serialize(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::Pid;

    /// `BE 3E B8 11` é a resposta que o carro dá no teste do handshake.
    #[test]
    fn le_a_mascara_bit_por_bit() {
        let mut c = Capacidades::otimista();
        c.juntar(0x00, &[0xBE, 0x3E, 0xB8, 0x11]);

        // 0xBE = 1011_1110 → PIDs 01, 03, 04, 05, 06, 07 (não o 02, não o 08).
        assert!(c.suporta(0x01), "o bit mais significativo é o PID 01");
        assert!(!c.suporta(0x02));
        assert!(c.suporta(0x04), "carga calculada");
        assert!(c.suporta(0x05), "temperatura");
        // 0x3E = 0011_1110 → PIDs 0B, 0C, 0D, 0E, 0F (não 09, não 0A, não 10).
        assert!(c.suporta(0x0B), "MAP");
        assert!(c.suporta(0x0C), "RPM");
        assert!(c.suporta(0x0D), "velocidade");
        assert!(c.suporta(0x0F), "temperatura do ar");
        assert!(!c.suporta(0x10), "este carro não anuncia MAF");
        // 0x11 = 0001_0001 → PIDs 1C e 20.
        assert!(c.suporta(0x1C));
        assert!(c.suporta(0x20));
    }

    #[test]
    fn o_segundo_bloco_e_onde_mora_o_nivel_de_combustivel() {
        let mut c = Capacidades::otimista();
        // Só o bit do PID 2F: é o 15º de 21..40, logo o bit 0x02 do segundo byte.
        c.juntar(0x20, &[0x00, 0x02, 0x00, 0x00]);

        assert!(c.suporta(Pid::Fuel.codigo().unwrap()));
        assert!(!c.suporta(0x2E));
    }

    #[test]
    fn sem_mascara_lida_tenta_tudo() {
        // O carro velho que responde 0100 sem máscara utilizável não pode virar um
        // painel apagado: pergunta-se tudo e o NO DATA vai desmentindo.
        let c = Capacidades::otimista();
        assert!(c.suporta(0x10));
        assert!(c.suporta(0x2F));
        assert!(!c.descoberto());
    }

    #[test]
    fn bloco_lido_e_definitivo_bloco_nao_lido_nao_e() {
        let mut c = Capacidades::otimista();
        c.juntar(0x00, &[0x00, 0x00, 0x00, 0x00]);

        assert!(c.descoberto());
        assert!(!c.suporta(0x10), "a máscara do primeiro bloco veio vazia");
        assert!(
            c.suporta(0x5E),
            "o terceiro bloco nunca foi lido — não vale como negativa"
        );
    }

    #[test]
    fn quem_responde_tem_mesmo_sem_anunciar() {
        let mut c = Capacidades::otimista();
        c.juntar(0x00, &[0x00, 0x00, 0x00, 0x00]);
        assert!(!c.suporta(0x10));

        c.marcar(0x10);
        assert!(c.suporta(0x10), "respondeu; logo tem, apesar da máscara");
    }

    #[test]
    fn quem_se_recusa_sai_da_roda_mesmo_estando_na_mascara() {
        let mut c = Capacidades::otimista();
        c.juntar(0x20, &[0x00, 0x02, 0x00, 0x00]);
        assert!(c.suporta(0x2F));

        // O caso do Eclipse: a máscara anuncia o nível de combustível e a ECU
        // responde NO DATA. Insistir custaria uma leitura de RPM a cada ciclo.
        c.recusar(0x2F);
        assert!(!c.suporta(0x2F));
    }

    #[test]
    fn bloco_curto_aproveita_o_que_veio() {
        let mut c = Capacidades::otimista();
        c.juntar(0x00, &[0xBE]);
        assert!(c.suporta(0x01));
        assert!(!c.suporta(0x0C), "o byte do RPM não chegou");
    }

    #[test]
    fn a_lista_sai_em_hex_para_a_tela() {
        let mut c = Capacidades::otimista();
        c.juntar(0x00, &[0x08, 0x00, 0x00, 0x00]);
        assert_eq!(c.lista(), vec!["05"]);
    }
}
