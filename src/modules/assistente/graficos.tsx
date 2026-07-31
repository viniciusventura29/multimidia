/**
 * Os dois gráficos do assistente, desenhados à mão.
 *
 * Sem recharts nem chart.js de propósito. O painel inteiro já desenha assim — o
 * `Gauge` é uma div com `width` em porcentagem, os indicadores de bateria e
 * gasolina são SVG escrito à mão — e uma biblioteca de gráfico traria eixos,
 * tooltips e tipografia própria que destoariam de tudo em volta, além de somar
 * uns 150 kB ao bundle para desenhar quatro barras.
 */

import type { CSSProperties } from "react";

import type { Ponto } from "./tipos";

/** Menos que isto não é gráfico, é um número. */
export const MINIMO_PONTOS = 2;
/** Mais que isto não cabe na coluna estreita. */
const MAXIMO_PONTOS = 8;

interface Props {
  pontos: Ponto[];
  cor: string;
}

/** Escala os valores para 0..1, tolerando série constante. */
function normalizar(valores: number[]): number[] {
  const maior = Math.max(...valores);
  const menor = Math.min(...valores);
  const faixa = maior - menor;

  // Série constante (quatro leituras iguais de temperatura, por exemplo): sem
  // este caso a divisão por zero viraria NaN e o gráfico sumiria.
  if (faixa === 0) return valores.map(() => 0.5);

  // O piso em 0,08 mantém o menor ponto visível: barra de altura zero parece
  // dado faltando, e o que se quer mostrar é que ele é o menor, não que não há.
  return valores.map((v) => 0.08 + ((v - menor) / faixa) * 0.92);
}

export function Barras({ pontos, cor }: Props) {
  const usados = pontos.slice(0, MAXIMO_PONTOS);
  const alturas = normalizar(usados.map((p) => p.valor));

  return (
    <div className="grf grf--barras" style={{ "--tom": cor } as CSSProperties}>
      {usados.map((ponto, i) => (
        <div className="grf__col" key={`${ponto.rotulo}-${i}`}>
          <div className="grf__trilho">
            <div
              className="grf__barra"
              style={{ height: `${alturas[i] * 100}%` }}
            />
          </div>
          <span className="grf__rotulo">{ponto.rotulo}</span>
        </div>
      ))}
    </div>
  );
}

export function Linha({ pontos, cor }: Props) {
  const usados = pontos.slice(0, MAXIMO_PONTOS);
  const alturas = normalizar(usados.map((p) => p.valor));

  // `viewBox` de 0..100 nos dois eixos com `preserveAspectRatio="none"`: a
  // caixa estica para o tamanho que a coluna tiver sem nenhuma conta de layout
  // aqui. O `vector-effect` é o que impede a linha de engordar na horizontal e
  // afinar na vertical quando isso acontece.
  const traco = alturas
    .map((altura, i) => {
      const x = usados.length === 1 ? 50 : (i / (usados.length - 1)) * 100;
      return `${x},${100 - altura * 100}`;
    })
    .join(" ");

  return (
    <div className="grf grf--linha" style={{ "--tom": cor } as CSSProperties}>
      <svg
        className="grf__svg"
        viewBox="0 0 100 100"
        preserveAspectRatio="none"
        aria-hidden
      >
        <polyline
          className="grf__traco"
          points={traco}
          vectorEffect="non-scaling-stroke"
        />
      </svg>

      <div className="grf__eixo">
        <span className="grf__rotulo">{usados[0]?.rotulo}</span>
        <span className="grf__rotulo">{usados[usados.length - 1]?.rotulo}</span>
      </div>
    </div>
  );
}
