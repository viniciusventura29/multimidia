//! O diário de bordo: o que aconteceu no carro, legível daqui de casa.
//!
//! O painel roda numa head unit dentro de um carro. Quando algo quebra lá, não
//! há console, não há `adb` e não há quem olhe — o dono vê uma tela estranha e
//! segue dirigindo. Todo bug do Eclipse até aqui foi descoberto por dedução
//! sobre um sintoma contado de memória horas depois.
//!
//! Este módulo troca isso por um arquivo de texto. Três decisões o definem:
//!
//! - **O buffer é em disco, não em memória.** A ignição corta energia sem
//!   avisar, e um buffer em RAM perde exatamente o log mais interessante: o do
//!   instante em que desligou.
//! - **Só o que dói sobe.** `warn` e `error` sempre; o `info` em volta vai junto
//!   como rastro, pelo mesmo motivo que um extrato só faz sentido com as linhas
//!   vizinhas. Mandar tudo seria inundar — o poller do OBD sozinho fala três
//!   vezes por segundo.
//! - **Envia em lote e tolera ficar sem rede.** Wi-Fi na garagem, hotspot na
//!   estrada, nada no túnel. Um logger que faz POST por linha perde tudo assim
//!   que cai — e cai justamente quando algo está errado.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// O arquivo de saída, no diretório de dados do app.
const CADERNO: &str = "diario.jsonl";

/// Quantas linhas de contexto acompanham cada erro.
///
/// Cinquenta é o que cabe na tela quando eu for ler, e cobre com folga a
/// sequência que interessa: o supervisor derrubando um módulo, as tentativas de
/// reconexão e o erro final. Mais que isso vira ruído em volta do sinal.
const RASTRO: usize = 50;

/// Teto do arquivo. Passou disso, a metade mais velha é descartada.
///
/// Meio mega de texto são uns poucos milhares de linhas — muito mais do que
/// qualquer viagem produz, e pequeno demais para incomodar os 128 GB da central.
/// O teto existe para o caso ruim: um laço de erro sem rede, gravando por
/// semanas. É melhor perder o começo de um erro repetido do que encher o disco
/// do carro.
const TETO_BYTES: u64 = 512 * 1024;

/// De quanto em quanto tempo se tenta subir o que está acumulado.
///
/// Trinta segundos: rápido o bastante para eu ver um erro quase ao vivo com o
/// carro parado na garagem, devagar o bastante para não gastar bateria nem dados
/// perguntando à toa. Sem nada acumulado, não sai requisição nenhuma.
const INTERVALO_ENVIO: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Nivel {
    Debug,
    Info,
    Aviso,
    Erro,
}

impl From<&tracing::Level> for Nivel {
    fn from(l: &tracing::Level) -> Self {
        match *l {
            tracing::Level::ERROR => Nivel::Erro,
            tracing::Level::WARN => Nivel::Aviso,
            tracing::Level::INFO => Nivel::Info,
            _ => Nivel::Debug,
        }
    }
}

/// Uma linha do diário.
///
/// `onde` é o módulo, não arquivo:linha. Módulo é a unidade que degrada e
/// reinicia no supervisor — é por ele que se pergunta "o que o OBD estava
/// fazendo?" — e sobrevive a um refactor que arquivo:linha não sobrevive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Linha {
    pub ts: String,
    pub nivel: Nivel,
    pub onde: String,
    pub msg: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub dados: Map<String, Value>,
}

impl Linha {
    pub fn nova(nivel: Nivel, onde: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            nivel,
            onde: onde.into(),
            msg: msg.into(),
            dados: Map::new(),
        }
    }
}

/// O caderno: o rastro recente em memória e a fila de envio em disco.
pub struct Diario {
    rastro: Mutex<VecDeque<Linha>>,
    caderno: PathBuf,
    /// Serializa as gravações E os envios: quem esvazia o arquivo não pode
    /// esvaziá-lo enquanto outra thread escreve nele.
    escrita: Mutex<()>,
}

impl Diario {
    pub fn novo(dir: &Path) -> Self {
        Self {
            rastro: Mutex::new(VecDeque::with_capacity(RASTRO)),
            caderno: dir.join(CADERNO),
            escrita: Mutex::new(()),
        }
    }

    /// Só os testes perguntam onde o caderno está — em produção quem sabe é o
    /// próprio `Diario`. `cfg(test)` em vez de `allow(dead_code)`: um acessório
    /// que some do binário é melhor que um aviso silenciado.
    #[cfg(test)]
    pub fn caderno(&self) -> &Path {
        &self.caderno
    }

    /// Uma linha que sobe MESMO sem erro nenhum.
    ///
    /// Existe porque o diário tinha um ponto cego grande: sessão limpa não
    /// mandava nada, e "o carro rodou e estava tudo bem" ficava idêntico a "o
    /// carro não rodou". Para quem lê de fora, silêncio não é resposta.
    ///
    /// Use com parcimônia — é o único caminho que ignora o nível, e todo marco
    /// custa uma requisição de um carro que às vezes está em hotspot.
    pub fn marco(&self, linha: Linha) {
        self.gravar(&[linha]);
        self.podar();
    }

    /// Anota uma linha. `info` e `debug` só viram rastro; `aviso` e `erro` vão
    /// para o disco levando o rastro junto.
    pub fn anotar(&self, linha: Linha) {
        if linha.nivel < Nivel::Aviso {
            let mut rastro = self.rastro.lock().unwrap();
            if rastro.len() == RASTRO {
                rastro.pop_front();
            }
            rastro.push_back(linha);
            return;
        }

        // O rastro é DRENADO, não copiado: se dois erros vierem em sequência, o
        // segundo não repete o contexto que o primeiro já levou.
        let contexto: Vec<Linha> = {
            let mut rastro = self.rastro.lock().unwrap();
            rastro.drain(..).collect()
        };

        let mut linhas = contexto;
        linhas.push(linha);
        self.gravar(&linhas);
        self.podar();
    }

    /// Escreve no fim do caderno. Falha em silêncio de propósito.
    fn gravar(&self, linhas: &[Linha]) {
        let _guarda = self.escrita.lock().unwrap();
        let mut texto = String::new();
        for l in linhas {
            if let Ok(json) = serde_json::to_string(&redigir(l.clone())) {
                texto.push_str(&json);
                texto.push('\n');
            }
        }

        if let Some(pai) = self.caderno.parent() {
            let _ = fs::create_dir_all(pai);
        }
        // Falha ao gravar log não pode derrubar nada nem virar outro log (que
        // gravaria de novo, e de novo). Some em silêncio, de propósito.
        if let Ok(mut arquivo) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.caderno)
        {
            let _ = arquivo.write_all(texto.as_bytes());
        }
    }

    /// Corta a metade mais velha quando o arquivo passa do teto.
    fn podar(&self) {
        let Ok(meta) = fs::metadata(&self.caderno) else {
            return;
        };
        if meta.len() <= TETO_BYTES {
            return;
        }
        let Ok(texto) = fs::read_to_string(&self.caderno) else {
            return;
        };
        let linhas: Vec<&str> = texto.lines().collect();
        let resto = linhas[linhas.len() / 2..].join("\n");
        let _ = fs::write(&self.caderno, format!("{resto}\n"));
    }

    /// Tira do caderno tudo que está lá, para enviar.
    ///
    /// Devolve as linhas **e** o tamanho lido. O tamanho é o que permite apagar
    /// só o que foi enviado: entre ler e confirmar, o carro pode ter escrito
    /// mais, e truncar o arquivo inteiro perderia justamente o erro novo.
    fn recolher(&self) -> Option<(Vec<Linha>, u64)> {
        let _guarda = self.escrita.lock().unwrap();
        let texto = fs::read_to_string(&self.caderno).ok()?;
        if texto.trim().is_empty() {
            return None;
        }
        let linhas: Vec<Linha> = texto
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if linhas.is_empty() {
            // Só lixo: apaga, senão trava a fila para sempre.
            let _ = fs::write(&self.caderno, "");
            return None;
        }
        Some((linhas, texto.len() as u64))
    }

    /// Confirma o envio, removendo só os bytes que foram lidos.
    fn confirmar(&self, enviados: u64) {
        let _guarda = self.escrita.lock().unwrap();
        let Ok(texto) = fs::read_to_string(&self.caderno) else {
            return;
        };
        let resto = texto.get(enviados as usize..).unwrap_or("");
        let _ = fs::write(&self.caderno, resto);
    }
}

/// Tira do caminho o que não deve sair do carro.
///
/// Coordenada vira duas casas decimais — cerca de um quilômetro, que é o que
/// preciso para saber "estava na estrada" ou "estava parado em casa" e não é o
/// suficiente para achar a casa. E qualquer campo que cheire a credencial vira
/// `<omitido>`: nenhum deveria ser logado, mas a hora de descobrir que um foi
/// não é depois de ele ter atravessado a internet.
fn redigir(mut linha: Linha) -> Linha {
    for (chave, valor) in linha.dados.iter_mut() {
        let c = chave.to_lowercase();
        if c.contains("token") || c.contains("chave") || c.contains("key") || c.contains("secret") {
            *valor = Value::String("<omitido>".to_string());
            continue;
        }
        if c == "lat" || c == "lon" || c == "latitude" || c == "longitude" {
            if let Some(n) = valor.as_f64() {
                *valor = serde_json::json!((n * 100.0).round() / 100.0);
            }
        }
    }
    linha
}

/// O que sobe de uma vez.
#[derive(Debug, Serialize)]
struct Lote<'a> {
    sessao: &'a str,
    versao: &'a str,
    linhas: &'a [Linha],
}

/// Sobe o que estiver acumulado, de tempos em tempos.
///
/// Não guarda estado de "tentativa": se falhar, o caderno continua intacto e a
/// próxima rodada leva tudo. É o backoff mais simples que existe e o certo aqui,
/// porque a causa quase sempre é a mesma — o carro está sem rede.
pub async fn enviar_periodicamente(
    diario: Arc<Diario>,
    destino: String,
    chave: String,
    sessao: String,
) {
    let versao = versao();
    let http = reqwest::Client::new();
    let mut relogio = tokio::time::interval(INTERVALO_ENVIO);

    loop {
        relogio.tick().await;

        let Some((linhas, lidos)) = diario.recolher() else {
            continue;
        };

        let corpo = Lote {
            sessao: &sessao,
            versao: &versao,
            linhas: &linhas,
        };

        match http
            .post(&destino)
            .header("x-eclipse-chave", &chave)
            .json(&corpo)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => diario.confirmar(lidos),
            // Sem `tracing::warn!` aqui: ele voltaria para o diário, que tentaria
            // enviar, que falharia de novo. Um laço que se alimenta do próprio
            // erro é pior que ficar calado.
            Ok(r) => eprintln!("[diario] a central recusou o lote: {}", r.status()),
            Err(e) => eprintln!("[diario] sem rede para enviar o lote: {e}"),
        }
    }
}

/// A versão que está rodando. Sem ela não dá para saber se um erro que voltou é
/// o velho ou um novo — metade dos bugs some com o APK seguinte.
fn versao() -> String {
    match option_env!("ECLIPSE_VERSION_CODE") {
        Some(code) if !code.trim().is_empty() => {
            format!("{}+{}", env!("CARGO_PKG_VERSION"), code.trim())
        }
        _ => format!("{}+dev", env!("CARGO_PKG_VERSION")),
    }
}

/// O diário vivo do app.
///
/// Um `static` porque o `tracing` instala o subscriber no começo do `run()`,
/// antes de existir `AppHandle` — e é só com ele que se descobre o diretório de
/// dados no Android. A camada é registrada de saída e fica calada até o `setup()`
/// entregar o caderno aqui. O que se perde nesse meio é o log da própria
/// inicialização do logger, que não interessa a ninguém.
static DIARIO: OnceLock<Arc<Diario>> = OnceLock::new();

pub fn instalar(diario: Arc<Diario>) {
    let _ = DIARIO.set(diario);
}

pub fn atual() -> Option<&'static Arc<Diario>> {
    DIARIO.get()
}

/// Liga o `tracing` ao caderno.
///
/// Vive ao lado da camada que imprime no console, e não no lugar dela: no
/// `tauri dev` eu quero ver no terminal, e no carro quero no arquivo. São dois
/// destinos do mesmo evento, não dois logs diferentes.
pub struct CamadaDiario;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CamadaDiario {
    fn on_event(
        &self,
        evento: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let Some(diario) = atual() else {
            return;
        };

        let meta = evento.metadata();
        let mut coletor = Coletor::default();
        evento.record(&mut coletor);

        let mut linha = Linha::nova(Nivel::from(meta.level()), onde(meta.target()), coletor.msg);
        linha.dados = coletor.dados;
        diario.anotar(linha);
    }
}

/// `eclipse_os_lib::modules::obd` vira `obd`.
///
/// O caminho inteiro do crate não ajuda a ler — o que eu pergunto é "o que o OBD
/// estava fazendo", e o último segmento é exatamente isso.
fn onde(target: &str) -> &str {
    target.rsplit("::").next().unwrap_or(target)
}

/// Junta a mensagem e os campos de um evento do `tracing`.
#[derive(Default)]
struct Coletor {
    msg: String,
    dados: Map<String, Value>,
}

impl Coletor {
    fn guardar(&mut self, campo: &tracing::field::Field, valor: Value) {
        if campo.name() == "message" {
            self.msg = valor
                .as_str()
                .map(str::to_string)
                .unwrap_or(valor.to_string());
        } else {
            self.dados.insert(campo.name().to_string(), valor);
        }
    }
}

impl tracing::field::Visit for Coletor {
    fn record_debug(&mut self, campo: &tracing::field::Field, valor: &dyn std::fmt::Debug) {
        self.guardar(campo, Value::String(format!("{valor:?}")));
    }
    fn record_str(&mut self, campo: &tracing::field::Field, valor: &str) {
        self.guardar(campo, Value::String(valor.to_string()));
    }
    fn record_i64(&mut self, campo: &tracing::field::Field, valor: i64) {
        self.guardar(campo, serde_json::json!(valor));
    }
    fn record_u64(&mut self, campo: &tracing::field::Field, valor: u64) {
        self.guardar(campo, serde_json::json!(valor));
    }
    fn record_f64(&mut self, campo: &tracing::field::Field, valor: f64) {
        self.guardar(campo, serde_json::json!(valor));
    }
    fn record_bool(&mut self, campo: &tracing::field::Field, valor: bool) {
        self.guardar(campo, Value::Bool(valor));
    }
    fn record_error(
        &mut self,
        campo: &tracing::field::Field,
        valor: &(dyn std::error::Error + 'static),
    ) {
        self.guardar(campo, Value::String(valor.to_string()));
    }
}

/// Faz um pânico do Rust virar linha de diário antes de morrer.
///
/// Um pânico numa task de módulo hoje some: o supervisor reinicia e a mensagem
/// fica só no logcat, que ninguém lê. O hook anterior é chamado depois, para o
/// comportamento de sempre (abortar, imprimir) continuar igual.
pub fn capturar_panicos() {
    let anterior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(diario) = atual() {
            let mut linha = Linha::nova(Nivel::Erro, "panico", info.to_string());
            if let Some(local) = info.location() {
                linha
                    .dados
                    .insert("onde".into(), Value::String(local.to_string()));
            }
            diario.anotar(linha);
        }
        anterior(info);
    }));
}

/// A ponte do WebView.
///
/// A tela branca do React não passa por `tracing` nenhum: ela morre em JavaScript
/// e o Rust nunca fica sabendo. Este comando é por onde o `window.onerror` e o
/// `unhandledrejection` do painel entram no mesmo caderno, com o mesmo formato.
#[tauri::command]
pub fn anotar_do_painel(
    nivel: Nivel,
    onde: String,
    msg: String,
    dados: Option<Map<String, Value>>,
) {
    let Some(diario) = atual() else {
        return;
    };
    let mut linha = Linha::nova(nivel, onde, msg);
    linha.dados = dados.unwrap_or_default();
    diario.anotar(linha);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eclipse-diario-{nome}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn info_sozinho_nao_gasta_disco() {
        let d = Diario::novo(&temp("so-info"));
        for i in 0..200 {
            d.anotar(Linha::nova(Nivel::Info, "obd", format!("leitura {i}")));
        }
        assert!(
            !d.caderno().exists(),
            "sem nenhum erro, nada precisa subir — o poller sozinho falaria 3x por segundo"
        );
    }

    #[test]
    fn um_marco_sobe_mesmo_sem_erro_nenhum() {
        let d = Diario::novo(&temp("marco"));
        for i in 0..20 {
            d.anotar(Linha::nova(Nivel::Info, "obd", format!("leitura {i}")));
        }
        d.marco(Linha::nova(Nivel::Info, "sessao", "o carro ligou"));

        let (linhas, _) = d
            .recolher()
            .expect("sessão limpa também precisa chegar: silêncio não é resposta");
        assert_eq!(
            linhas.len(),
            1,
            "o marco sobe sozinho, sem arrastar o rastro"
        );
        assert_eq!(linhas[0].msg, "o carro ligou");
    }

    #[test]
    fn um_erro_leva_o_contexto_junto() {
        let d = Diario::novo(&temp("contexto"));
        for i in 0..5 {
            d.anotar(Linha::nova(Nivel::Info, "obd", format!("tentativa {i}")));
        }
        d.anotar(Linha::nova(Nivel::Erro, "obd", "o adaptador soltou"));

        let (linhas, _) = d.recolher().expect("tem que ter o que enviar");
        assert_eq!(linhas.len(), 6, "as 5 de contexto mais o erro");
        assert_eq!(linhas[0].msg, "tentativa 0");
        assert_eq!(linhas[5].nivel, Nivel::Erro);
    }

    #[test]
    fn o_contexto_nao_se_repete_no_erro_seguinte() {
        let d = Diario::novo(&temp("sem-repetir"));
        d.anotar(Linha::nova(Nivel::Info, "nav", "procurando rota"));
        d.anotar(Linha::nova(Nivel::Erro, "nav", "primeira falha"));
        d.anotar(Linha::nova(Nivel::Erro, "nav", "segunda falha"));

        let (linhas, _) = d.recolher().unwrap();
        let contextos = linhas.iter().filter(|l| l.msg == "procurando rota").count();
        assert_eq!(contextos, 1, "o rastro é drenado, não copiado");
    }

    #[test]
    fn so_o_que_foi_enviado_some() {
        let d = Diario::novo(&temp("confirmar"));
        d.anotar(Linha::nova(Nivel::Erro, "obd", "erro velho"));
        let (_, lidos) = d.recolher().unwrap();

        // Chegou um erro novo enquanto o lote estava no ar.
        d.anotar(Linha::nova(Nivel::Erro, "obd", "erro novo"));
        d.confirmar(lidos);

        let (linhas, _) = d.recolher().expect("o erro novo tem que sobreviver");
        assert_eq!(linhas.len(), 1);
        assert_eq!(linhas[0].msg, "erro novo");
    }

    #[test]
    fn coordenada_sai_arredondada_e_credencial_nao_sai() {
        let mut l = Linha::nova(Nivel::Erro, "nav", "falhou");
        l.dados.insert("lat".into(), serde_json::json!(-23.5617384));
        l.dados.insert(
            "accessToken".into(),
            serde_json::json!("segredo-de-verdade"),
        );

        let limpa = redigir(l);
        assert_eq!(limpa.dados["lat"], serde_json::json!(-23.56), "~1 km");
        assert_eq!(limpa.dados["accessToken"], "<omitido>");
    }

    #[test]
    fn o_caderno_nao_cresce_para_sempre() {
        let d = Diario::novo(&temp("teto"));
        let gordo = "x".repeat(2000);
        for _ in 0..400 {
            d.anotar(Linha::nova(Nivel::Erro, "obd", gordo.clone()));
        }
        let tamanho = fs::metadata(d.caderno()).unwrap().len();
        assert!(
            tamanho <= TETO_BYTES,
            "um laço de erro sem rede não pode encher o disco do carro: {tamanho} bytes"
        );
    }
}
