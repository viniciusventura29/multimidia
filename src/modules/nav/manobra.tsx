import type { Progresso } from "./tipos";

/**
 * A seta de cada manobra.
 *
 * Os códigos vêm do Directions. Vários viram a mesma seta de propósito: com o
 * carro andando, "vire à direita" e "vire à direita e continue" são o mesmo
 * gesto, e distinguir os dois na figura só atrapalharia.
 */
const SETAS: Record<string, string> = {
  "turn-left": "↰",
  "turn-slight-left": "↖",
  "turn-sharp-left": "⬉",
  "uturn-left": "⤶",
  "ramp-left": "↖",
  "fork-left": "↖",
  "keep-left": "↖",
  "turn-right": "↱",
  "turn-slight-right": "↗",
  "turn-sharp-right": "⬈",
  "uturn-right": "⤷",
  "ramp-right": "↗",
  "fork-right": "↗",
  "keep-right": "↗",
  "roundabout-left": "↺",
  "roundabout-right": "↻",
  merge: "⤳",
  straight: "↑",
};

function seta(manobra: string | null): string {
  return (manobra && SETAS[manobra]) || "↑";
}

function formatarDistancia(metros: number): string {
  if (metros >= 1000) return `${(metros / 1000).toFixed(1)} km`;
  if (metros >= 100) return `${Math.round(metros / 50) * 50} m`;
  return `${Math.round(metros / 10) * 10} m`;
}

function formatarTempo(segundos: number): string {
  const min = Math.round(segundos / 60);
  if (min < 60) return `${min} min`;
  return `${Math.floor(min / 60)} h ${String(min % 60).padStart(2, "0")}`;
}

/** A que horas se chega, que é o que o passageiro pergunta. */
function horaDeChegada(segundos: number): string {
  const chegada = new Date(Date.now() + segundos * 1000);
  return chegada.toLocaleTimeString("pt-BR", {
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function Manobra({ progresso }: { progresso: Progresso }) {
  if (progresso.chegou) {
    return (
      <div className="manobra manobra--chegou">
        <span className="manobra__seta">◎</span>
        <strong>chegou</strong>
      </div>
    );
  }

  if (progresso.recalcular) {
    return (
      <div className="manobra manobra--fora">
        <span className="manobra__seta">⟳</span>
        <strong>recalculando</strong>
      </div>
    );
  }

  return (
    <div className="manobra">
      <span className="manobra__seta">{seta(progresso.proximaManobra)}</span>

      <div className="manobra__texto">
        <strong className="manobra__distancia">
          {formatarDistancia(progresso.distanciaParaManobraM)}
        </strong>
        <span className="manobra__instrucao">{progresso.proximaInstrucao}</span>
        {progresso.proximoDetalhe && (
          <span className="manobra__detalhe">{progresso.proximoDetalhe}</span>
        )}
      </div>

      <div className="manobra__resumo">
        <strong>{horaDeChegada(progresso.chegadaEmS)}</strong>
        <span>
          {formatarDistancia(progresso.distanciaRestanteM)} ·{" "}
          {formatarTempo(progresso.chegadaEmS)}
        </span>
      </div>
    </div>
  );
}
