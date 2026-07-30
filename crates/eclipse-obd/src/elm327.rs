//! O ELM327 de verdade.
//!
//! Duas coisas que costumam vir grudadas moram separadas aqui:
//!
//! - **O protocolo** — o handshake AT, qual comando cada [`Pid`] manda e como o
//!   hex de volta vira número. É determinístico e se testa sem carro.
//! - **O transporte** ([`Elm327Transport`]) — quem carrega os bytes até o
//!   adaptador. No Android é um socket Bluetooth (mora no `src-tauri`); nos
//!   testes é um dublê que devolve respostas de mentira.
//!
//! Essa separação é o que permite provar o parser contra respostas reais de
//! ELM327 num `cargo test` no laptop, sem depender de aparelho nem de carro.

use async_trait::async_trait;

use crate::pid::Pid;
use crate::source::{ObdError, ObdSource};

/// Carrega um comando até o adaptador e devolve a resposta crua — o texto entre
/// o comando e o prompt `>`, já sem o eco.
///
/// É só isto que o Bluetooth precisa saber fazer. O que mandar, quanto esperar
/// e como ler de volta é do protocolo, e o protocolo se testa com um transporte
/// de mentira. O `timeout_ms` vem do protocolo porque só ele sabe quais comandos
/// são lentos: a negociação de protocolo do carro leva vários segundos; um PID
/// já negociado, centenas de ms.
#[async_trait]
pub trait Elm327Transport: Send {
    async fn command(&mut self, cmd: &str, timeout_ms: u32) -> Result<String, ObdError>;
}

/// Teto para comandos comuns (AT do handshake, PIDs já negociados). Folgado: o
/// barramento ISO 9141-2 responde em centenas de ms; só estoura com o adaptador
/// mudo de verdade.
const TIMEOUT_COMANDO_MS: u32 = 5_000;

/// Teto para o `0100` de aquecimento, quando o adaptador ainda está descobrindo
/// o protocolo do carro (`SEARCHING...`). O slow init do ISO 9141-2 mais a busca
/// pelos outros protocolos pode passar fácil de 10 s — e interromper a busca
/// mandando outro comando faz ela recomeçar do zero.
const TIMEOUT_BUSCA_MS: u32 = 30_000;

/// Quantas vezes repetir o `0100` se a busca de protocolo ainda não terminou.
const TENTATIVAS_BUSCA: usize = 3;

/// Uma fonte OBD falando com um ELM327 de verdade por um transporte qualquer.
pub struct Elm327Source<T> {
    transport: T,
}

impl<T: Elm327Transport> Elm327Source<T> {
    /// Conecta e deixa o adaptador pronto.
    ///
    /// - `ATZ` reinicia (o comando mais lento; o adaptador volta do zero).
    /// - `ATE0` desliga o eco — senão cada resposta vem com o comando colado na
    ///   frente e o parser teria de adivinhar onde ele acaba.
    /// - `ATL0` tira os `\n`, `ATS0` tira os espaços: a resposta fica hex puro.
    /// - `ATSP0` deixa o adaptador descobrir sozinho o protocolo do carro.
    /// - `0100` de aquecimento: com `ATSP0`, a negociação com o carro só acontece
    ///   no primeiro pedido de modo 01 — e ela pode levar muitos segundos (slow
    ///   init do ISO 9141-2). Melhor pagar essa espera aqui, uma vez, com timeout
    ///   folgado, do que estourar na primeira leitura do painel e derrubar a
    ///   conexão inteira (o reconectar reseta o adaptador e a busca recomeçaria
    ///   do zero, para sempre).
    ///
    /// Se qualquer um não voltar (timeout/barramento), falha aqui: é melhor o
    /// supervisor reconectar do que entregar lixo ao painel.
    pub async fn conectar(mut transport: T) -> Result<Self, ObdError> {
        for cmd in ["ATZ", "ATE0", "ATL0", "ATS0", "ATSP0"] {
            // O conteúdo da resposta ao handshake não importa (versão de firmware,
            // "OK", eco do power-on); o que importa é o adaptador ter respondido
            // sem erro de transporte, que o `?` propaga.
            transport.command(cmd, TIMEOUT_COMANDO_MS).await?;
        }

        for _ in 0..TENTATIVAS_BUSCA {
            match transport.command("0100", TIMEOUT_BUSCA_MS).await {
                // Negociou: o carro respondeu o quadro do `0100` (`41 00 ...`).
                Ok(bruto) if hex_limpo(&bruto).contains("4100") => {
                    return Ok(Self { transport })
                }
                // `SEARCHING`/`STOPPED`/`NO DATA`/timeout: a busca não terminou
                // (ou foi interrompida) — vale insistir.
                Ok(bruto) => match classificar_erro(&bruto) {
                    ObdError::Timeout | ObdError::Unsupported => {}
                    e => return Err(e),
                },
                Err(ObdError::Timeout) => {}
                Err(e) => return Err(e),
            }
        }
        Err(ObdError::Timeout)
    }
}

/// O comando que lê cada grandeza.
///
/// Modo 01 (dados em tempo real) para os PIDs padrão; a voltagem é a da bateria
/// medida pelo próprio adaptador (`ATRV`), não um PID do carro.
fn comando_de(pid: Pid) -> &'static str {
    match pid {
        Pid::Rpm => "010C",
        Pid::Speed => "010D",
        Pid::Coolant => "0105",
        Pid::Fuel => "012F",
        Pid::Voltage => "ATRV",
    }
}

/// O começo da resposta positiva a um PID do modo 01: `0x41` mais o byte do PID.
/// Ex.: pedido `010C` → resposta começa em `410C`, e o que vem depois são os dados.
fn prefixo_resposta(pid: Pid) -> &'static str {
    match pid {
        Pid::Rpm => "410C",
        Pid::Speed => "410D",
        Pid::Coolant => "4105",
        Pid::Fuel => "412F",
        Pid::Voltage => "", // voltagem não é modo 01; ver `interpretar`
    }
}

#[async_trait]
impl<T: Elm327Transport> ObdSource for Elm327Source<T> {
    async fn read(&mut self, pid: Pid) -> Result<f32, ObdError> {
        let bruto = self
            .transport
            .command(comando_de(pid), TIMEOUT_COMANDO_MS)
            .await?;
        interpretar(pid, &bruto)
    }
}

/// Traduz a resposta crua do adaptador no número do PID.
///
/// A ordem importa: primeiro tenta achar o quadro de dados; só se não achar é que
/// classifica o erro. Isso é de propósito — o ELM327 costuma responder o primeiro
/// PID com `SEARCHING...` colado no quadro bom (`SEARCHING...410C1AF8`), e tratar
/// `SEARCHING` como erro cedo demais jogaria fora uma leitura válida.
fn interpretar(pid: Pid, bruto: &str) -> Result<f32, ObdError> {
    if pid == Pid::Voltage {
        return interpretar_voltagem(bruto);
    }

    let hex = hex_limpo(bruto);
    let Some(bytes) = bytes_apos(&hex, prefixo_resposta(pid)) else {
        // Sem quadro de dados: agora sim, que erro é este?
        return Err(classificar_erro(bruto));
    };

    let a = *bytes.first().ok_or(ObdError::Unsupported)? as f32;
    Ok(match pid {
        Pid::Rpm => {
            let b = *bytes.get(1).ok_or(ObdError::Unsupported)? as f32;
            (a * 256.0 + b) / 4.0
        }
        Pid::Speed => a,
        Pid::Coolant => a - 40.0,
        Pid::Fuel => a * 100.0 / 255.0,
        Pid::Voltage => unreachable!("voltagem tratada acima"),
    })
}

/// A voltagem vem como texto, tipo `12.5V` — pega o primeiro número.
fn interpretar_voltagem(bruto: &str) -> Result<f32, ObdError> {
    let numero: String = bruto
        .trim()
        .trim_start_matches(|c: char| !c.is_ascii_digit()) // pula lixo antes do número
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    numero
        .parse::<f32>()
        .map_err(|_| classificar_erro(bruto))
}

/// Só os dígitos hex da resposta, em maiúscula e sem espaços.
///
/// Só é chamado depois de a gente procurar por um quadro conhecido, então o lixo
/// que sobrevive ao filtro (letras hex de `SEARCHING`, por ex.) não atrapalha:
/// [`bytes_apos`] procura o prefixo exato do quadro, que o lixo não contém.
fn hex_limpo(bruto: &str) -> String {
    bruto
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_uppercase()
}

/// Os bytes que vêm logo depois do `prefixo` dentro do hex.
///
/// Acha a primeira ocorrência do prefixo (o começo do quadro de resposta) e lê os
/// pares hex seguintes. Devolve `None` se o prefixo não aparece ou se sobra hex
/// solto sem formar um byte inteiro.
fn bytes_apos(hex: &str, prefixo: &str) -> Option<Vec<u8>> {
    let inicio = hex.find(prefixo)? + prefixo.len();
    let dados = &hex[inicio..];
    if dados.is_empty() || !dados.len().is_multiple_of(2) {
        return None;
    }
    (0..dados.len() / 2)
        .map(|i| u8::from_str_radix(&dados[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// Que erro do ELM327 é este texto.
///
/// Só entra quando não há quadro de dados. `NO DATA`/`STOPPED` é o carro não
/// respondendo aquele PID (segue a vida sem ele); `SEARCHING` é o adaptador ainda
/// negociando o protocolo (vale tentar de novo); o resto é barramento — o que
/// derruba a varredura e faz o supervisor reconectar.
fn classificar_erro(bruto: &str) -> ObdError {
    let t = bruto.to_uppercase();
    if t.contains("NO DATA") || t.contains("STOPPED") {
        ObdError::Unsupported
    } else if t.contains("SEARCHING") {
        ObdError::Timeout
    } else {
        ObdError::Bus(bruto.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn rpm_do_exemplo_do_manual() {
        // 41 0C 1A F8 → ((0x1A*256)+0xF8)/4 = 1726 rpm.
        assert_eq!(interpretar(Pid::Rpm, "410C1AF8").unwrap(), 1726.0);
    }

    #[test]
    fn espacos_do_adaptador_nao_atrapalham() {
        // Se o `ATS0` não pegar, a resposta ainda vem com espaços — tem que ler igual.
        assert_eq!(interpretar(Pid::Rpm, "41 0C 1A F8").unwrap(), 1726.0);
    }

    #[test]
    fn velocidade_temperatura_e_combustivel() {
        assert_eq!(interpretar(Pid::Speed, "410D45").unwrap(), 69.0);
        assert_eq!(interpretar(Pid::Coolant, "41056E").unwrap(), 70.0); // 0x6E-40
        // 0x80 = 128 → 128*100/255 ≈ 50.196
        let fuel = interpretar(Pid::Fuel, "412F80").unwrap();
        assert!((fuel - 50.196).abs() < 0.01, "combustível {fuel}");
    }

    #[test]
    fn voltagem_vem_como_texto() {
        assert_eq!(interpretar(Pid::Voltage, "12.5V").unwrap(), 12.5);
        assert_eq!(interpretar(Pid::Voltage, "14.0V\r").unwrap(), 14.0);
    }

    #[test]
    fn searching_colado_no_quadro_bom_ainda_le() {
        // O clássico do primeiro PID: negociação colada no dado. Não pode perder o dado.
        assert_eq!(interpretar(Pid::Rpm, "SEARCHING...410C1AF8").unwrap(), 1726.0);
    }

    #[test]
    fn no_data_vira_unsupported() {
        // Carro velho não responde tudo — o Poller trata Unsupported como "pula".
        assert!(matches!(
            interpretar(Pid::Speed, "NO DATA"),
            Err(ObdError::Unsupported)
        ));
    }

    #[test]
    fn searching_sozinho_pede_pra_tentar_de_novo() {
        assert!(matches!(
            interpretar(Pid::Rpm, "SEARCHING..."),
            Err(ObdError::Timeout)
        ));
    }

    #[test]
    fn falha_de_barramento_e_bus() {
        assert!(matches!(
            interpretar(Pid::Rpm, "UNABLE TO CONNECT"),
            Err(ObdError::Bus(_))
        ));
        assert!(matches!(
            interpretar(Pid::Speed, "CAN ERROR"),
            Err(ObdError::Bus(_))
        ));
    }

    /// Um adaptador de mentira: responde cada comando com a fila de textos
    /// pré-combinada (uma resposta por chamada; a última se repete).
    struct FakeElm {
        respostas: HashMap<String, Vec<String>>,
        visto: Vec<(String, u32)>,
    }

    impl FakeElm {
        fn new<const N: usize>(respostas: [(&str, &[&str]); N]) -> Self {
            Self {
                respostas: respostas
                    .into_iter()
                    .map(|(cmd, rs)| {
                        (cmd.to_string(), rs.iter().map(|r| r.to_string()).collect())
                    })
                    .collect(),
                visto: Vec::new(),
            }
        }
    }

    #[async_trait]
    impl Elm327Transport for FakeElm {
        async fn command(&mut self, cmd: &str, timeout_ms: u32) -> Result<String, ObdError> {
            self.visto.push((cmd.to_string(), timeout_ms));
            Ok(match self.respostas.get_mut(cmd) {
                Some(fila) if fila.len() > 1 => fila.remove(0),
                Some(fila) => fila[0].clone(),
                None => "OK".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn conectar_faz_o_handshake_aquece_o_protocolo_e_le() {
        let fake = FakeElm::new([
            ("ATZ", &["ELM327 v1.5"] as &[&str]),
            // O clássico do primeiro pedido: a negociação colada no quadro bom.
            ("0100", &["SEARCHING...4100BE3EB811"]),
            ("010C", &["410C1AF8"]),
            ("010D", &["410D3C"]),
            ("ATRV", &["13.9V"]),
        ]);

        let mut fonte = Elm327Source::conectar(fake).await.expect("handshake");
        assert_eq!(fonte.read(Pid::Rpm).await.unwrap(), 1726.0);
        assert_eq!(fonte.read(Pid::Speed).await.unwrap(), 60.0);
        assert_eq!(fonte.read(Pid::Voltage).await.unwrap(), 13.9);

        let comandos: Vec<&str> = fonte
            .transport
            .visto
            .iter()
            .map(|(c, _)| c.as_str())
            .collect();
        // O eco tem que ser desligado, senão o parser lê o comando de volta como dado.
        assert!(comandos.contains(&"ATE0"));
        // O aquecimento vem depois do ATSP0 e antes de qualquer PID do painel.
        let pos = |c| comandos.iter().position(|x| *x == c).unwrap();
        assert!(pos("ATSP0") < pos("0100") && pos("0100") < pos("010C"));

        // A busca de protocolo pode levar >10 s; o 0100 tem que esperar bem mais
        // que um PID comum, senão volta o bug do estouro na primeira leitura.
        let timeout_do_0100 = fonte
            .transport
            .visto
            .iter()
            .find(|(c, _)| c == "0100")
            .unwrap()
            .1;
        assert!(timeout_do_0100 >= 30_000, "timeout {timeout_do_0100}");
    }

    #[tokio::test]
    async fn busca_lenta_insiste_no_0100_ate_negociar() {
        // Primeira tentativa devolve só SEARCHING (estourou o teto no meio da
        // busca); a segunda vem com o quadro. Não pode desistir na primeira.
        let fake = FakeElm::new([("0100", &["SEARCHING...", "4100BE3EB811"] as &[&str])]);
        let fonte = Elm327Source::conectar(fake).await;
        assert!(fonte.is_ok(), "tinha que negociar na segunda tentativa");
    }

    #[tokio::test]
    async fn carro_desligado_falha_o_conectar() {
        // UNABLE TO CONNECT = ignição desligada/sem carro: falha firme, o
        // supervisor fica reconectando com backoff até o carro ligar.
        let fake = FakeElm::new([("0100", &["UNABLE TO CONNECT"] as &[&str])]);
        assert!(matches!(
            Elm327Source::conectar(fake).await,
            Err(ObdError::Bus(_))
        ));
    }

    #[tokio::test]
    async fn handshake_falho_nao_vira_fonte() {
        struct Morto;
        #[async_trait]
        impl Elm327Transport for Morto {
            async fn command(&mut self, _cmd: &str, _t: u32) -> Result<String, ObdError> {
                Err(ObdError::Timeout)
            }
        }
        assert!(Elm327Source::conectar(Morto).await.is_err());
    }
}
