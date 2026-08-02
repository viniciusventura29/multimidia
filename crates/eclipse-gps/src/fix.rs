use serde::Serialize;

/// Uma posição.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    /// Rumo em graus, 0 = norte, crescendo no sentido horário.
    ///
    /// É o que gira o mapa para o sentido da marcha. Parado, um GPS de verdade
    /// devolve rumo instável — por isso o último rumo bom é preservado em vez de
    /// zerar, senão o mapa rodaria sozinho em cada semáforo.
    pub heading: f32,
    pub speed_kmh: f32,
    /// Raio de incerteza relatado pelo provedor (o círculo de 68%), em metros.
    ///
    /// GPS de verdade dá unidades; geolocalização por Wi-Fi dá dezenas. É o que
    /// dimensiona a zona morta do [`FiltroDeParada`](crate::FiltroDeParada) —
    /// um raio fixo que servisse para o Wi-Fi engoliria movimento real no GPS.
    pub accuracy_m: f32,
}

/// Rumo do ponto `a` para o ponto `b`, em graus.
pub fn rumo(a: (f64, f64), b: (f64, f64)) -> f32 {
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let dlon = lon2 - lon1;

    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();

    (y.atan2(x).to_degrees() as f32).rem_euclid(360.0)
}

/// Distância entre dois pontos em metros (haversine).
pub fn distancia_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    const RAIO_TERRA_M: f64 = 6_371_000.0;

    let (lat1, lat2) = (a.0.to_radians(), b.0.to_radians());
    let dlat = lat2 - lat1;
    let dlon = (b.1 - a.1).to_radians();

    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * RAIO_TERRA_M * h.sqrt().asin()
}

/// Ponto a `fracao` do caminho entre `a` e `b`.
///
/// Interpolação linear em graus. Em segmentos de rua, com décimos de grau, o
/// erro contra a geodésica é de centímetros — não vale a complexidade.
pub fn interpolar(a: (f64, f64), b: (f64, f64), fracao: f64) -> (f64, f64) {
    (
        a.0 + (b.0 - a.0) * fracao,
        a.1 + (b.1 - a.1) * fracao,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAULISTA: (f64, f64) = (-23.5614, -46.6559);

    #[test]
    fn rumo_aponta_para_os_pontos_cardeais() {
        let norte = (PAULISTA.0 + 0.01, PAULISTA.1);
        let leste = (PAULISTA.0, PAULISTA.1 + 0.01);
        let sul = (PAULISTA.0 - 0.01, PAULISTA.1);
        let oeste = (PAULISTA.0, PAULISTA.1 - 0.01);

        assert!(rumo(PAULISTA, norte).abs() < 1.0);
        assert!((rumo(PAULISTA, leste) - 90.0).abs() < 1.0);
        assert!((rumo(PAULISTA, sul) - 180.0).abs() < 1.0);
        assert!((rumo(PAULISTA, oeste) - 270.0).abs() < 1.0);
    }

    #[test]
    fn rumo_fica_sempre_entre_0_e_360() {
        let oeste = (PAULISTA.0, PAULISTA.1 - 0.01);
        let r = rumo(PAULISTA, oeste);
        assert!((0.0..360.0).contains(&r), "rumo fora da faixa: {r}");
    }

    #[test]
    fn distancia_bate_com_a_realidade() {
        // Um grau de latitude são ~111 km em qualquer lugar do planeta.
        let um_grau_ao_norte = (PAULISTA.0 + 1.0, PAULISTA.1);
        let d = distancia_m(PAULISTA, um_grau_ao_norte);
        assert!((110_000.0..112_000.0).contains(&d), "deu {d} m");
    }

    #[test]
    fn interpolar_no_meio_fica_no_meio() {
        let b = (PAULISTA.0 + 0.02, PAULISTA.1 + 0.02);
        let meio = interpolar(PAULISTA, b, 0.5);
        let d1 = distancia_m(PAULISTA, meio);
        let d2 = distancia_m(meio, b);
        assert!((d1 - d2).abs() < 1.0, "{d1} vs {d2}");
    }
}
