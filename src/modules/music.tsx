import type { MouseEvent } from "react";

import { dispatchAction } from "../core/actions";
import { defineTile, type AnyTileSpec, type NowPlaying } from "../core/types";

const MUSIC = "music";

function Controles({ tocando }: { tocando: boolean }) {
  // O tile inteiro expande ao toque, então os botões precisam segurar o clique
  // deles — senão apertar "play" abriria a tela cheia junto.
  const acionar = (event: MouseEvent, acao: string) => {
    event.stopPropagation();
    dispatchAction(MUSIC, { acao });
  };

  return (
    <div className="controles">
      <button
        className="controles__botao"
        onClick={(e) => acionar(e, "prev")}
        aria-label="Faixa anterior"
      >
        ⏮
      </button>
      <button
        className="controles__botao controles__botao--principal"
        onClick={(e) => acionar(e, "toggle")}
        aria-label={tocando ? "Pausar" : "Tocar"}
      >
        {tocando ? "⏸" : "▶"}
      </button>
      <button
        className="controles__botao"
        onClick={(e) => acionar(e, "next")}
        aria-label="Próxima faixa"
      >
        ⏭
      </button>
    </div>
  );
}

function Faixa({ data, grande }: { data: NowPlaying | null; grande?: boolean }) {
  if (!data) return <p className="musica__vazio">nada tocando</p>;

  return (
    <div className={`musica${grande ? " musica--grande" : ""}`}>
      <p className="musica__track">{data.track}</p>
      <p className="musica__artist">{data.artist}</p>
      <Controles tocando={data.isPlaying} />
    </div>
  );
}

export const musicTile: AnyTileSpec = defineTile<NowPlaying>({
  id: "musica",
  module: MUSIC,
  title: "Spotify",
  area: "musica",
  Compact: ({ data }) => <Faixa data={data} />,
  Expanded: ({ data }) => <Faixa data={data} grande />,
});
