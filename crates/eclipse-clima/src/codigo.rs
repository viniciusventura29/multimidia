//! O código WMO virando algo que se desenha e se lê.
//!
//! O Open-Meteo devolve o tempo como um inteiro do catálogo WMO 4677 — `61` é
//! chuva fraca, `95` é tempestade. Isso não serve nem para a tela nem para a
//! IA: o painel precisa de um ícone, e o motorista precisa de duas palavras em
//! português.
//!
//! A tradução mora aqui, e não no React, por dois motivos. O primeiro é a regra
//! da casa — quem entende o dado é o Rust, e o front só desenha o que já veio
//! com significado. O segundo é que a tabela tem vinte e oito entradas e uma
//! regra de agrupamento; testá-la aqui custa um `assert`, testá-la no WebView
//! custaria montar um runner de teste que o projeto não tem.

use serde::Serialize;

/// A que família o tempo pertence — é isto que escolhe o ícone.
///
/// Existe para o painel não precisar decorar os vinte e oito códigos WMO só
/// para saber se desenha uma nuvem ou um raio. As famílias são grosseiras de
/// propósito: num relance de motorista, garoa e chuva fraca são a mesma coisa.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Familia {
    /// Sem nuvem que atrapalhe. É o único que troca de ícone à noite (lua).
    Limpo,
    Nuvem,
    Nevoa,
    Chuva,
    Neve,
    Tempestade,
}

/// O código WMO em família e frase.
///
/// Códigos fora do catálogo caem em [`Familia::Nuvem`] com "tempo instável":
/// inventar "céu limpo" para um número desconhecido seria mentir para o lado
/// errado — melhor um palpite morno que uma promessa de sol.
pub fn descrever(codigo: u8) -> (Familia, &'static str) {
    use Familia::*;

    match codigo {
        0 => (Limpo, "céu limpo"),
        1 => (Limpo, "quase limpo"),
        2 => (Nuvem, "parcialmente nublado"),
        3 => (Nuvem, "nublado"),
        45 => (Nevoa, "névoa"),
        48 => (Nevoa, "névoa gelada"),
        51 => (Chuva, "garoa fraca"),
        53 => (Chuva, "garoa"),
        55 => (Chuva, "garoa forte"),
        56 | 57 => (Chuva, "garoa congelante"),
        61 => (Chuva, "chuva fraca"),
        63 => (Chuva, "chuva"),
        65 => (Chuva, "chuva forte"),
        66 | 67 => (Chuva, "chuva congelante"),
        71 => (Neve, "neve fraca"),
        73 => (Neve, "neve"),
        75 => (Neve, "neve forte"),
        77 => (Neve, "grãos de neve"),
        80 => (Chuva, "pancadas fracas"),
        81 => (Chuva, "pancadas de chuva"),
        82 => (Chuva, "pancadas fortes"),
        85 => (Neve, "pancadas de neve"),
        86 => (Neve, "nevasca"),
        95 => (Tempestade, "tempestade"),
        96 | 99 => (Tempestade, "tempestade com granizo"),
        _ => (Nuvem, "tempo instável"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_faixas_caem_na_familia_certa() {
        assert_eq!(descrever(0).0, Familia::Limpo);
        assert_eq!(descrever(3).0, Familia::Nuvem);
        assert_eq!(descrever(45).0, Familia::Nevoa);
        assert_eq!(descrever(65).0, Familia::Chuva);
        assert_eq!(descrever(75).0, Familia::Neve);
        assert_eq!(descrever(99).0, Familia::Tempestade);
    }

    /// Pancada de chuva é chuva e pancada de neve é neve — o `8x` é a faixa
    /// onde é fácil errar, porque os dois moram coladinhos.
    #[test]
    fn pancadas_separam_agua_de_neve() {
        assert_eq!(descrever(82).0, Familia::Chuva);
        assert_eq!(descrever(85).0, Familia::Neve);
    }

    /// O desconhecido não pode virar sol: o painel diria "céu limpo" debaixo de
    /// um temporal só porque o catálogo cresceu.
    #[test]
    fn codigo_de_fora_do_catalogo_nao_promete_sol() {
        let (familia, frase) = descrever(200);
        assert_ne!(familia, Familia::Limpo);
        assert_eq!(frase, "tempo instável");
    }

    /// Nada de frase vazia: ela vai direto para a tela e para o prompt da IA.
    #[test]
    fn toda_frase_tem_texto() {
        for codigo in 0..=u8::MAX {
            assert!(!descrever(codigo).1.is_empty(), "código {codigo} sem frase");
        }
    }
}
