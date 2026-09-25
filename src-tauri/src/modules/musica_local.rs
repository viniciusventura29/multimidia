//! A música do carro: o app do Spotify DESTA central, e mais nada.
//!
//! # Uma coisa toca. Uma só.
//!
//! O dono foi direto: *"o pior caso n serve. Ou a gente faz o web API muito bom
//! e ele fica incrível, ou a gente foca nesse outro jeito e ele fica incrível.
//! Um ou outro, os dois não."*
//!
//! Ele tem razão, e não é só questão de gosto. A cadeia de fallback que existia
//! aqui — App Remote, depois sessão de mídia, depois Web API — custava caro de
//! três jeitos: três caminhos para manter, falhas ambíguas (qual camada
//! desistiu?), e um "pior caso" que mascarava o problema em vez de expô-lo.
//!
//! Então sobrou um: **o App Remote**. Se ele não atender, a música não toca e a
//! tela diz por quê.
//!
//! # Por que ele, e não a Web API
//!
//! Não foi preferência. A Web API só comanda um aparelho que já esteja
//! anunciado no Spotify Connect, e o app do Spotify só se anuncia depois de ser
//! ABERTO na mão — fluxo que o dono recusou, com razão. Três versões tentaram
//! contornar isso:
//!
//! - tocar pela WebView (o Web Playback SDK): virou "pula cinco músicas e o som
//!   morre", porque decodificar áudio com DRM numa SoC barata não dá;
//! - casar o nome do aparelho: o Android chama a central de "K706" em todos os
//!   campos que conhece, e o Spotify se anuncia como "HT-9960CA";
//! - acordar o app por `bindService`: eu afirmei que funcionava, baseado numa
//!   correlação no diário, e o diário seguinte desmentiu.
//!
//! O App Remote não esbarra em nada disso: ele inicia o processo do Spotify
//! sozinho e toca no aparelho onde roda.
//!
//! # O que a Web API ainda faz — e por que isso NÃO é "os dois"
//!
//! Buscar. O App Remote não tem busca, ponto: a interface dele não expõe
//! nenhuma (conferido no `.aar`, não de memória). E o dono gosta da barra de
//! busca.
//!
//! A diferença é que agora a Web API não TOCA mais nada. Ela lê listas. Há
//! exatamente um caminho que produz som, e quando ele falha ninguém precisa
//! perguntar qual camada falhou.

use async_trait::async_trait;
use eclipse_music::{Busca, Contexto, MusicError, MusicSource, NowPlaying, Recentes};
use serde::Deserialize;
use tauri_plugin_obd_bt::ObdBtExt;

/// O que o Kotlin devolve. Ver `AppRemoteSpotify.estado`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct EstadoLocal {
    #[serde(default)]
    conectado: bool,
    #[serde(default)]
    tem: bool,
    #[serde(default)]
    motivo: String,
    #[serde(default)]
    faixa: String,
    #[serde(default)]
    artista: String,
    #[serde(default)]
    tocando: bool,
    #[serde(default)]
    posicao_ms: i64,
    #[serde(default)]
    duracao_ms: i64,
    /// Só vem quando a faixa MUDA — ver `capaEnviadaDe` no Kotlin.
    #[serde(default)]
    capa: Option<String>,
}

pub struct SessaoLocal {
    app: tauri::AppHandle,
    /// Credenciais do App Remote. O plugin não guarda credencial; elas
    /// atravessam a ponte a cada chamada.
    client_id: String,
    redirect_uri: &'static str,
    /// A biblioteca: busca, playlists, recentes. **Não toca nada.**
    biblioteca: Box<dyn MusicSource>,
    /// A última capa recebida.
    ///
    /// O Kotlin só manda a capa quando a faixa muda — ela custa dezenas de KB
    /// de base64, e repeti-la a cada segundo seria desperdício. "Veio sem
    /// capa" quer dizer "a mesma de antes", não "não tem capa".
    capa: Option<String>,
    /// Por que o App Remote não está atendendo. Para o diário, uma vez só.
    ja_reclamou: bool,
}

impl SessaoLocal {
    pub fn nova(
        app: tauri::AppHandle,
        client_id: String,
        biblioteca: Box<dyn MusicSource>,
    ) -> Self {
        Self {
            app,
            client_id,
            redirect_uri: eclipse_music::REDIRECT_URI,
            biblioteca,
            capa: None,
            ja_reclamou: false,
        }
    }

    /// Manda tocar. `Ok(())` = o app da central atendeu.
    ///
    /// Sem plano B: o erro sobe para a tela com o motivo que o Spotify deu.
    ///
    /// Recebe as credenciais por valor em vez de usar `&self`: `MusicSource` é
    /// `Send` mas não `Sync`, e segurar um `&self` através do `await` tornaria
    /// esta future não-`Send` — que o trait exige.
    async fn tocar_no_app(
        app: tauri::AppHandle,
        id: String,
        redirect: &'static str,
        faixa: Option<&str>,
        contexto: Option<&str>,
    ) -> Result<(), MusicError> {
        let uri = faixa.map(str::to_string);
        let ctx = contexto.map(str::to_string);

        let saida = tokio::task::spawn_blocking(move || {
            app.obd_bt()
                .app_remote_tocar(&id, redirect, uri.as_deref(), ctx.as_deref(), -1)
        })
        .await;

        match saida {
            Ok(Ok(None)) => Ok(()),
            Ok(Ok(Some(motivo))) => Err(traduzir(&motivo)),
            Ok(Err(err)) => Err(MusicError::Network(format!("a ponte falhou: {err}"))),
            Err(err) => Err(MusicError::Network(format!("a task morreu: {err}"))),
        }
    }

    /// Um toque de transporte. Mesma regra: sem plano B, e sem `&self` vivo
    /// através do `await` (ver `tocar_no_app`).
    async fn transporte(
        app: tauri::AppHandle,
        id: String,
        redirect: &'static str,
        acao: &'static str,
        valor: i64,
    ) -> Result<(), MusicError> {
        let saida = tokio::task::spawn_blocking(move || {
            app.obd_bt().app_remote_comando(&id, redirect, acao, valor)
        })
        .await;

        match saida {
            Ok(Ok(None)) => Ok(()),
            Ok(Ok(Some(motivo))) => Err(traduzir(&motivo)),
            Ok(Err(err)) => Err(MusicError::Network(format!("a ponte falhou: {err}"))),
            Err(err) => Err(MusicError::Network(format!("a task morreu: {err}"))),
        }
    }
}

/// O motivo cru do Kotlin vira um erro que a tela sabe apresentar.
///
/// Importa distinguir: "não está instalado" e "recusou a conexão" pedem coisas
/// diferentes do dono, e um texto genérico deixaria os dois iguais.
fn traduzir(motivo: &str) -> MusicError {
    let m = motivo.to_lowercase();
    if m.contains("não está instalado") || m.contains("not installed") {
        return MusicError::Network("instale o Spotify nesta central".into());
    }
    if m.contains("not logged in") || m.contains("authentication") || m.contains("auth") {
        return MusicError::NeedsReauth;
    }
    MusicError::Network(format!("o Spotify da central: {motivo}"))
}

/// Converte a leitura crua em [`NowPlaying`], lembrando a capa.
///
/// Fora do método de propósito: é a única lógica aqui que não depende do
/// Android, e é a que quebra calada. Se o cache da capa falhar, a capa aparece
/// por um segundo e SOME — sem erro nenhum, porque tecnicamente tudo funcionou.
fn montar(estado: EstadoLocal, guardada: &mut Option<String>) -> Option<NowPlaying> {
    if !estado.tem {
        return None;
    }

    // "Veio sem capa" quer dizer "a mesma de antes", e não "não tem capa".
    if let Some(capa) = estado.capa {
        *guardada = Some(capa);
    }

    Some(NowPlaying {
        track: estado.faixa,
        artist: estado.artista,
        is_playing: estado.tocando,
        album_art: guardada.clone(),
        // `max(0)` + `try_from`: posição negativa ou absurda vira ausência, e
        // não uma barra de progresso maluca.
        progress_ms: u32::try_from(estado.posicao_ms.max(0)).ok(),
        duration_ms: u32::try_from(estado.duracao_ms.max(0))
            .ok()
            .filter(|d| *d > 0),
    })
}

#[async_trait]
impl MusicSource for SessaoLocal {
    async fn now_playing(&mut self) -> Result<Option<NowPlaying>, MusicError> {
        let (app, id, redirect) = (self.app.clone(), self.client_id.clone(), self.redirect_uri);
        let bruto =
            tokio::task::spawn_blocking(move || app.obd_bt().app_remote_estado(&id, redirect))
                .await
                .map_err(|e| MusicError::Network(format!("a task morreu: {e}")))?
                .map_err(|e| MusicError::Network(format!("a ponte falhou: {e}")))?;

        let estado: EstadoLocal = serde_json::from_str(&bruto)
            .map_err(|e| MusicError::Network(format!("resposta ilegível: {e}")))?;

        if !estado.conectado {
            if !self.ja_reclamou {
                self.ja_reclamou = true;
                tracing::warn!(motivo = %estado.motivo, "o app do Spotify da central não atendeu");
            }
            return Err(traduzir(&estado.motivo));
        }
        if self.ja_reclamou {
            self.ja_reclamou = false;
            tracing::info!("o app do Spotify da central voltou");
        }

        Ok(montar(estado, &mut self.capa))
    }

    async fn toggle(&mut self) -> Result<(), MusicError> {
        Self::transporte(
            self.app.clone(),
            self.client_id.clone(),
            self.redirect_uri,
            "alternar",
            0,
        )
        .await
    }

    async fn next(&mut self) -> Result<(), MusicError> {
        Self::transporte(
            self.app.clone(),
            self.client_id.clone(),
            self.redirect_uri,
            "proxima",
            0,
        )
        .await
    }

    async fn previous(&mut self) -> Result<(), MusicError> {
        Self::transporte(
            self.app.clone(),
            self.client_id.clone(),
            self.redirect_uri,
            "anterior",
            0,
        )
        .await
    }

    async fn seek(&mut self, posicao_ms: u32) -> Result<(), MusicError> {
        Self::transporte(
            self.app.clone(),
            self.client_id.clone(),
            self.redirect_uri,
            "saltar",
            i64::from(posicao_ms),
        )
        .await
    }

    async fn tocar(
        &mut self,
        faixa: Option<&str>,
        contexto: Option<&str>,
    ) -> Result<(), MusicError> {
        Self::tocar_no_app(
            self.app.clone(),
            self.client_id.clone(),
            self.redirect_uri,
            faixa,
            contexto,
        )
        .await
    }

    // A biblioteca inteira segue na Web API: o App Remote toca e controla, mas
    // não sabe BUSCAR — a interface dele não expõe busca nenhuma.
    async fn buscar(&mut self, termo: &str) -> Result<Busca, MusicError> {
        self.biblioteca.buscar(termo).await
    }

    async fn abrir(&mut self, uri: &str) -> Result<Contexto, MusicError> {
        self.biblioteca.abrir(uri).await
    }

    async fn recentes(&mut self) -> Result<Recentes, MusicError> {
        self.biblioteca.recentes().await
    }

    async fn playlists(&mut self) -> Result<Vec<eclipse_music::Playlist>, MusicError> {
        self.biblioteca.playlists().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tocando(faixa: &str, capa: Option<&str>) -> EstadoLocal {
        EstadoLocal {
            conectado: true,
            tem: true,
            faixa: faixa.into(),
            artista: "Alguém".into(),
            tocando: true,
            posicao_ms: 1_000,
            duracao_ms: 200_000,
            capa: capa.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn a_capa_sobrevive_as_leituras_seguintes() {
        // O Kotlin manda a capa UMA vez, quando a faixa muda. Sem guardá-la,
        // ela apareceria por um segundo e sumiria — sem erro nenhum, porque
        // tecnicamente tudo funcionou.
        let mut capa = None;
        let primeira = montar(
            tocando("Faixa A", Some("data:image/jpeg;base64,AAA")),
            &mut capa,
        )
        .expect("tocando");
        assert_eq!(
            primeira.album_art.as_deref(),
            Some("data:image/jpeg;base64,AAA")
        );

        let segunda = montar(tocando("Faixa A", None), &mut capa).expect("tocando");
        assert_eq!(
            segunda.album_art.as_deref(),
            Some("data:image/jpeg;base64,AAA"),
            "vir sem capa quer dizer 'a mesma de antes'"
        );
    }

    #[test]
    fn trocar_de_faixa_troca_a_capa() {
        let mut capa = None;
        montar(tocando("Faixa A", Some("capa-A")), &mut capa);
        let nova = montar(tocando("Faixa B", Some("capa-B")), &mut capa).expect("tocando");
        assert_eq!(nova.album_art.as_deref(), Some("capa-B"));
    }

    #[test]
    fn parado_nao_e_erro_e_nao_inventa_faixa() {
        let mut capa = Some("capa-velha".to_string());
        let parado = EstadoLocal {
            conectado: true,
            tem: false,
            ..Default::default()
        };
        assert!(montar(parado, &mut capa).is_none());
    }

    #[test]
    fn duracao_zero_vira_ausencia_e_nao_barra_vazia() {
        let mut capa = None;
        let sem = EstadoLocal {
            duracao_ms: 0,
            ..tocando("Faixa", None)
        };
        assert_eq!(montar(sem, &mut capa).unwrap().duration_ms, None);
    }

    #[test]
    fn posicao_negativa_nao_vira_numero_gigante() {
        let mut capa = None;
        let torta = EstadoLocal {
            posicao_ms: -5,
            ..tocando("Faixa", None)
        };
        assert_eq!(montar(torta, &mut capa).unwrap().progress_ms, Some(0));
    }

    #[test]
    fn o_motivo_vira_a_saida_certa() {
        // "Não instalado" e "recusou" pedem coisas diferentes do dono; um
        // texto genérico deixaria os dois iguais e sem saída.
        assert!(matches!(
            traduzir("o Spotify não está instalado nesta central"),
            MusicError::Network(m) if m.contains("instale")
        ));
        assert!(matches!(
            traduzir("User is not logged in"),
            MusicError::NeedsReauth
        ));
        assert!(matches!(
            traduzir("qualquer outra coisa"),
            MusicError::Network(_)
        ));
    }
}
