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
              {/* Logo abaixo do para-choque: na altura do corpo ela virava um
                  pontinho solto boiando ao lado do carro. */}
              <circle cx="17" cy="67" r="4" />
              <circle cx="17" cy="67" r="3" style={{ animationDelay: "0.9s" }} />
            </g>
          )}

          <g className="carrinho__corpo">
            {/*
              Proporção de brinquedo, não de fotografia. Três coisas fazem o
              carisma, e as três são deliberadas:

              1. Rodas grandes demais para o corpo (raio 15 num carro de 46 de
                 altura). Roda em escala real deixa o desenho sério.
              2. Corpo preenchido, e não contorno fino. Linha fina lê como
                 desenho técnico; forma cheia lê como ilustração.
              3. Um farol grande e redondo na frente — o olho lê como olho, e é
                 daí que vem a impressão de que o carro tem cara.
            */}
            <path
              className="carrinho__lata"
              d="M 20,64
                 C 20,52 23,46 33,43
                 L 52,40
                 C 60,22 78,16 100,16
                 L 118,16
                 C 140,18 153,26 163,40
                 L 172,44
                 C 179,47 181,54 181,64
                 Z"
            />

            {/* Um vidro só, grande e arredondado — dois vidrinhos separados
                puxam o desenho de volta para o realismo. */}
            <path
              className="carrinho__vidro"
              d="M 60,39 C 67,26 82,21 100,21 L 116,21 C 134,23 145,29 153,39 Z"
            />
            <path className="carrinho__pilar" d="M 106,22 L 106,39" />
            {/* O brilhinho no vidro: truque velho de ilustração, custa uma linha. */}
            <path className="carrinho__brilho" d="M 72,36 L 84,25" />

            <path className="carrinho__retrovisor" d="M 156,36 L 164,33" />

            {/* O farol. Grande e redondo de propósito — é o olho do bicho. */}
            <circle className="carrinho__farol" cx="170" cy="52" r="6.5" />
            <circle className="carrinho__farol-luz" cx="172" cy="50" r="2" />

            {/* A lanterna fica para dentro da borda: em cima dela, o traço
                grosso da carroceria a engolia. */}
            <rect className="carrinho__lanterna" x="24" y="47" width="7" height="8" rx="3.5" />
            <rect className="carrinho__freio" x="24" y="47" width="7" height="8" rx="3.5" />
          </g>

          {/* Rodas fora do grupo da carroceria: elas giram, ele mergulha. */}
          <g className="carrinho__eixo">
            <circle className="carrinho__pneu" cx="58" cy="64" r="15" />
            <g className="carrinho__roda" style={{ transformOrigin: "58px 64px" }}>
              <circle className="carrinho__aro" cx="58" cy="64" r="7" />
              <path
                className="carrinho__raio"
                d="M58,64 L58,56 M58,64 L64.9,68 M58,64 L51.1,68"
              />
            </g>

            <circle className="carrinho__pneu" cx="146" cy="64" r="15" />
            <g className="carrinho__roda" style={{ transformOrigin: "146px 64px" }}>
              <circle className="carrinho__aro" cx="146" cy="64" r="7" />
              <path
                className="carrinho__raio"
                d="M146,64 L146,56 M146,64 L152.9,68 M146,64 L139.1,68"
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
