/**
 * O que o carro da tela lê do carro de verdade.
 *
 * Mora fora dos dois desenhos — o de SVG e o de três dimensões — porque os dois
 * respondem à mesma coisa e não podem discordar: se a roda do 3D girasse por uma
 * conta e a do SVG por outra, o dia em que um substituísse o outro o painel
 * mudaria de comportamento sem ninguém ter pedido.
 */

import { useEffect, useRef, useState } from "react";

import { shallowEqual, useModuleSelector } from "../../core/moduleStore";
import type { ObdReadings } from "../../core/types";

interface Fix {
  heading: number;
  speedKmh: number;
}

/** Abaixo disto o carro conta como parado. GPS parado treme uns décimos. */
export const LIMIAR_PARADO = 2;

export function limitar(valor: number, minimo: number, maximo: number): number {
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

export interface EstadoDoCarro {
  /** km/h. Zero quando não há leitura — é o repouso, não uma ausência. */
  velocidade: number;
  /** Giro do motor, ou `null` enquanto o OBD não responde. */
  rpm: number | null;
  /** km/h por segundo. Negativa é freada. */
  aceleracao: number;
  /** Graus por segundo. Positiva é curva para um lado, negativa para o outro. */
  curva: number;
  parado: boolean;
}

/**
 * A telemetria como o desenho do carro precisa dela.
 *
 * A exceção declarada do painel: esta animação reage à telemetria de verdade, e
 * isso exige ler os módulos vizinhos. Assina direto no store, com seletores — o
 * resto do tile do assistente não paga por estes ticks.
 */
export function useTelemetriaDoCarro(): EstadoDoCarro {
  const obd = useModuleSelector<
    ObdReadings,
    { speedKmh: number | null; rpm: number | null }
  >(
    "obd",
    (d) => ({ speedKmh: d?.speedKmh ?? null, rpm: d?.rpm ?? null }),
    shallowEqual,
  );
  const fix = useModuleSelector<
    { fix?: Fix | null },
    { heading: number | null; speedKmh: number | null }
  >(
    "nav",
    (d) => ({
      heading: d?.fix?.heading ?? null,
      speedKmh: d?.fix?.speedKmh ?? null,
    }),
    shallowEqual,
  );

  // O OBD primeiro, o GPS como reserva. No Mac o OBD é sempre degradado (é
  // Bluetooth, só existe no Android), e é o GPS que faz a animação funcionar
  // enquanto se trabalha no layout.
  const velocidade = obd.speedKmh ?? fix.speedKmh ?? null;
  const { aceleracao, curva } = useDinamica(velocidade, fix.heading);

  const vel = velocidade ?? 0;

  return {
    velocidade: vel,
    rpm: obd.rpm,
    aceleracao,
    curva,
    parado: vel < LIMIAR_PARADO,
  };
}
