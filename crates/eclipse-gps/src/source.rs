use async_trait::async_trait;

use crate::fix::Fix;

#[derive(Debug, thiserror::Error)]
pub enum GpsError {
    /// No Android, ler posição exige permissão concedida pelo usuário.
    #[error("permissão de localização não concedida")]
    SemPermissao,

    /// Céu bloqueado, garagem, túnel. É estado normal de um GPS, não defeito.
    #[error("sem sinal de satélite")]
    SemSinal,
}

/// De onde vem a posição.
///
/// Mesma forma dos outros sensores: hoje é o trajeto simulado, amanhã é o
/// `LocationManager` do Android atrás de um plugin Kotlin, ou um dongle USB.
#[async_trait]
pub trait LocationSource: Send {
    /// Espera a próxima posição. A espera é parte do contrato — um GPS entrega
    /// cerca de uma leitura por segundo, não um fluxo contínuo.
    async fn next_fix(&mut self) -> Result<Fix, GpsError>;

    /// Pede para o carro passar a andar por este caminho.
    ///
    /// Só faz sentido para o simulador, e por isso o padrão é não fazer nada: um
    /// GPS de verdade relata onde o carro está, não obedece a para onde ir. Está
    /// no trait — e não num tipo concreto — porque quem tem a rota é o módulo, e
    /// ele só enxerga o trait.
    fn seguir(&mut self, _caminho: &[(f64, f64)]) {}
}
