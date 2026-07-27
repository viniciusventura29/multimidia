import { useEffect, useRef, useState } from "react";
import { APIProvider, Map, useMap } from "@vis.gl/react-google-maps";

import { defineTile, type AnyTileSpec, type TileView } from "../core/types";

const NAV = "nav";

/** Enquanto o GPS não fixa, o mapa nasce olhando para São Paulo. */
const CENTRO_PADRAO = { lat: -23.5505, lng: -46.6333 };

/** Quantos pontos do caminho já andado manter desenhados. */
const RASTRO_MAXIMO = 120;

interface Fix {
  lat: number;
  lon: number;
  heading: number;
  speedKmh: number;
}

interface MapaState {
  apiKey: string;
  mapId: string | null;
  fix: Fix | null;
}

/**
 * Faz o mapa seguir o carro.
 *
 * Fica num componente separado porque só quem está dentro do `APIProvider`
 * consegue pegar a instância do mapa com `useMap`.
 *
 * `heading` e `tilt` só têm efeito em mapa **vetorial**, que exige um Map ID
 * configurado como tal. Sem Map ID o mapa segue o carro, mas fica chapado e
 * olhando para o norte — que é o que diferencia "um mapa" de "modo navegação".
 */
function SeguirCarro({ fix, navegando }: { fix: Fix | null; navegando: boolean }) {
  const map = useMap();
  const rastro = useRef<google.maps.Polyline | null>(null);
  const pontos = useRef<google.maps.LatLngLiteral[]>([]);

  useEffect(() => {
    if (!map || !fix) return;

    const posicao = { lat: fix.lat, lng: fix.lon };

    // `moveCamera` move os três de uma vez, sem disparar três animações que
    // brigariam entre si a cada leitura de GPS.
    map.moveCamera({
      center: posicao,
      ...(navegando ? { heading: fix.heading, tilt: 60, zoom: 17 } : {}),
    });

    pontos.current = [...pontos.current, posicao].slice(-RASTRO_MAXIMO);

    if (!rastro.current) {
      rastro.current = new google.maps.Polyline({
        map,
        strokeColor: "#a06bff",
        strokeOpacity: 0.9,
        strokeWeight: 5,
      });
    }
    rastro.current.setPath(pontos.current);
  }, [map, fix, navegando]);

  useEffect(() => () => rastro.current?.setMap(null), []);

  return null;
}

/**
 * O mapa.
 *
 * Como a UI roda num WebView, ele é um elemento comum da página: o mesmo
 * componente serve de widget e de tela cheia, e a transição é só o CSS mudando
 * de tamanho. Era exatamente isso que o SDK nativo do Android não permitiria.
 *
 * O que **não** dá para fazer aqui: navegação turn-by-turn. O Maps SDK entrega
 * mapa, não navegação — quem entrega é o Navigation SDK, que é enterprise.
 * Guiar de verdade é abrir o app do Google Maps por cima.
 */
function Mapa({ data, status }: TileView<MapaState>) {
  const [navegando, setNavegando] = useState(true);

  if (!data?.apiKey) {
    return (
      <div className="mapa">
        <span className="mapa__marca">mapa</span>
      </div>
    );
  }

  const modoNavegacao = navegando && Boolean(data.mapId);

  return (
    <div className={`mapa mapa--vivo${status === "degraded" ? " mapa--sem-sinal" : ""}`}>
      <APIProvider apiKey={data.apiKey}>
        <Map
          className="mapa__canvas"
          defaultCenter={CENTRO_PADRAO}
          defaultZoom={16}
          mapId={data.mapId ?? undefined}
          colorScheme="DARK"
          disableDefaultUI
          gestureHandling="greedy"
          reuseMaps
        />
        <SeguirCarro fix={data.fix} navegando={modoNavegacao} />
      </APIProvider>

      {data.fix && (
        <div className="mapa__velocidade">
          <strong>{Math.round(data.fix.speedKmh)}</strong> km/h
        </div>
      )}

      {data.mapId ? (
        <button
          className="mapa__modo"
          onClick={(e) => {
            e.stopPropagation();
            setNavegando((v) => !v);
          }}
        >
          {navegando ? "ver de cima" : "seguir"}
        </button>
      ) : (
        // Sem Map ID não há como inclinar nem girar. Dizer isso é melhor que
        // deixar o usuário achar que o modo navegação está quebrado.
        <span className="mapa__aviso">sem Map ID vetorial — mapa chapado</span>
      )}
    </div>
  );
}

export const navTile: AnyTileSpec = defineTile<MapaState>({
  id: "mapa",
  module: NAV,
  title: "Navegação",
  area: "mapa",
  Compact: (view) => <Mapa {...view} />,
  Expanded: (view) => <Mapa {...view} />,
});
