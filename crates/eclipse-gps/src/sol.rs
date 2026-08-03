//! É dia ou é noite?
//!
//! O tema do mapa vira sozinho, e a pergunta certa não é "que horas são" — é
//! "onde está o sol". Horário fixo erraria uma hora e meia entre junho e
//! dezembro; consultar uma API custaria rede e cota para responder algo que é
//! só astronomia. A elevação solar sai de lat/lon + instante com a fórmula da
//! NOAA simplificada, erro abaixo de meio grau — sobra para "dia ou noite".

/// Elevação do sol em graus acima do horizonte para (lat, lon) num instante
/// Unix (UTC). Negativa = sol abaixo do horizonte.
fn elevacao_solar(lat: f64, lon: f64, unix_s: u64) -> f64 {
    // Dias desde J2000.0 (2000-01-01 12:00 UTC), o zero das efemérides.
    let n = unix_s as f64 / 86_400.0 - 10_957.5;

    // Posição do sol na eclíptica: longitude média corrigida pela elipse da
    // órbita (anomalia média `g`), depois projetada em declinação e ascensão
    // reta pela inclinação do eixo da Terra.
    let g = (357.528 + 0.985_600_3 * n).to_radians();
    let lambda =
        ((280.460 + 0.985_647_4 * n) + 1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians();
    let epsilon = (23.439 - 0.000_000_4 * n).to_radians();

    let declinacao = (epsilon.sin() * lambda.sin()).asin();
    let ascensao = (epsilon.cos() * lambda.sin()).atan2(lambda.cos());

    // Ângulo horário: quanto o sol já passou do meridiano local. A hora
    // sideral de Greenwich gira ~360,9857°/dia (o dia sideral é mais curto).
    let gmst_graus = 280.460_618_37 + 360.985_647_366_29 * n;
    let hora = (gmst_graus + lon - ascensao.to_degrees()).to_radians();

    let phi = lat.to_radians();
    (phi.sin() * declinacao.sin() + phi.cos() * declinacao.cos() * hora.cos())
        .asin()
        .to_degrees()
}

/// Noite civil: sol mais de 0,833° abaixo do horizonte (o raio do disco somado
/// à refração atmosférica — o mesmo limiar que define o pôr do sol no jornal).
pub fn e_noite(lat: f64, lon: f64, unix_s: u64) -> bool {
    elevacao_solar(lat, lon, unix_s) < -0.833
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAO_PAULO: (f64, f64) = (-23.5505, -46.6333);

    /// 2026-01-15 na Unix epoch, meia-noite UTC.
    const JAN_15: u64 = 1_768_435_200;

    #[test]
    fn meio_dia_em_sao_paulo_e_dia() {
        // 12:00 local = 15:00 UTC, verão — sol quase a pino.
        assert!(!e_noite(SAO_PAULO.0, SAO_PAULO.1, JAN_15 + 15 * 3600));
    }

    #[test]
    fn meia_noite_em_sao_paulo_e_noite() {
        // 00:00 local = 03:00 UTC.
        assert!(e_noite(SAO_PAULO.0, SAO_PAULO.1, JAN_15 + 3 * 3600));
    }

    /// O motivo de calcular em vez de tabelar horários: às 18h30 locais é
    /// noite fechada no inverno e ainda dia claro no verão.
    #[test]
    fn as_dezoito_e_meia_depende_da_estacao() {
        const JUN_15: u64 = 1_781_481_600; // 2026-06-15 00:00 UTC
        const DEZ_15: u64 = 1_797_292_800; // 2026-12-15 00:00 UTC
        let dezoito_e_meia_local = 21 * 3600 + 1800; // 21:30 UTC

        assert!(e_noite(
            SAO_PAULO.0,
            SAO_PAULO.1,
            JUN_15 + dezoito_e_meia_local
        ));
        assert!(!e_noite(
            SAO_PAULO.0,
            SAO_PAULO.1,
            DEZ_15 + dezoito_e_meia_local
        ));
    }

    #[test]
    fn na_linha_do_equador_meio_dia_e_dia_e_meia_noite_e_noite() {
        assert!(!e_noite(0.0, 0.0, JAN_15 + 12 * 3600));
        assert!(e_noite(0.0, 0.0, JAN_15));
    }

    /// Sol da meia-noite: em Tromsø (69,6° N) em junho o sol não se põe. Se a
    /// fórmula aguenta latitude alta, aguenta qualquer estrada daqui.
    #[test]
    fn latitude_alta_nao_quebra() {
        const JUN_15: u64 = 1_781_481_600;
        // Meia-noite solar local (~22:45 UTC do dia anterior, lon 18,96° E).
        assert!(!e_noite(69.6, 18.96, JUN_15 - 3600 - 900));
    }
}
