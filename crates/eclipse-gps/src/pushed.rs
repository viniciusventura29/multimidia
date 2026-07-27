//! Posição empurrada de fora.
//!
//! O Rust não tem como chamar `navigator.geolocation` — quem fala com o
//! navegador é o JS. Então a posição chega pelo caminho oposto do resto dos
//! sensores: em vez do módulo perguntar, o JS empurra cada leitura (ou erro,
//! como permissão negada) por um canal, e o módulo só espera a próxima.
//!
//! É um `watch`, não uma fila: importa a posição **atual**, não o histórico de
//! todas as leituras. Isso importa porque o supervisor pode reconstruir o
//! módulo depois de um pânico, e um `watch::Receiver` clona — um `mpsc` não
//! teria como existir duas vezes.

use tokio::sync::watch;

use crate::fix::Fix;
use crate::source::{GpsError, LocationSource};

pub type Emissor = watch::Sender<Result<Fix, GpsError>>;
pub type Receptor = watch::Receiver<Result<Fix, GpsError>>;

pub struct PushedLocation {
    receptor: Receptor,
    /// A primeira leitura devolve o que já estiver no canal, mesmo sem
    /// mudança nova — é o que faz um módulo reconstruído após um pânico
    /// recuperar a última posição conhecida na hora, em vez de esperar o
    /// navegador mandar mais uma atualização.
    primeira_leitura: bool,
}

impl PushedLocation {
    /// Cria o par: o lado que o módulo escuta, e o lado que o comando do Tauri
    /// usa para repassar o que o JS mandou. Começa em "sem sinal" — é
    /// literalmente verdade até a primeira leitura do navegador chegar.
    pub fn canal() -> (Emissor, Receptor) {
        watch::channel(Err(GpsError::SemSinal))
    }

    pub fn nova(receptor: Receptor) -> Self {
        Self {
            receptor,
            primeira_leitura: true,
        }
    }
}

#[async_trait::async_trait]
impl LocationSource for PushedLocation {
    async fn next_fix(&mut self) -> Result<Fix, GpsError> {
        if self.primeira_leitura {
            self.primeira_leitura = false;
            return self.receptor.borrow_and_update().clone();
        }

        // O emissor só some se o app inteiro estiver desligando — travar para
        // sempre seria pior que reportar sem sinal.
        if self.receptor.changed().await.is_err() {
            return Err(GpsError::SemSinal);
        }
        self.receptor.borrow_and_update().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix_em(lat: f64, lon: f64) -> Fix {
        Fix {
            lat,
            lon,
            heading: 90.0,
            speed_kmh: 0.0,
        }
    }

    #[tokio::test]
    async fn comeca_sem_sinal_antes_de_qualquer_envio() {
        let (_tx, rx) = PushedLocation::canal();
        let mut gps = PushedLocation::nova(rx);
        assert!(matches!(gps.next_fix().await, Err(GpsError::SemSinal)));
    }

    #[tokio::test]
    async fn entrega_a_posicao_mais_recente_enviada() {
        let (tx, rx) = PushedLocation::canal();
        let mut gps = PushedLocation::nova(rx);

        tx.send(Ok(fix_em(-23.0, -46.0))).unwrap();
        assert_eq!(gps.next_fix().await.unwrap().lat, -23.0);

        tx.send(Ok(fix_em(-23.1, -46.1))).unwrap();
        assert_eq!(gps.next_fix().await.unwrap().lat, -23.1);
    }

    /// O erro do navegador (permissão negada) chega pelo mesmo canal, e
    /// precisa sair como erro — não travar esperando uma posição que não vem.
    #[tokio::test]
    async fn repassa_o_erro_enviado() {
        let (tx, rx) = PushedLocation::canal();
        let mut gps = PushedLocation::nova(rx);

        tx.send(Err(GpsError::SemPermissao)).unwrap();
        assert!(matches!(gps.next_fix().await, Err(GpsError::SemPermissao)));
    }

    /// A garantia que motivou usar `watch` em vez de `mpsc`: um módulo
    /// reconstruído após um pânico (um receptor clonado do mesmo canal) vê a
    /// última posição na hora, sem depender do navegador mandar de novo.
    #[tokio::test]
    async fn um_receptor_novo_ve_a_ultima_posicao_sem_precisar_de_novo_envio() {
        let (tx, rx) = PushedLocation::canal();
        tx.send(Ok(fix_em(-23.5, -46.5))).unwrap();

        let mut gps = PushedLocation::nova(rx.clone());
        assert_eq!(gps.next_fix().await.unwrap().lat, -23.5);
    }
}
