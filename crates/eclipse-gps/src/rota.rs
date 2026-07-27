//! O traçado que o carro simulado percorre.
//!
//! São coordenadas aproximadas de um trajeto real: sobe a Av. Paulista do
//! Paraíso até a Consolação, e desce a Consolação em direção à República. Não
//! são precisas ao meio-fio — a intenção é o mapa andar por ruas que existem,
//! para o modo navegação parecer o que vai parecer no carro.

/// Pontos do traçado, em ordem.
pub const TRACADO: [(f64, f64); 14] = [
    (-23.5719, -46.6410), // Paulista, altura do Paraíso
    (-23.5700, -46.6440),
    (-23.5680, -46.6470),
    (-23.5660, -46.6500),
    (-23.5640, -46.6530),
    (-23.5620, -46.6560),
    (-23.5600, -46.6590),
    (-23.5580, -46.6615), // fim da Paulista, Consolação
    (-23.5550, -46.6600), // desce a Consolação
    (-23.5520, -46.6570),
    (-23.5490, -46.6540),
    (-23.5460, -46.6510),
    (-23.5440, -46.6480),
    (-23.5430, -46.6440), // República
];
