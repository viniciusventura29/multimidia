/**
 * O carro da coluna do assistente.
 *
 * Aparece quando não há quadro para mostrar — que é a maior parte do tempo. Em
 * vez de um espaço vazio ou de um cartão dizendo "tudo normal", a coluna vira
 * uma animação do próprio carro.
 *
 * **Andando, ela lê a telemetria de verdade.** As rodas giram na velocidade que
 * o OBD (ou o GPS) está relatando, a carroceria mergulha quando o carro freia
 * de fato, a luz de freio acende junto, e o carro inclina para o lado quando o
 * rumo do GPS muda. É a mesma ideia do resto do painel: o desenho é uma
 * projeção do carro, não um enfeite ao lado dele.
 *
 * **Parado, ela vira bobagem.** O carro balança no ponto morto, solta fumaça e
 * de vez em quando passa um cachorro.
 *
 * Tudo é `@keyframes` alimentado por custom properties, e não `requestAnimation
 * Frame` quadro a quadro: o WebView de uma head unit barata não sobra CPU, e
 * animação declarada é a única que o compositor consegue tocar sozinho. Mesma
 * técnica da barra do `Gauge`.
 */

import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";

import type { ModuleStates, ObdReadings } from "../../core/types";

interface Fix {
  heading: number;
  speedKmh: number;
}

/** Abaixo disto o carro conta como parado. GPS parado treme uns décimos. */
const LIMIAR_PARADO = 2;

function limitar(valor: number, minimo: number, maximo: number): number {
  return Math.max(minimo, Math.min(maximo, valor));
}

/**
 * Diferença de rumo pelo caminho mais curto, em graus (-180 a 180).
 *
 * Sem isto, passar de 359° para 1° pareceria uma guinada de 358 graus e o carro
 * daria um pulo na tela. Mesmo cuidado que a câmera do mapa toma.
 */
function diferencaDeRumo(atual: number, anterior: number): number {
  return ((atual - anterior + 540) % 360) - 180;
}

/** Aceleração (km/h por segundo) e velocidade de guinada (graus por segundo). */
function useDinamica(velocidade: number | null, rumo: number | null) {
  const anterior = useRef<{ v: number; r: number | null; t: number } | null>(null);
  const [dinamica, setDinamica] = useState({ aceleracao: 0, curva: 0 });

  useEffect(() => {
    const agora = performance.now();
    const v = velocidade ?? 0;
    const antes = anterior.current;
    anterior.current = { v, r: rumo, t: agora };

    if (!antes) return;

    const dt = (agora - antes.t) / 1000;
    // Duas amostras coladas dariam uma derivada absurda — e um mergulho de
    // freada onde não houve freada nenhuma.
    if (dt < 0.05) return;

    setDinamica({
      aceleracao: (v - antes.v) / dt,
      curva:
        antes.r === null || rumo === null
          ? 0
          : diferencaDeRumo(rumo, antes.r) / dt,
    });
  }, [velocidade, rumo]);

  return dinamica;
}

export function Carrinho({ painel }: { painel: ModuleStates }) {
  const obd = painel["obd"]?.data as ObdReadings | null | undefined;
  const nav = painel["nav"]?.data as { fix?: Fix | null } | null | undefined;

  // O OBD primeiro, o GPS como reserva. No Mac o OBD é sempre degradado (é
  // Bluetooth, só existe no Android), e é o GPS que faz a animação funcionar
  // enquanto se trabalha no layout.
  const velocidade = obd?.speedKmh ?? nav?.fix?.speedKmh ?? null;
  const rpm = obd?.rpm ?? null;
  const rumo = nav?.fix?.heading ?? null;

  const { aceleracao, curva } = useDinamica(velocidade, rumo);

  const vel = velocidade ?? 0;
  const parado = vel < LIMIAR_PARADO;

  const estilo = {
    // Períodos, não velocidades: é o que `animation-duration` consome. Quanto
    // mais rápido o carro, menor o período.
    "--giro": `${limitar(30 / (vel + 5), 0.15, 3).toFixed(2)}s`,
    "--pista": `${limitar(24 / (vel + 4), 0.1, 4).toFixed(2)}s`,
    // A partir da marcha lenta (~700 rpm) até o corte.
    "--tremor": limitar(((rpm ?? 700) - 700) / 5200, 0, 1).toFixed(2),
    // Desacelerar joga o nariz para baixo; acelerar levanta, mas menos —
    // suspensão de carro não é simétrica.
    "--mergulho": `${limitar(-aceleracao * 0.18, -2, 3).toFixed(2)}deg`,
    "--inclina": `${limitar(curva * 0.12, -7, 7).toFixed(2)}px`,
    "--freio": limitar(-aceleracao / 12, 0, 1).toFixed(2),
  } as CSSProperties;

  return (
    <div
      className={`carrinho ${parado ? "carrinho--parado" : "carrinho--andando"}`}
      style={estilo}
      aria-hidden
    >
      {/*
        A cena é em retrato porque a coluna é. Um carro de perfil é largo por
        natureza; sozinho, numa coluna estreita e alta, ele viraria uma tirinha
        no meio de muito vazio. Com morro e horizonte atrás, a mesma faixa vira
        uma cena que ocupa a coluna inteira.
      */}
      <svg className="carrinho__svg" viewBox="0 0 200 360" role="presentation">
        <defs>
          {/* O brilho do horizonte. Abstrato de propósito: lua e estrelas
              diriam "é noite", e o painel fica ligado o dia inteiro. */}
          <radialGradient id="ia-brilho" cx="50%" cy="100%" r="80%">
            <stop offset="0%" stopColor="var(--accent)" stopOpacity="0.22" />
            <stop offset="55%" stopColor="var(--accent)" stopOpacity="0.06" />
            <stop offset="100%" stopColor="var(--accent)" stopOpacity="0" />
          </radialGradient>
        </defs>

        {/* Sobe até o topo da cena: sem isso o céu vira um retângulo morto
            ocupando metade da coluna. */}
        <rect x="0" y="0" width="200" height="318" fill="url(#ia-brilho)" />

        {/* Dois planos de morro atrás, com o horizonte na base dos dois. */}
        <path
          className="carrinho__morro carrinho__morro--fundo"
          d="M0,318 L26,236 L52,278 L88,206 L124,262 L158,222 L200,268 L200,318 Z"
        />
        <path
          className="carrinho__morro"
          d="M0,318 L34,268 L74,300 L112,246 L150,290 L200,258 L200,318 Z"
        />
        <line className="carrinho__horizonte" x1="0" y1="318" x2="200" y2="318" />

        {/* O carro e a pista, encaixados na faixa de baixo da cena. */}
        <g transform="translate(4, 245) scale(0.96)">
          {/* Fumaça do escapamento: só existe com o motor em marcha lenta. */}
          {parado && (
            <g className="carrinho__fumaca">
              <circle cx="15" cy="76" r="4" />
              <circle cx="15" cy="76" r="3" style={{ animationDelay: "0.9s" }} />
            </g>
          )}

          {/*
            As rodas vêm ANTES da lataria de propósito. Com a soleira baixa que o
            blueprint pede, metade da roda fica dentro do vão do para-lama — e é
            a lataria por cima que recorta esse vão. Desenhando a roda depois,
            ela ficaria colada por fora, como um adesivo.
          */}
          <g className="carrinho__eixo">
            <circle className="carrinho__pneu" cx="48" cy="65" r="14.5" />
            <g className="carrinho__roda" style={{ transformOrigin: "48px 65px" }}>
              <circle className="carrinho__aro" cx="48" cy="65" r="8" />
              {/* Cinco raios, como a roda do blueprint. */}
              <path
                className="carrinho__raio"
                d="M48,65 L48,57 M48,65 L55.6,67.5 M48,65 L52.7,58.5
                   M48,65 L43.3,58.5 M48,65 L40.4,67.5"
              />
            </g>

            <circle className="carrinho__pneu" cx="151" cy="65" r="14.5" />
            <g className="carrinho__roda" style={{ transformOrigin: "151px 65px" }}>
              <circle className="carrinho__aro" cx="151" cy="65" r="8" />
              <path
                className="carrinho__raio"
                d="M151,65 L151,57 M151,65 L158.6,67.5 M151,65 L155.7,58.5
                   M151,65 L146.3,58.5 M151,65 L143.4,67.5"
              />
            </g>
          </g>

          <g className="carrinho__corpo">
            {/*
              O Eclipse 3G, e não um carrinho genérico. A silhueta é o que
              identifica: cunha baixa e comprida, nariz caído, para-brisa muito
              deitado, teto curto que desce em fastback até uma rabeta alta —
              e a asa em cima dela, que é o traço que mais entrega o carro.

              O estilo continua sendo ilustração e não diagrama: forma
              preenchida, traço grosso arredondado e roda maior que a escala
              real pediria. É o que separa "personagem" de "manual de oficina".
            */}

            {/* A asa, atrás e por baixo da lataria para os montantes sumirem
                dentro dela. */}
            <path className="carrinho__asa" d="M 11,39 L 37,36" />
            <path className="carrinho__asa-pe" d="M 18,38 L 19,44 M 31,37 L 32,42" />

            {/*
              A silhueta saiu do blueprint, não do olho: entre-eixos em 58% do
              comprimento, balanços de ~21% cada, altura em 27%. A cabine é
              recuada — capô longo, vidro curto e empurrado para trás.

              O que mais mudou em relação às tentativas anteriores foi a
              **soleira**: ela desce quase até o chão (y 74 de 80). Antes estava
              em 68, e o carro parecia levantado — era o que mais roubava a cara
              de esportivo, mais que qualquer detalhe.

              Os dois arcos no fim do traçado são os vãos de roda, e são arcos
              de circunferência de verdade (`A`), centrados na roda e com raio
              1,5 maior que ela. Feitos com Bézier antes, ficavam altos demais e
              abriam um buraco por onde se via o fundo em cima do pneu.
            */}
            <path
              className="carrinho__lata"
              d="M 12,74
                 C 10,66 10,56 13,50
                 L 30,46
                 L 43,44
                 C 52,37 62,32 79,32
                 C 98,32 113,37 128,45
                 L 176,51
                 C 183,53 188,57 188,62
                 L 188,74
                 L 164.2,74
                 A 16 16 0 1 0 137.8,74
                 L 61.2,74
                 A 16 16 0 1 0 34.8,74
                 Z"
            />

            {/* Vidro deitado, curto e recuado. */}
            <path
              className="carrinho__vidro"
              d="M 49,46 C 57,38 66,34.5 79,35 C 95,35.5 108,40 121,48 Z"
            />
            <path className="carrinho__pilar" d="M 88,35 L 88,47" />
            <path className="carrinho__brilho" d="M 60,44 L 72,37" />

            {/* O para-lama traseiro estufado — a marca registrada do 3G. */}
            <path className="carrinho__anca" d="M 27,53 C 35,47 47,45 61,48" />
            {/* A tampa redonda do tanque, que aparece no blueprint. */}
            <circle className="carrinho__tanque" cx="37" cy="49" r="3" />
            <path className="carrinho__porta" d="M 88,48 L 86,66" />
            <path className="carrinho__retrovisor" d="M 130,42 L 138,39" />

            {/* Farol repuxado para trás, não redondo: é ele que dá o olhar de
                carro esportivo. Continua piscando. */}
            <path
              className="carrinho__farol"
              d="M 169,52 C 176,52 183,55 187,60 C 179,61 172,58 168,56 Z"
            />
            <path className="carrinho__farol-luz" d="M 173,54 C 178,55 182,57 185,59" />

            {/* A entrada de ar embaixo do para-choque. */}
            <rect className="carrinho__admissao" x="171" y="64" width="15" height="5" rx="2" />

            <rect className="carrinho__lanterna" x="15" y="48" width="10" height="5" rx="2.5" />
            <rect className="carrinho__freio" x="15" y="48" width="10" height="5" rx="2.5" />
          </g>

          {/* A pista. Andando ela corre; parada, fica. */}
          <line
            className="carrinho__pista"
            x1="-10"
            y1="80"
            x2="210"
            y2="80"
            strokeDasharray="18 14"
          />

          {/* O cachorro atravessa na frente do carro. Atrás ele ficava escondido
              pela lataria, que agora vai quase até o chão — sobravam as patas. */}
          {parado && (
            <g className="carrinho__cachorro">
              <rect x="5" y="67" width="19" height="9" rx="4.5" />
              <circle cx="26" cy="67" r="4.5" />
              <path d="M23,64 L26,58.5 L29.5,64 Z" />
              <path
                className="carrinho__patas"
                d="M8,76 L8,81 M13,76 L13,81 M18,76 L18,81 M22,76 L22,81"
              />
              <path className="carrinho__rabo" d="M5,69 L0,62" />
            </g>
          )}
        </g>
      </svg>
    </div>
  );
}
