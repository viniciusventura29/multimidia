//! A versão nova, e o toque que leva até ela.
//!
//! A head unit não tem loja de aplicativos nem máquina de desenvolvimento por
//! perto: a única forma de o carro receber código novo é alguém baixar um APK
//! de uma URL. Isso só funciona se alguém LEMBRAR de baixar — e é esse o
//! trabalho daqui. Perguntar de vez em quando se o release rolante já passou da
//! versão instalada e, quando passou, acender um alvo do tamanho de um polegar
//! no cabeçalho do painel.
//!
//! POR QUE COMANDO, E NÃO MÓDULO NO BARRAMENTO. O padrão da casa é módulo que
//! publica estado, e ele é o certo para o que muda sozinho e tem vários
//! leitores — o `obd` tica a cada 0,9 s e alimenta seis lugares. Aqui muda
//! quatro vezes por dia, tem um leitor só, e não há socket para cair nem task
//! para entrar em pânico: nada que o `Supervisor` tenha o que supervisionar.
//! Pior: o envelope de módulo carrega `status: degraded` + `reason`, e o `Tile`
//! MOSTRA isso na tela — enquanto a regra aqui é que carro sem rede não vê
//! aviso nenhum. Seria usar uma máquina cujo comportamento padrão é exatamente
//! o que preciso impedir. O molde certo é o `spotify_access_token`: valor
//! pontual, buscado na rede, devolvido por retorno.
//!
//! POR QUE NO RUST, E NÃO UM `fetch` NO WEBVIEW. Três motivos duros: o
//! `versionCode` local só existe aqui, embutido em tempo de compilação; HTTP a
//! partir do JS exigiria instalar o `tauri-plugin-http` e abrir permissão nova
//! em `capabilities/default.json`, que hoje só tem `core:default` e
//! `opener:default`; e o `reqwest` já está na árvore com `rustls-tls`.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

/// O manifesto que a CI publica ao lado do APK, no release rolante de tag
/// `apk`. Pública, sem login, e a URL não muda nunca — ver `.github/workflows/apk.yml`.
const MANIFESTO: &str =
    "https://github.com/viniciusventura29/multimidia/releases/download/apk/versao.json";

/// Para onde mandar o navegador quando o manifesto não disser — ou disser algo
/// que não passa por `escolher_url`.
const APK_PADRAO: &str =
    "https://github.com/viniciusventura29/multimidia/releases/download/apk/eclipse-os.apk";

/// A única origem que este app entrega ao navegador do carro.
///
/// Não é paranoia de laboratório: a URL vem de um JSON da rede e vira um
/// `ACTION_VIEW` nativo no Android. Um `intent://` ali seria um buraco de
/// verdade, e a peneira custa uma linha.
const ORIGEM_PERMITIDA: &str = "https://github.com/viniciusventura29/multimidia/releases/";

/// Teto da pergunta, ponta a ponta.
///
/// O `reqwest` **não tem timeout por padrão** — a mesma armadilha documentada
/// em `modules/nav.rs` — e socket meio aberto é o estado normal de um carro
/// entrando em túnel. Dez segundos: o GitHub responde em menos de um.
const TETO: Duration = Duration::from_secs(10);

/// O manifesto como a CI escreve.
///
/// `commit`, `data` e `assinatura` existem lá e são ignorados aqui de propósito:
/// o motorista não vai ler um sha, e cada campo a mais é um jeito a mais de a
/// desserialização quebrar.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifesto {
    version_code: u64,
    url: Option<String>,
}

/// O que a tela recebe.
///
/// `camelCase` porque atravessa até o WebView, como no `eclipse-clima`. A `url`
/// fica FORA do JSON: quem abre o navegador é o Rust, com o valor que ele mesmo
/// peneirou. Mesmo princípio do `dispatch_action` — a tela pede o efeito de um
/// toque, não o inventa.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Atualizacao {
    pub version_code: u64,
    #[serde(skip)]
    url: String,
}

/// A última versão nova confirmada. Memória, e nunca disco.
///
/// Serve a duas coisas: é a URL que o toque abre, e é o que impede o chip de
/// piscar quando uma checagem posterior falha.
///
/// NÃO vai para disco, e isso é decisão. Um "já avisei da 42" gravado seria uma
/// segunda fonte de verdade capaz de discordar da primeira: dispensei o aviso,
/// a instalação falhou, e o carro nunca mais avisa. A dispensa de verdade já
/// existe e é grátis — instalar faz o `versionCode` crescer e o chip sumir
/// sozinho. E um "tem versão nova" que sobrevivesse ao reboot seria mentira até
/// o próximo fetch, justamente logo depois de instalar.
#[derive(Default)]
pub struct UltimaVersao(Mutex<Option<Atualizacao>>);

/// Em que versão este binário acha que está.
///
/// Env primeiro, embutido depois — a mesma ordem do `credencial()` no `lib.rs`,
/// e pelo mesmo motivo: variável de ambiente para desenvolver, valor embutido
/// para a head unit, onde não há shell antes do launcher. No Mac a env é o
/// único jeito de fingir uma versão.
///
/// **Ausente vira 0, e 0 quer dizer "não sei em que versão estou".** Todo build
/// local cai aqui, e a resposta certa não é "avise sempre" nem "avise nunca por
/// acaso": é nem perguntar. Um build local avisando de versão nova mandaria
/// quem está compilando instalar um APK por cima do próprio trabalho.
fn versao_local() -> u64 {
    std::env::var("ECLIPSE_VERSION_CODE")
        .ok()
        .or_else(|| option_env!("ECLIPSE_VERSION_CODE").map(str::to_string))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// De onde perguntar. A env existe para testar contra um arquivo local antes de
/// a CI publicar o primeiro `versao.json`; na head unit ela nunca está definida.
fn endereco_do_manifesto() -> String {
    std::env::var("ECLIPSE_MANIFESTO_URL").unwrap_or_else(|_| MANIFESTO.to_string())
}

/// Um cliente só, com teto próprio, criado na primeira pergunta.
fn cliente() -> &'static reqwest::Client {
    static CLIENTE: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENTE.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TETO)
            .user_agent("eclipse-os")
            .build()
            .unwrap_or_default()
    })
}

/// Vale avisar? Só quando eu sei em que versão estou E a de lá é maior.
fn ha_novidade(local: u64, remoto: u64) -> bool {
    local > 0 && remoto > local
}

/// A URL do manifesto, se ela passar na peneira; senão, a constante.
fn escolher_url(bruta: Option<String>) -> String {
    bruta
        .filter(|u| u.starts_with(ORIGEM_PERMITIDA))
        .unwrap_or_else(|| APK_PADRAO.to_string())
}

/// Tem versão nova?
///
/// `Ok(None)` = perguntei e não há (ou não sei em que versão estou).
/// `Err` = **não consegui** perguntar. A diferença importa, e é o front que a
/// usa: volta em 15 min quando falhou, em 6 h quando deu certo. Se os dois
/// casos devolvessem `Ok(None)`, o carro que passou uma hora sem rede só saberia
/// da versão nova seis horas depois de a rede voltar.
#[tauri::command]
pub async fn checar_atualizacao(app: tauri::AppHandle) -> Result<Option<Atualizacao>, String> {
    let local = versao_local();
    if local == 0 {
        tracing::debug!(
            "sem ECLIPSE_VERSION_CODE: não sei em que versão estou, então não pergunto"
        );
        return Ok(None);
    }

    // Cache-buster: o asset de release passa por CDN e o nome do arquivo nunca
    // muda — é justamente o que dá a URL eterna. Sem isto o carro pode passar
    // horas lendo o manifesto de ontem, que é o defeito que este código existe
    // para não ter.
    let agora = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let resposta = cliente()
        .get(endereco_do_manifesto())
        .query(&[("t", agora.to_string())])
        .header("cache-control", "no-cache")
        .send()
        .await
        .map_err(|err| {
            // `debug`, e NÃO `warn`: carro na garagem sem Wi-Fi é o estado de
            // repouso deste código, não uma anomalia. Divirjo do `nav`, que usa
            // `warn` — lá rede fora é notícia (o motorista perdeu a rota).
            // Encher o log de aviso aqui treinaria quem lê a ignorar avisos.
            tracing::debug!(%err, "não deu para perguntar se há versão nova");
            "sem rede".to_string()
        })?;

    if !resposta.status().is_success() {
        let status = resposta.status();
        tracing::debug!(%status, "o release recusou o manifesto");
        return Err(format!("o release respondeu {status}"));
    }

    let manifesto: Manifesto = resposta.json().await.map_err(|err| {
        // Este SIM é `warn`: rede ausente não é defeito, manifesto quebrado é.
        // Significa que a CI escreveu algo que este binário não entende, e o
        // carro fica cego para atualizações sem nunca reclamar.
        tracing::warn!(%err, "manifesto de versão ilegível");
        "manifesto ilegível".to_string()
    })?;

    let achado = ha_novidade(local, manifesto.version_code).then(|| Atualizacao {
        version_code: manifesto.version_code,
        url: escolher_url(manifesto.url),
    });

    if let Some(a) = &achado {
        tracing::info!(local, nova = a.version_code, "tem versão nova para o carro");
    }

    // Guardar o `None` também importa: sem isso, um toque atrasado poderia
    // abrir a URL de uma versão que já foi instalada.
    *app.state::<UltimaVersao>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = achado.clone();

    Ok(achado)
}

/// Abre o APK no navegador do carro.
///
/// ⚠️ `app.opener().open_url` (método do MANAGER), nunca a função livre
/// `tauri_plugin_opener::open_url` — a mesma armadilha já documentada no
/// `connect_spotify`: a função livre cai no crate `open`, que tenta *exec* de um
/// helper estilo xdg-open, o Android nega com EACCES, e o navegador nem abre sem
/// erro visível. O método do manager tem o branch `#[cfg(mobile)]` que dispara
/// um ACTION_VIEW nativo.
///
/// Vai direto no `.apk` e não na página do release: o navegador baixa e o
/// sistema oferece instalar, que é um passo em vez de três — e três passos numa
/// tela de carro é onde a atualização morre. Os dois saltos são HTTPS
/// (github.com → objects.githubusercontent.com), então o
/// `usesCleartextTraffic=false` do release não atrapalha.
#[tauri::command]
pub async fn baixar_atualizacao(app: tauri::AppHandle) -> Result<(), String> {
    // O `State` num `let` próprio: encadear `app.state::<_>().0.lock()` cria um
    // temporário que morre no fim da expressão, e o guarda sairia emprestando
    // algo já solto (E0716).
    let estado = app.state::<UltimaVersao>();
    let url = {
        let guarda = estado.0.lock().unwrap_or_else(|e| e.into_inner());
        guarda
            .as_ref()
            .map(|a| a.url.clone())
            .ok_or("não há atualização para baixar")?
    };

    // Vale um `info!`: quando o motorista tocar e "não acontecer nada", esta é
    // a única linha que separa "o Android recusou o ACTION_VIEW" de "o toque
    // não chegou até aqui". No carro isso se lê por `adb logcat`.
    tracing::info!(%url, "abrindo o download no navegador");
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O manifesto como o `apk.yml` escreve — inclusive os campos que ignoro.
    /// Este teste é o contrato entre a CI e o app: se o formato mudar lá, é
    /// aqui que aparece, e não num carro que parou de avisar em silêncio.
    const MANIFESTO_JSON: &str = r#"{
      "versionCode": 42,
      "versionName": "0.1.0",
      "commit": "ba37bf8",
      "data": "2026-09-10T12:00:00Z",
      "url": "https://github.com/viniciusventura29/multimidia/releases/download/apk/eclipse-os.apk",
      "notas": "https://github.com/viniciusventura29/multimidia/releases/tag/apk",
      "assinatura": "AB:CD"
    }"#;

    #[test]
    fn le_o_manifesto_da_ci_e_ignora_o_resto() {
        let m: Manifesto = serde_json::from_str(MANIFESTO_JSON).unwrap();
        assert_eq!(m.version_code, 42);
        assert!(m.url.unwrap().ends_with("eclipse-os.apk"));
    }

    #[test]
    fn versao_zero_nunca_avisa() {
        // Build local: não sei em que versão estou, então não existe "mais nova".
        assert!(!ha_novidade(0, 999));
        assert!(!ha_novidade(42, 42));
        assert!(!ha_novidade(43, 42));
        assert!(ha_novidade(42, 43));
    }

    #[test]
    fn so_abre_o_que_veio_do_nosso_release() {
        let nossa = "https://github.com/viniciusventura29/multimidia/releases/download/apk/x.apk";
        assert_eq!(escolher_url(Some(nossa.into())), nossa);
        assert_eq!(escolher_url(None), APK_PADRAO);
        assert_eq!(escolher_url(Some("intent://malvado".into())), APK_PADRAO);
        assert_eq!(
            escolher_url(Some(
                "http://github.com/viniciusventura29/multimidia/releases/x".into()
            )),
            APK_PADRAO
        );
        assert_eq!(
            escolher_url(Some("https://github.com/outro/repo/releases/x".into())),
            APK_PADRAO
        );
    }
}
