import { lazy } from "react";
import { Navigation } from "lucide-react";

import { defineTile, type AnyTileSpec } from "../../core/types";
import type { MapaState } from "./tipos";

const NAV = "nav";

/**
 * O tile é só o metadado; o mapa de verdade (`mapa.tsx`) chega por chunk
 * separado. É o que tira o wrapper do Google Maps — e a ida ao CDN do próprio
 * Maps, que o `APIProvider` dispara ao montar — do caminho do primeiro paint.
 */
const Mapa = lazy(() => import("./mapa").then((m) => ({ default: m.Mapa })));
const MapaCheio = lazy(() =>
  import("./mapa").then((m) => ({ default: m.MapaCheio })),
);

export { useLocalizacaoReal } from "./localizacao";

export const navTile: AnyTileSpec = defineTile<MapaState>({
  id: "mapa",
  module: NAV,
  title: "Navegação",
  area: "maps",
  icon: <Navigation size="1em" />,
  // O mapa É a imagem que ele mostra: título e borda em volta só o fazem
  // parecer menor do que é.
  chrome: "nu",
  Compact: Mapa,
  Expanded: MapaCheio,
});
