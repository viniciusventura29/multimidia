import { defineTile, type AnyTileSpec } from "../core/types";

const NAV = "nav";

/**
 * O mapa.
 *
 * Hoje é um vazio honesto: o módulo `nav` se declara degradado porque não há
 * chave do Google Maps configurada. A Fase 6 põe o mapa de verdade aqui dentro —
 * e como a UI roda num WebView, ele é um elemento comum da página, que encolhe
 * pro widget e cresce pra tela cheia sem truque nenhum.
 */
function Mapa({ grande }: { grande?: boolean }) {
  return (
    <div className={`mapa${grande ? " mapa--grande" : ""}`}>
      <span className="mapa__marca">mapa</span>
    </div>
  );
}

export const navTile: AnyTileSpec = defineTile<unknown>({
  id: "mapa",
  module: NAV,
  title: "Navegação",
  area: "mapa",
  Compact: () => <Mapa />,
  Expanded: () => <Mapa grande />,
});
