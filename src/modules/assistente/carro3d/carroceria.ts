/**
 * A carroceria, construída por seções transversais.
 *
 * ## Por que a extrusão foi jogada fora
 *
 * A primeira versão extrudava a silhueta lateral e depois espremia os vértices
 * das pontas. Funciona para um logotipo; para um carro, não. O que sai é um
 * bloco de lados chapados com a quina arredondada — e a lataria de um carro não
 * tem lado chapado nenhum. Cada milímetro dela é uma superfície virando, e é
 * justamente esse virar que faz o reflexo comprido escorrer do para-lama até a
 * porta. Sem ele, nenhuma quantidade de verniz ou de luz resolve: o brilho não
 * tem por onde correr.
 *
 * ## O que se faz no lugar
 *
 * O mesmo que um modelador faria: definir a **seção transversal** do carro em
 * várias estações ao longo do comprimento e costurar uma na outra. Cada seção
 * sabe onde está o assoalho, onde está o teto e qual a largura em cada altura —
 * cheia na cintura, recuada na soleira, estreita no teto. Costuradas, elas dão
 * uma superfície contínua, com ombro, cintura e para-lama, e as normais saem
 * suaves de graça.
 *
 * As curvas vêm de listas de pontos de controle interpoladas por Catmull-Rom, e
 * não de fórmulas: um capô é uma linha que alguém desenhou, não uma parábola.
 * Os pontos são os mesmos do blueprint em SVG, no sistema de lá.
 */

import { BufferAttribute, BufferGeometry } from "three";

import { CARRO, ponto } from "./blueprint";

/* ------------------------------------------------------------------ */
/* Interpolação                                                        */
/* ------------------------------------------------------------------ */

/**
 * Catmull-Rom sobre uma lista de pontos ordenados por x.
 *
 * Passa por todos os pontos de controle — que é o que se quer quando eles são o
 * desenho, e não sugestões. Bézier exigiria inventar tangentes; aqui a tangente
 * sai dos vizinhos.
 */
function curva(pontos: [number, number][], x: number): number {
  const n = pontos.length;
  if (x <= pontos[0][0]) return pontos[0][1];
  if (x >= pontos[n - 1][0]) return pontos[n - 1][1];

  let i = 0;
  while (i < n - 2 && pontos[i + 1][0] < x) i++;

  const p0 = pontos[Math.max(0, i - 1)];
  const p1 = pontos[i];
  const p2 = pontos[i + 1];
  const p3 = pontos[Math.min(n - 1, i + 2)];

  const t = (x - p1[0]) / (p2[0] - p1[0]);
  const t2 = t * t;
  const t3 = t2 * t;

  return (
    0.5 *
    (2 * p1[1] +
      (-p0[1] + p2[1]) * t +
      (2 * p0[1] - 5 * p1[1] + 4 * p2[1] - p3[1]) * t2 +
      (-p0[1] + 3 * p1[1] - 3 * p2[1] + p3[1]) * t3)
  );
}

/* ------------------------------------------------------------------ */
/* O desenho, em coordenadas do blueprint                              */
/* ------------------------------------------------------------------ */

/** Onde a lataria começa e acaba, no eixo do comprimento. */
const X_RABETA = 13;
const X_NARIZ = 187;

/**
 * A linha de cima: rabeta, vigia, teto, para-brisa, capô, nariz.
 *
 * É a silhueta que identifica o 3G — para-brisa muito deitado, teto curto e
 * fastback caindo até uma rabeta alta.
 */
const TOPO: [number, number][] = [
  [13, 50],
  [24, 47.5],
  [36, 45],
  [48, 42],
  [60, 36],
  [72, 32.6],
  [86, 32],
  [100, 33.4],
  [114, 37.5],
  [128, 44],
  [146, 47.5],
  [164, 49.5],
  [176, 51.5],
  [183, 55],
  [187, 61],
];

/** A linha de baixo: soleira reta, com o para-choque subindo nas pontas. */
const BASE: [number, number][] = [
  [13, 71],
  [22, 73.4],
  [40, 74],
  [150, 74],
  [172, 73.6],
  [187, 70.5],
];

/**
 * A largura, ao longo do comprimento, como fração da meia-largura máxima.
 *
 * Nariz e rabeta fecham em planta; o meio é cheio. Sem isto o carro teria a
 * mesma largura do para-choque ao vidro traseiro e leria como um tijolo.
 */
const LARGURA: [number, number][] = [
  [13, 0.7],
  [24, 0.88],
  [40, 0.985],
  [64, 1],
  [120, 1],
  [150, 0.985],
  [170, 0.93],
  [180, 0.85],
  [187, 0.66],
];

/**
 * O perfil do OMBRO: quanto da largura sobra em cada altura da seção.
 *
 * É a peça que mais define se a coisa parece um carro. Zero é o assoalho e um é
 * o teto. A lataria recua um pouco na soleira, incha na cintura (é lá que a
 * seção é mais larga), segura o ombro e fecha forte no teto — que num cupê é
 * bem mais estreito que a cintura.
 */
const OMBRO: [number, number][] = [
  [0, 0.8],
  [0.12, 0.93],
  [0.3, 1],
  [0.52, 0.99],
  [0.68, 0.93],
  [0.82, 0.78],
  [0.93, 0.6],
  [1, 0.4],
];

/* ------------------------------------------------------------------ */
/* O casco                                                            */
/* ------------------------------------------------------------------ */

/** Quantas fatias ao longo do comprimento e quantos pontos em volta da seção. */
const ESTACOES = 46;
const VOLTA = 26;

/**
 * O sino do para-lama: quanto a seção incha perto de cada eixo.
 *
 * O 3G tem o para-lama estufado, e ele é o traço que a lateral precisa para
 * deixar de ser uma parede. Só incha na altura da roda: inchar junto do teto
 * daria um carro barrigudo.
 */
function paraLama(xBlueprint: number, v: number): number {
  const perto = (eixo: number) => Math.exp(-Math.pow((xBlueprint - eixo) / 15, 2));
  const sino = Math.max(perto(48), perto(151));

  /*
   * Duas coisas ao mesmo tempo: a lataria INFLA acima da roda e RECOLHE na
   * altura dela.
   *
   * Só inflando, o para-lama crescia por cima da roda e a escondia — o carro
   * ficava com quatro vultos escuros embaixo em vez de rodas. O recolhimento na
   * faixa de baixo é o que abre o vão do para-lama, e é ele que deixa a face da
   * roda aparecer num três-quartos. Juntos, os dois desenham o arco.
   */
  const infla = Math.exp(-Math.pow((v - 0.46) / 0.2, 2));
  const recolhe = Math.exp(-Math.pow((v - 0.1) / 0.14, 2));
  return 1 + sino * (0.13 * infla - 0.16 * recolhe);
}

export function construirCarroceria(): BufferGeometry {
  const meiaLargura = CARRO.meiaLargura;

  const posicoes: number[] = [];
  const indices: number[] = [];

  for (let i = 0; i < ESTACOES; i++) {
    const s = i / (ESTACOES - 1);
    const xb = X_RABETA + (X_NARIZ - X_RABETA) * s;

    const topoB = curva(TOPO, xb);
    const baseB = curva(BASE, xb);
    const larg = curva(LARGURA, xb);

    for (let j = 0; j < VOLTA; j++) {
      /*
       * A volta completa da seção: sobe pelo lado direito, passa pelo teto e
       * desce pelo esquerdo. Um anel fechado, não duas metades — assim a costura
       * do teto e a do assoalho não existem, e não há emenda visível bem no meio
       * do capô, que é onde o reflexo mais denuncia.
       */
      const u = j / VOLTA;
      const lado = u < 0.5 ? 1 : -1;
      // 0 no assoalho, 1 no teto, ida e volta.
      const v = u < 0.5 ? u * 2 : (1 - u) * 2;

      const yB = baseB + (topoB - baseB) * v;
      const [x3, y3] = ponto(xb, yB);
      const z3 =
        lado * meiaLargura * larg * curva(OMBRO, v) * paraLama(xb, v);

      posicoes.push(x3, y3, z3);
    }
  }

  for (let i = 0; i < ESTACOES - 1; i++) {
    for (let j = 0; j < VOLTA; j++) {
      const a = i * VOLTA + j;
      const b = i * VOLTA + ((j + 1) % VOLTA);
      const c = (i + 1) * VOLTA + j;
      const d = (i + 1) * VOLTA + ((j + 1) % VOLTA);
      indices.push(a, c, b, b, c, d);
    }
  }

  /*
   * As tampas do nariz e da rabeta.
   *
   * O anel de cada ponta é achatado pelo perfil de largura, mas não fecha
   * sozinho — sem tampa o carro fica oco e, de três-quartos, vê-se por dentro
   * dele. Um vértice no centro de cada ponta e um leque de triângulos resolve.
   */
  const tampa = (estacao: number, inverte: boolean) => {
    let cx = 0;
    let cy = 0;
    for (let j = 0; j < VOLTA; j++) {
      const p = (estacao * VOLTA + j) * 3;
      cx += posicoes[p];
      cy += posicoes[p + 1];
    }
    const centro = posicoes.length / 3;
    posicoes.push(cx / VOLTA, cy / VOLTA, 0);

    for (let j = 0; j < VOLTA; j++) {
      const a = estacao * VOLTA + j;
      const b = estacao * VOLTA + ((j + 1) % VOLTA);
      if (inverte) indices.push(centro, b, a);
      else indices.push(centro, a, b);
    }
  };
  tampa(0, true);
  tampa(ESTACOES - 1, false);

  const geo = new BufferGeometry();
  geo.setAttribute("position", new BufferAttribute(new Float32Array(posicoes), 3));
  geo.setIndex(indices);
  geo.computeVertexNormals();

  return geo;
}

/* ------------------------------------------------------------------ */
/* A estufa                                                            */
/* ------------------------------------------------------------------ */

/**
 * A faixa envidraçada, um fio POR FORA da lataria.
 *
 * Duas tentativas antes desta falharam, e as duas por um motivo que só aparece
 * na tela. A primeira foi uma casca de vidro modelada por dentro do casco:
 * sumiu inteira, porque o casco virou um sólido fechado. A segunda foi pintar o
 * vidro de preto na própria superfície do carro — e ele desapareceu de novo,
 * agora por outra razão: com `clearcoat` alto, uma superfície preta espelha a
 * softbox e devolve um cinza claro. Preto envernizado sob luz grande é claro; é
 * o mesmo motivo pelo qual carro preto aparece branco em foto de estúdio.
 *
 * O que resolve é o vidro ter MATERIAL próprio — menos verniz, mais rugosidade,
 * mais escuro — e portanto ser malha própria. Ela reusa as mesmas seções do
 * casco, deslocada quatro milímetros para fora: perto o bastante para parecer
 * a mesma superfície, longe o bastante para dois polígonos não brigarem pelo
 * mesmo pixel.
 */
const V_CINTURA = 0.63;
const V_TETO = 0.9;

export function construirEstufa(): BufferGeometry {
  const meiaLargura = CARRO.meiaLargura;
  const posicoes: number[] = [];
  const indices: number[] = [];

  const ESTACOES_V = 30;
  const FAIXAS = 6;

  const X_INICIO = 31;
  const X_FIM = 129;

  /*
   * Duas faixas, uma de cada lado, seguindo a MESMA seção do casco.
   *
   * A tentativa anterior varria um arco que fechava no meio do teto, e o que
   * saía era um risco escuro no ombro em vez de uma janela. O vidro de um cupê
   * não é um arco: é a faixa entre a cintura e o teto, do mesmo formato da
   * lataria ali. Reusando as curvas do casco, ela encaixa por definição.
   */
  for (const lado of [1, -1]) {
    const base0 = posicoes.length / 3;

    for (let i = 0; i < ESTACOES_V; i++) {
      const s = i / (ESTACOES_V - 1);
      const xb = X_INICIO + (X_FIM - X_INICIO) * s;

      const topoB = curva(TOPO, xb);
      const baseB = curva(BASE, xb);
      const larg = curva(LARGURA, xb);
      // Fecha nas pontas: sem isso o vidro terminaria numa parede reta.
      const fecha = Math.sin(Math.PI * s) ** 0.28;

      for (let k = 0; k < FAIXAS; k++) {
        const v = V_CINTURA + (V_TETO - V_CINTURA) * (k / (FAIXAS - 1));
        const yB = baseB + (topoB - baseB) * v;
        const [x3, y3] = ponto(xb, yB);
        const z3 = lado * meiaLargura * larg * curva(OMBRO, v) * fecha;

        // Quatro milímetros para fora: perto o bastante para parecer a mesma
        // superfície, longe o bastante para não brigar por pixel com ela.
        posicoes.push(x3, y3, z3 * 1.014);
      }
    }

    for (let i = 0; i < ESTACOES_V - 1; i++) {
      for (let k = 0; k < FAIXAS - 1; k++) {
        const a = base0 + i * FAIXAS + k;
        const b = base0 + i * FAIXAS + k + 1;
        const c = base0 + (i + 1) * FAIXAS + k;
        const d = base0 + (i + 1) * FAIXAS + k + 1;
        if (lado > 0) indices.push(a, c, b, b, c, d);
        else indices.push(a, b, c, b, d, c);
      }
    }
  }

  const geo = new BufferGeometry();
  geo.setAttribute("position", new BufferAttribute(new Float32Array(posicoes), 3));
  geo.setIndex(indices);
  geo.computeVertexNormals();

  return geo;
}
