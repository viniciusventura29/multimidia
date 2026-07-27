import { APIProvider, Map } from "@vis.gl/react-google-maps";

import { defineTile, type AnyTileSpec, type TileView } from "../core/types";

const NAV = "nav";

/** Enquanto não há GPS, o mapa nasce olhando para São Paulo. */
const CENTRO_PADRAO = { lat: -23.5505, lng: -46.6333 };

interface MapaState {
  apiKey: string;
}

/**
 * O mapa.
 *
 * Como a UI roda num WebView, ele é um elemento comum da página: o mesmo
 * componente serve de widget e de tela cheia, e a transição é só o CSS mudando
 * de tamanho. Era exatamente isso que o SDK nativo do Android não permitiria.
 *
 * O que **não** dá para fazer aqui: navegação turn-by-turn. O Maps SDK entrega
 * mapa, não navegação — quem entrega é o Navigation SDK, que é enterprise. Guiar
 * de verdade é abrir o app do Google Maps por cima.
 */
function Mapa({ data, status }: TileView<MapaState>) {
  if (status !== "ready" || !data?.apiKey) {
    // Só a marca d'água aqui: explicar o motivo é trabalho do rodapé do tile,
    // e repetir o texto nos dois lugares polui o quadro maior da tela.
    return (
      <div className="mapa">
        <span className="mapa__marca">mapa</span>
      </div>
    );
  }

  return (
    <div className="mapa mapa--vivo">
      <APIProvider apiKey={data.apiKey}>
        <Map
          defaultCenter={CENTRO_PADRAO}
          defaultZoom={13}
          colorScheme="DARK"
          disableDefaultUI
          gestureHandling="greedy"
          reuseMaps
        />
      </APIProvider>
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
