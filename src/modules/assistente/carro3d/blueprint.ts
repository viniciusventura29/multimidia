/**
 * A silhueta do Eclipse, em números.
 *
 * São **os mesmos** do desenho em SVG (`carrinho.tsx`) — de propósito, e escritos
 * aqui no sistema de coordenadas de lá em vez de já convertidos. Um carro é
 * reconhecido pela silhueta antes de qualquer outra coisa; se o 3D partisse de
 * uma curva nova, ele viraria "um cupê" em vez de "o carro do Vinicius", e o
 * painel teria dois carros diferentes dependendo de qual desenho estivesse na
 * tela. Convertendo em tempo de carga, a silhueta continua tendo um dono só.
 *
 * O sistema do SVG: x cresce para a frente do carro, y cresce para BAIXO, o
 * chão está em y=80 e o carro ocupa de x=12 a x=188.
 */

import { Shape } from "three";

/** 1 unidade do blueprint em metros. Sai de: 176 unidades = 4,39 m de Eclipse. */
const ESCALA = 0.025;

/** Onde fica o meio do carro no eixo do blueprint. */
const MEIO = 100;

/** A linha do chão no blueprint. */
const CHAO = 80;

/**
 * Blueprint → metros, com o chão em y=0 e o carro centrado na origem.
 *
 * Devolve tupla, e não `Vector2`: os pontos são espalhados direto nos
 * `bezierCurveTo` da `Shape`, e só uma tupla de tamanho conhecido pode ser
 * espalhada num argumento posicional sem o TypeScript reclamar.
 */
export const ponto = (x: number, y: number): [number, number] => [
  (x - MEIO) * ESCALA,
  (CHAO - y) * ESCALA,
];

const m = (n: number) => n * ESCALA;

/* ------------------------------------------------------------------ */
/* Medidas que o resto da cena precisa                                 */
/* ------------------------------------------------------------------ */

export const CARRO = {
  comprimento: m(188 - 12),
  altura: m(CHAO - 32),
  /**
   * Meia-largura da carroceria.
   *
   * **Mais estreita que o carro de verdade (1,75 m), e isso é o conserto de um
   * bug de leitura.** O desenho em SVG recorta o para-lama para a roda aparecer;
   * em três dimensões esse recorte viraria um túnel de um lado ao outro, e sem
   * ele a carroceria — larga como a bitola — engolia as quatro rodas: o carro
   * saía um bloco liso pousado no chão.
   *
   * Estreitando o corpo e mantendo a bitola, a roda passa a sobrar para fora da
   * silhueta e volta a ser vista, que é o que um três-quartos precisa mostrar.
   * Um carro de brinquedo bem-proporcionado faz exatamente isso.
   */
  meiaLargura: 0.7,
  /** Altura do teto, em metros — o eixo do afunilamento. */
  teto: m(CHAO - 32),
  eixo: {
    traseiro: m(48 - MEIO),
    dianteiro: m(151 - MEIO),
    altura: m(CHAO - 65),
    raio: m(14.5),
    largura: 0.26,
    /** Distância do plano central até o meio da roda. Maior que a meia-largura
     *  da carroceria de propósito — ver acima. */
    bitola: 0.74,
  },
};

/* ------------------------------------------------------------------ */
/* Os contornos                                                        */
/* ------------------------------------------------------------------ */

/**
 * A carroceria vista de lado.
 *
 * **Sem os recortes de para-lama** que o SVG tem. Lá eles são necessários
 * porque o desenho é chapado e a roda precisa de um buraco por onde aparecer;
 * aqui a roda é um sólido que atravessa a carroceria, e o recorte viraria um
 * túnel de um lado ao outro do carro. O que se vê é o mesmo: roda encaixada
 * embaixo do para-lama.
 */
export function perfilDaCarroceria(): Shape {
  const s = new Shape();
  const p = ponto;

  s.moveTo(...p(12, 74));
  // A traseira sobe quase reta — é a rabeta alta do fastback.
  s.bezierCurveTo(...p(10, 66), ...p(10, 56), ...p(13, 50));
  s.lineTo(...p(30, 46));
  s.lineTo(...p(43, 44));
  // Para-brisa muito deitado e teto curto: é o traço que mais entrega o 3G.
  s.bezierCurveTo(...p(52, 37), ...p(62, 32), ...p(79, 32));
  s.bezierCurveTo(...p(98, 32), ...p(113, 37), ...p(128, 45));
  // Capô longo, caindo até o nariz.
  s.lineTo(...p(176, 51));
  s.bezierCurveTo(...p(183, 53), ...p(188, 57), ...p(188, 62));
  s.lineTo(...p(188, 74));
  s.closePath();

  return s;
}

/** O vidro: para-brisa, teto e vigia, numa peça só. */
export function perfilDoVidro(): Shape {
  const s = new Shape();
  const p = ponto;

  s.moveTo(...p(49, 46));
  s.bezierCurveTo(...p(57, 38), ...p(66, 34.5), ...p(79, 35));
  s.bezierCurveTo(...p(95, 35.5), ...p(108, 40), ...p(121, 48));
  s.closePath();

  return s;
}

/* ------------------------------------------------------------------ */
/* O afunilamento                                                      */
/* ------------------------------------------------------------------ */

/**
 * O que transforma um contorno extrudado em carroceria.
 *
 * Extrusão pura dá uma fatia de bolo: o mesmo perfil repetido de um lado ao
 * outro, com quinas vivas nas laterais. Nenhum carro é assim — a lataria
 * afunila para cima (o teto é mais estreito que a cintura) e recua nas pontas.
 *
 * A conta abaixo empurra cada vértice para dentro conforme ele se afasta do
 * plano central, e empurra MAIS quanto mais alto ele estiver: é o que dá o teto
 * afunilado e a cintura cheia. O expoente alto deixa a maior parte da largura
 * intacta e concentra a curvatura perto das laterais, que é onde a luz precisa
 * de uma superfície virando para render brilho de carro em vez de mancha.
 *
 * Feito no vértice, e não com um modelo pronto: um GLTF de carro bom pesa
 * megabytes, precisa de licença, e nunca seria um Eclipse. Aqui a forma sai da
 * silhueta que já era nossa.
 */
export function afunilar(
  posicoes: Float32Array,
  meiaLargura: number,
  teto: number,
): void {
  for (let i = 0; i < posicoes.length; i += 3) {
    const x = posicoes[i];
    const y = posicoes[i + 1];
    const z = posicoes[i + 2];

    // 0 no plano central do carro, 1 na lateral.
    const t = Math.min(1, Math.abs(z) / meiaLargura);
    // 0 no chão, 1 no teto.
    const alt = Math.min(1, Math.max(0, y / teto));

    // O teto fecha bem mais que a soleira.
    const fechaY = 0.05 + 0.34 * Math.pow(alt, 1.5);
    posicoes[i + 1] = y * (1 - fechaY * Math.pow(t, 2.4));
    // As pontas recuam de leve: nariz e rabeta arredondam em planta.
    posicoes[i] = x * (1 - 0.05 * Math.pow(t, 3));
  }
}
