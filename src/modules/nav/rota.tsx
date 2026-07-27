import { useEffect, useRef, useState, type FormEvent, type MouseEvent } from "react";
import { useMap, useMapsLibrary } from "@vis.gl/react-google-maps";

import { dispatchAction } from "../../core/actions";
import type { Fix, Progresso, Rota } from "./tipos";

const NAV = "nav";

/** Tira as tags que o Directions manda dentro das instruções. */
function semHtml(texto: string): string {
  return texto
    .replace(/<[^>]*>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function formatarDistancia(metros: number): string {
  return metros >= 1000
    ? `${(metros / 1000).toFixed(1)} km`
    : `${Math.round(metros / 10) * 10} m`;
}

function formatarTempo(segundos: number): string {
  const min = Math.round(segundos / 60);
  if (min < 60) return `${min} min`;
  return `${Math.floor(min / 60)} h ${String(min % 60).padStart(2, "0")}`;
}

/**
 * Busca a rota e entrega ao Rust.
 *
 * O `DirectionsService` mora aqui porque é parte do SDK do mapa. Mas a rota não
 * fica aqui: assim que chega, vai para o Rust, que é onde a posição vive. É lá
 * que dá para responder "quanto falta" e "saí do caminho" — perguntas que
 * precisam da rota e do GPS ao mesmo tempo.
 */
export function BuscarRota({ fix, rota }: { fix: Fix | null; rota: Rota | null }) {
  const map = useMap();
  const biblioteca = useMapsLibrary("routes");
  const [destino, setDestino] = useState("");
  const [buscando, setBuscando] = useState(false);
  const [erro, setErro] = useState<string | null>(null);
  const desenho = useRef<google.maps.Polyline | null>(null);

  // Desenha a rota recebida de volta do Rust — não a que acabou de ser buscada.
  // Assim o que aparece na tela é sempre o que o Rust está usando para guiar.
  useEffect(() => {
    if (!map) return;

    desenho.current ??= new google.maps.Polyline({
      map,
      strokeColor: "#4da3ff",
      strokeOpacity: 0.75,
      strokeWeight: 9,
      zIndex: 1,
    });

    desenho.current.setPath(
      (rota?.pontos ?? []).map(([lat, lng]) => ({ lat, lng })),
    );
  }, [map, rota]);

  useEffect(() => () => desenho.current?.setMap(null), []);


  const tracar = async (event: FormEvent) => {
    event.preventDefault();
    event.stopPropagation();

    const alvo = destino.trim();
    if (!alvo || !biblioteca || !fix) return;

    setBuscando(true);
    setErro(null);

    try {
      const servico = new biblioteca.DirectionsService();
      const resposta = await servico.route({
        origin: { lat: fix.lat, lng: fix.lon },
        destination: alvo,
        travelMode: google.maps.TravelMode.DRIVING,
      });

      const perna = resposta.routes[0].legs[0];

      dispatchAction(NAV, {
        acao: "rota",
        rota: {
          destino: perna.end_address ?? alvo,
          pontos: resposta.routes[0].overview_path.map((p) => [p.lat(), p.lng()]),
          passos: perna.steps.map((passo) => ({
            instrucao: semHtml(passo.instructions ?? ""),
            distanciaM: passo.distance?.value ?? 0,
          })),
          distanciaTotalM: perna.distance?.value ?? 0,
          duracaoTotalS: perna.duration?.value ?? 0,
        },
      });

      setDestino("");
    } catch {
      // A mensagem do Google é técnica demais para um painel de carro.
      setErro("não achei esse endereço");
    } finally {
      setBuscando(false);
    }
  };

  const cancelar = (event: MouseEvent) => {
    event.stopPropagation();
    dispatchAction(NAV, { acao: "cancelar" });
  };

  if (rota) {
    return (
      <div className="rota__ativa" onClick={(e) => e.stopPropagation()}>
        <span className="rota__destino">{rota.destino}</span>
        <button className="rota__cancelar" onClick={cancelar}>
          encerrar
        </button>
      </div>
    );
  }

  return (
    <form className="rota__busca" onSubmit={tracar} onClick={(e) => e.stopPropagation()}>
      <input
        className="rota__campo"
        value={destino}
        onChange={(e) => setDestino(e.target.value)}
        placeholder={erro ?? "para onde?"}
        aria-label="Destino"
      />
      <button className="rota__ir" type="submit" disabled={!destino.trim() || buscando || !fix}>
        {buscando ? "…" : "ir"}
      </button>
    </form>
  );
}

/**
 * A faixa de manobra.
 *
 * É o mais perto de turn-by-turn que dá para chegar sem o Navigation SDK: a
 * próxima manobra e quanto falta para ela, calculados cruzando a rota com a
 * posição. Não há orientação de faixa nem trânsito ao vivo.
 */
export function Manobra({ progresso }: { progresso: Progresso }) {
  if (progresso.chegou) {
    return (
      <div className="manobra manobra--chegou">
        <strong>chegou</strong>
      </div>
    );
  }

  if (progresso.foraDaRota) {
    // Dizer que saiu é honesto; fingir que recalcula sozinho não seria.
    return (
      <div className="manobra manobra--fora">
        <strong>fora da rota</strong>
        <span>{formatarDistancia(progresso.desvioM)} do caminho</span>
      </div>
    );
  }

  return (
    <div className="manobra">
      <strong className="manobra__distancia">
        {formatarDistancia(progresso.distanciaParaManobraM)}
      </strong>
      <span className="manobra__instrucao">{progresso.proximaInstrucao}</span>
      <span className="manobra__restante">
        {formatarDistancia(progresso.distanciaRestanteM)} ·{" "}
        {formatarTempo(progresso.chegadaEmS)}
      </span>
    </div>
  );
}
