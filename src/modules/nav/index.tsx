import { useEffect, useRef, useState } from "react";
import { APIProvider, Map, useMap } from "@vis.gl/react-google-maps";

import { defineTile, type AnyTileSpec, type TileView } from "../../core/types";
import { Destino } from "./destino";
import type { Fix, MapaState } from "./tipos";

const NAV = "nav";

/** Enquanto o GPS não fixa, o mapa nasce olhando para São Paulo. */
const CENTRO_PADRAO = { lat: -23.5505, lng: -46.6333 };

/** Quantos pontos do caminho já andado manter desenhados. */
const RASTRO_MAXIMO = 120;

/** De quanto em quanto tempo chega uma posição nova. */
const INTERVALO_GPS_MS = 1000;

/**
 * Interpola dois rumos pelo caminho mais curto.
 *
 * Sem isto, ir de 359° para 1° faria o mapa girar 358° para trás em vez de 2°
 * para a frente — o carro daria um pião na tela toda vez que cruzasse o norte.
 */
function interpolarRumo(de: number, para: number, t: number): number {
  const delta = ((para - de + 540) % 360) - 180;
  return (de + delta * t + 360) % 360;
}

/**
 * Faz o mapa seguir o carro.
 *
 * O GPS entrega **uma posição por segundo**. Mandar a câmera direto para cada
 * uma faz o mapa teleportar 1x por segundo — era isso que dava a sensação de
 * travado. Aqui a câmera é redesenhada a cada quadro, interpolando entre a
 * leitura anterior e a atual. É o que todo navegador faz, e é a diferença entre
 * "um mapa que atualiza" e "um mapa que anda".
 *
 * Fica num componente separado porque só quem está dentro do `APIProvider`
 * consegue pegar a instância do mapa com `useMap`.
 *
 * `heading` e `tilt` só têm efeito em mapa **vetorial**, que exige um Map ID
 * configurado como tal.
 */
function SeguirCarro({ fix, navegando }: { fix: Fix | null; navegando: boolean }) {
  const map = useMap();
  const trecho = useRef<{ de: Fix; para: Fix; inicio: number } | null>(null);
  const rastro = useRef<google.maps.Polyline | null>(null);
  const pontos = useRef<google.maps.LatLngLiteral[]>([]);

  // Cada leitura abre um trecho novo a ser percorrido até a próxima chegar.
  useEffect(() => {
    if (!fix) return;

    trecho.current = {
      de: trecho.current?.para ?? fix,
      para: fix,
      inicio: performance.now(),
    };

    pontos.current = [...pontos.current, { lat: fix.lat, lng: fix.lon }].slice(
      -RASTRO_MAXIMO,
    );
    rastro.current?.setPath(pontos.current);
  }, [fix]);

  useEffect(() => {
    if (!map) return;

    rastro.current ??= new google.maps.Polyline({
      map,
      strokeColor: "#a06bff",
      strokeOpacity: 0.9,
      strokeWeight: 6,
    });

    let quadro = 0;
    const desenhar = () => {
      const atual = trecho.current;
      if (atual) {
        // Trava em 1 quando a próxima leitura atrasa. Deixar passar continuaria
        // extrapolando o carro para longe do que se sabe; parar e esperar é
        // honesto — é exatamente o que o aparelho conhece.
        const t = Math.min(1, (performance.now() - atual.inicio) / INTERVALO_GPS_MS);

        map.moveCamera({
          center: {
            lat: atual.de.lat + (atual.para.lat - atual.de.lat) * t,
            lng: atual.de.lon + (atual.para.lon - atual.de.lon) * t,
          },
          ...(navegando
            ? {
                heading: interpolarRumo(atual.de.heading, atual.para.heading, t),
                tilt: 62,
                zoom: 18,
              }
            : {}),
        });
      }
      quadro = requestAnimationFrame(desenhar);
    };

    quadro = requestAnimationFrame(desenhar);
    return () => cancelAnimationFrame(quadro);
  }, [map, navegando]);

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
      <APIProvider apiKey={data.apiKey} language="pt-BR" region="BR">
        <Map
          className="mapa__canvas"
          defaultCenter={CENTRO_PADRAO}
          defaultZoom={18}
          mapId={data.mapId ?? undefined}
          colorScheme="DARK"
          disableDefaultUI
          gestureHandling="greedy"
          reuseMaps
        />
        <SeguirCarro fix={data.fix} navegando={modoNavegacao} />
      </APIProvider>

      <Destino />

      {/* Em modo navegação o carro fica parado no centro e o mundo gira em
          volta. Um marcador que se move seria redundante — e erraria, porque a
          câmera é que está sendo interpolada. */}
      {modoNavegacao && data.fix && <span className="mapa__carro" aria-hidden />}

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
