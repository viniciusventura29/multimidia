import { assistenteTile } from "../modules/assistente";
import { messagingTile } from "../modules/messaging";
import { musicTile } from "../modules/music";
import { navTile } from "../modules/nav";
import { obdTiles } from "../modules/obd";
import type { AnyTileSpec } from "../core/types";

/**
 * Tudo que aparece no painel.
 *
 * A ordem aqui não define o layout — cada tile diz em que `area` do grid mora,
 * e o desenho fica no CSS. Acrescentar um módulo é acrescentar uma entrada aqui
 * e uma área lá.
 */
export const TILES: AnyTileSpec[] = [
  navTile,
  messagingTile,
  musicTile,
  assistenteTile,
  ...obdTiles,
];
