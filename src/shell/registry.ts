import { assistenteTile } from "../modules/assistente";
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
 *
 * O WhatsApp não está aqui de propósito: deixou de ser um quadro e virou
 * notificação que sobe da base (ver `shell/Notificacoes.tsx`). O módulo Rust
 * continua rodando; só a apresentação mudou.
 */
export const TILES: AnyTileSpec[] = [
  navTile,
  musicTile,
  assistenteTile,
  ...obdTiles,
];
