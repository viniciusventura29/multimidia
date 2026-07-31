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
          {/* Atrás do carro: o cachorro passa por trás, não por cima. */}
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

          {/* Fumaça do escapamento: só existe com o motor em marcha lenta. */}
          {parado && (
            <g className="carrinho__fumaca">
              <circle cx="15" cy="65" r="4" />
              <circle cx="15" cy="65" r="3" style={{ animationDelay: "0.9s" }} />
            </g>
          )}

          <g className="carrinho__corpo">
            {/*
              O perfil é de coupé, não de sedã: teto curto entre os pilares,
              vidro traseiro deitado e para-brisa dianteiro bem inclinado
              descendo até um nariz baixo. É o desenho do Eclipse 2G, e é o que
              separa "um carro" de "este carro".
            */}
            <path
              className="carrinho__lata"
              d="M 14,62 L 14,54 C 14,49 17,46 23,45 L 44,42
                 C 54,30 74,25 98,25 L 116,25
                 C 136,27 150,33 164,44 L 186,50
                 C 191,51 193,55 193,59 L 193,62 Z"
            />

            {/* Vidros, com o pilar B entre os dois. */}
            <path
              className="carrinho__vidro"
              d="M 52,41 C 60,31 76,28 96,28 L 100,28 L 100,41 Z"
            />
            <path
              className="carrinho__vidro"
              d="M 104,28 L 114,28 C 132,30 145,35 156,41 L 104,41 Z"
            />

            <path className="carrinho__vinco" d="M 24,53 L 182,53" />
            <path className="carrinho__porta" d="M 101,42 L 99,61" />
            <path className="carrinho__retrovisor" d="M 158,40 L 165,38" />

            {/* Faróis. O de trás acende com a freada. */}
            <ellipse className="carrinho__farol" cx="187" cy="54" rx="5" ry="2.8" />
            <rect className="carrinho__lanterna" x="14.5" y="48" width="5" height="6" rx="1.5" />
            <rect className="carrinho__freio" x="14.5" y="48" width="5" height="6" rx="1.5" />
          </g>

          {/* Rodas fora do grupo da carroceria: elas giram, ele mergulha. */}
          <g className="carrinho__eixo">
            <circle className="carrinho__pneu" cx="54" cy="64" r="12" />
            <g className="carrinho__roda" style={{ transformOrigin: "54px 64px" }}>
              <circle className="carrinho__aro" cx="54" cy="64" r="6.5" />
              {/* Três raios: com dois, cruzados, a roda vira uma mira. */}
              <path
                className="carrinho__raio"
                d="M54,64 L54,57.5 M54,64 L59.6,67.3 M54,64 L48.4,67.3"
              />
            </g>

            <circle className="carrinho__pneu" cx="150" cy="64" r="12" />
            <g className="carrinho__roda" style={{ transformOrigin: "150px 64px" }}>
              <circle className="carrinho__aro" cx="150" cy="64" r="6.5" />
              <path
                className="carrinho__raio"
                d="M150,64 L150,57.5 M150,64 L155.6,67.3 M150,64 L144.4,67.3"
              />
            </g>
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
        </g>
      </svg>
    </div>
  );
}
