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

/**
 * Só estes três motivos se resolvem com login. "Nenhum dispositivo ativo" não:
 * ali a sessão está de pé e oferecer "conectar" mandaria o usuário refazer um
 * login que já funcionou.
 *
 * Casar com o texto é frágil — o certo seria o Rust mandar a natureza da falha
 * junto do motivo, e não a tela adivinhar pela prosa.
 */
const PRECISA_LOGIN = /reconectar|conectou o Spotify|Client ID/i;

function Faixa({
  data,
  status,
  reason,
  grande,
}: TileView<NowPlaying> & { grande?: boolean }) {
  if (status === "degraded" && PRECISA_LOGIN.test(reason ?? "")) {
    return (
      <div className="musica">
        <Conectar />
      </div>
    );
  }

  if (!data) {
    // Conectado, mas não há o que controlar: a Web API comanda um device que já
    // esteja tocando, ela não cria um.
    return (
      <p className="musica__vazio">
        abra o Spotify em algum aparelho e dê play
      </p>
    );
  }

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
