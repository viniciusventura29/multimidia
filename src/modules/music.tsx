import { useState, type MouseEvent } from "react";

import { conectarSpotify, dispatchAction } from "../core/actions";
import {
  defineTile,
  type AnyTileSpec,
  type NowPlaying,
  type TileView,
} from "../core/types";

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

/**
 * O caminho de reconexão.
 *
 * O Spotify expira o refresh token em 6 meses, contados do login original — não
 * dá para evitar. Então reconectar é um estado previsto do painel, com um toque
 * só, e não uma tela de erro.
 */
function Conectar() {
  const [conectando, setConectando] = useState(false);

  const abrir = async (event: MouseEvent) => {
    event.stopPropagation();
    setConectando(true);
    try {
      await conectarSpotify();
    } catch (err) {
      console.error("[eclipse] não consegui conectar o Spotify", err);
    } finally {
      setConectando(false);
    }
  };

  return (
    <button className="musica__conectar" onClick={abrir} disabled={conectando}>
      {conectando ? "aguardando o navegador…" : "conectar Spotify"}
    </button>
  );
}

function Faixa({
  data,
  status,
  reason,
  grande,
}: TileView<NowPlaying> & { grande?: boolean }) {
  // Degradado sem nada guardado quer dizer que nunca conectou, ou que a sessão
  // caiu de vez: oferecer o caminho de volta é mais útil que repetir o erro.
  const precisaConectar =
    status === "degraded" &&
    (!data || /reconectar|conectou|Client ID/i.test(reason ?? ""));

  if (precisaConectar) {
    return (
      <div className="musica">
        <Conectar />
      </div>
    );
  }

  if (!data) return <p className="musica__vazio">nada tocando</p>;

  return (
    <div className={`musica${grande ? " musica--grande" : ""}`}>
      {grande && data.albumArt && (
        <img className="musica__capa" src={data.albumArt} alt="" />
      )}
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
  Compact: (view) => <Faixa {...view} />,
  Expanded: (view) => <Faixa {...view} grande />,
});
