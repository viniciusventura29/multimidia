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
              A proporção que identifica o carro é a **cabine recuada**: capô
              longo, vidro curto e empurrado para trás, rabeta curta. Uma versão
              anterior espalhou o vidro por toda a distância entre eixos e o
              desenho virou um sedã genérico na hora.
            */}
            <path
              className="carrinho__lata"
              d="M 12,68
                 C 10,58 13,50 20,46
                 L 30,43
                 C 40,33 53,29 69,28
                 L 84,28
                 C 101,30 114,37 127,46
                 L 150,49
                 C 170,52 184,58 188,68
                 Z"
            />

            {/* Vidro deitado, curto e recuado. */}
            <path
              className="carrinho__vidro"
              d="M 40,43 C 50,35 60,32 71,32 L 84,32 C 99,34 110,40 120,46 Z"
            />
            <path className="carrinho__pilar" d="M 78,32 L 78,45" />
            <path className="carrinho__brilho" d="M 52,40 L 64,33" />

            {/* O para-lama traseiro estufado — a marca registrada do 3G. */}
            <path className="carrinho__anca" d="M 26,52 C 34,45 46,43 60,46" />
            <path className="carrinho__porta" d="M 78,46 L 76,66" />
            <path className="carrinho__retrovisor" d="M 129,43 L 137,40" />

            {/* Farol repuxado para trás, não redondo: é ele que dá o olhar de
                carro esportivo. Continua piscando. */}
            <path
              className="carrinho__farol"
              d="M 157,52 C 165,51 174,54 180,58 C 172,60 163,58 156,56 Z"
            />
            <path className="carrinho__farol-luz" d="M 162,53 C 168,53 173,55 177,57" />

            {/* A entrada de ar embaixo do para-choque. */}
            <rect className="carrinho__admissao" x="163" y="62" width="16" height="4" rx="2" />

            <rect className="carrinho__lanterna" x="13" y="49" width="10" height="6" rx="2.5" />
            <rect className="carrinho__freio" x="13" y="49" width="10" height="6" rx="2.5" />
          </g>

          {/* Rodas fora do grupo da carroceria: elas giram, ele mergulha. */}
          <g className="carrinho__eixo">
            <circle className="carrinho__pneu" cx="48" cy="65" r="15" />
            <g className="carrinho__roda" style={{ transformOrigin: "48px 65px" }}>
              <circle className="carrinho__aro" cx="50" cy="65" r="8.5" />
              {/* Cinco raios, e não três: a roda de muitos raios é metade do
                  visual do carro nas fotos. */}
              <path
                className="carrinho__raio"
                d="M48,65 L48,56.5 M48,65 L56.1,67.6 M48,65 L53,58.1
                   M48,65 L43,58.1 M48,65 L39.9,67.6"
              />
            </g>

            <circle className="carrinho__pneu" cx="150" cy="65" r="15" />
            <g className="carrinho__roda" style={{ transformOrigin: "150px 65px" }}>
              <circle className="carrinho__aro" cx="152" cy="65" r="8.5" />
              <path
                className="carrinho__raio"
                d="M150,65 L150,56.5 M150,65 L158.1,67.6 M150,65 L155,58.1
                   M150,65 L145,58.1 M150,65 L141.9,67.6"
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
