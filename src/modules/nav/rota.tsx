import { useEffect, useRef, useState, type FormEvent, type MouseEvent } from "react";
import type { GeoJSONSource } from "maplibre-gl";

import { dispatchAction } from "../../core/actions";
import { NOSSO, useMapa } from "./mapaContexto";
import { destacar, useSugestoes } from "./sugestoes";
import { calarSe } from "./voz";
import type { Fix, Rota } from "./tipos";

const NAV = "nav";

/** Fonte e camada da rota, no estilo do mapa. */
const FONTE = `${NOSSO}rota`;

/**
 * A rota desenhada no mapa.
 *
 * Desenha a rota recebida de volta do Rust — não uma que este lado tenha
 * buscado. Quem busca é o módulo `nav` (ver `directions.rs`), então o que
 * aparece na tela é sempre, por construção, o que o Rust está usando para
 * guiar: não existe versão da rota que só a tela conheça.
 *
 * Vive nos dois tamanhos do tile: o campo de destino só existe em tela cheia,
 * mas a linha precisa continuar aparecendo no mapa pequeno — é com ele na tela
 * que se dirige.
 */
export function RotaDesenhada({ rota }: { rota: Rota | null }) {
  const map = useMapa();
  // Que traçado já está desenhado. O envelope do Rust chega a cada leitura de
  // GPS com um objeto novinho em folha, então comparar identidade acusaria
  // mudança 1x por segundo e redesenharia a rota inteira à toa — numa head unit
  // isso é gasto contínuo de CPU para pintar exatamente os mesmos pixels.
  const desenhado = useRef<string | null>(null);

  useEffect(() => {
    if (!map) return;

    // A camada é criada uma vez e sobrevive à troca de tema: quem a reconduz
    // para o estilo novo é o `transformStyle` em `mapa.tsx`, pelo prefixo.
    if (!map.getSource(FONTE)) {
      map.addSource(FONTE, {
        type: "geojson",
        data: { type: "FeatureCollection", features: [] },
      });
      map.addLayer({
        id: FONTE,
        type: "line",
        source: FONTE,
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#4da3ff",
          "line-width": 9,
          "line-opacity": 0.75,
        },
      });
      desenhado.current = null;
    }

    const assinatura = rota ? `${rota.destino}:${rota.pontos.length}` : null;
    if (assinatura === desenhado.current) return;
    desenhado.current = assinatura;

    const fonte = map.getSource<GeoJSONSource>(FONTE);
    if (!fonte) return;

    fonte.setData({
      type: "Feature",
      properties: {},
      // GeoJSON é [longitude, latitude]; o Rust manda (lat, lon).
      geometry: {
        type: "LineString",
        coordinates: (rota?.pontos ?? []).map(([lat, lng]) => [lng, lat]),
      },
    });
  }, [map, rota]);

  return null;
}

/**
 * O campo de destino, as sugestões e a rota em curso.
 *
 * Só existe em tela cheia: digitar endereço num tile do tamanho de um cartão é
 * briga perdida, e era metade da poluição do mapa pequeno.
 *
 * Daqui sai apenas **para onde** ir — o `placeId` que o motorista tocou, ou o
 * texto que ele digitou. Quem transforma isso em rota é o Rust, que é onde a
 * posição vive; e é por isso que o recálculo ao sair do caminho não passa mais
 * por este lado: virou uma chamada lá, não um pedido para cá.
 */
export function BuscarRota({
  rota,
  apiKey,
  fix,
  buscando,
  erro,
}: {
  rota: Rota | null;
  apiKey: string;
  /** Só para enviesar as sugestões pelo que está perto do carro — a origem da
   *  rota quem sabe é o Rust. */
  fix: Fix | null;
  buscando: boolean;
  erro: string | null;
}) {
  const [destino, setDestino] = useState("");
  const { sugestoes, limpar } = useSugestoes(destino, apiKey, fix);

  const pedir = (alvo: { placeId?: string; texto?: string; rotulo: string }) => {
    dispatchAction(NAV, { acao: "rota", alvo });
    setDestino("");
    limpar();
  };

  const escolher = (placeId: string, rotulo: string) => {
    // Vai pelo `placeId`, não pelo texto: é o lugar exato que o motorista tocou,
    // sem o Google ter que adivinhar de novo a partir da frase. E o rótulo é o
    // nome que ele leu na lista — melhor num painel de carro que o endereço
    // formatado que a API devolveria.
    pedir({ placeId, rotulo });
  };

  const tracar = (event: FormEvent) => {
    event.preventDefault();
    event.stopPropagation();

    // Enter sem escolher nada aproveita a primeira sugestão: é o que o motorista
    // espera, e o lugar exato dá rota melhor que o texto solto.
    const primeira = sugestoes[0];
    if (primeira) {
      escolher(primeira.placeId, primeira.principal);
      return;
    }

    const texto = destino.trim();
    if (!texto) return;
    pedir({ texto, rotulo: texto });
  };

  const cancelar = (event: MouseEvent) => {
    event.stopPropagation();
    calarSe();
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
    <div className="rota__caixa" onClick={(e) => e.stopPropagation()}>
      {sugestoes.length > 0 && (
        <ul className="sugestoes">
          {sugestoes.map((s) => (
            <li key={s.placeId}>
              <button
                className="sugestao"
                onClick={() => escolher(s.placeId, s.principal)}
              >
                <span className="sugestao__pino" aria-hidden>
                  ⌖
                </span>
                <span className="sugestao__texto">
                  <span className="sugestao__principal">
                    {destacar(s.principal, s.destaques).map((parte, i) =>
                      parte.forte ? (
                        <strong key={i}>{parte.trecho}</strong>
                      ) : (
                        <span key={i}>{parte.trecho}</span>
                      ),
                    )}
                  </span>
                  <span className="sugestao__complemento">{s.complemento}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}

      <form className="rota__busca" onSubmit={tracar}>
        <input
          className="rota__campo"
          value={destino}
          onChange={(e) => setDestino(e.target.value)}
          placeholder={erro ?? "para onde?"}
          aria-label="Destino"
        />
        <button
          className="rota__ir"
          type="submit"
          // Sem posição não há de onde partir, e o Rust ignoraria o pedido em
          // silêncio — melhor o botão dizer isso antes do toque.
          disabled={!destino.trim() || buscando || !fix}
        >
          {buscando ? "…" : "ir"}
        </button>
      </form>
    </div>
  );
}
