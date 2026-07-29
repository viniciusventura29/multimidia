import { useEffect, useState, type MouseEvent } from "react";
import { Music, Pause, Play, SkipBack, SkipForward } from "lucide-react";

import { conectarSpotify, dispatchAction } from "../core/actions";
import { ativarAudio, useStatusPlayer } from "./spotifyPlayer";
import {
  defineTile,
  type AnyTileSpec,
  type MusicState,
  type TileView,
} from "../core/types";

const MUSIC = "music";

function Controles({ tocando }: { tocando: boolean }) {
  // O tile compacto expande ao toque, então os botões seguram o clique deles —
  // senão apertar "play" abriria a tela cheia junto.
  const acionar = (event: MouseEvent, acao: string) => {
    event.stopPropagation();
    void ativarAudio();
    dispatchAction(MUSIC, { acao });
  };

  return (
    <div className="controles">
      <button className="controles__botao" onClick={(e) => acionar(e, "prev")} aria-label="Faixa anterior">
        <SkipBack size="1em" fill="currentColor" />
      </button>
      <button
        className="controles__botao controles__botao--principal"
        onClick={(e) => acionar(e, "toggle")}
        aria-label={tocando ? "Pausar" : "Tocar"}
      >
        {tocando ? <Pause size="1em" fill="currentColor" /> : <Play size="1em" fill="currentColor" />}
      </button>
      <button className="controles__botao" onClick={(e) => acionar(e, "next")} aria-label="Próxima faixa">
        <SkipForward size="1em" fill="currentColor" />
      </button>
    </div>
  );
}

/**
 * O login (e a reconexão).
 *
 * O Spotify expira o refresh token em 6 meses, contados do login original — não
 * dá para evitar. Então (re)conectar é um estado previsto do painel, com um
 * toque só, e não uma tela de erro.
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
 * Precisa logar? "reconectar"/"conectou o Spotify"/"Client ID" são os motivos
 * que se resolvem com login. Casar por texto é frágil, mas é o que o barramento
 * entrega hoje (o motivo é só uma frase).
 */
const PRECISA_LOGIN = /reconectar|conectou o Spotify|Client ID/i;

function precisaLogin(status: string, reason: string | null): boolean {
  return status === "degraded" && PRECISA_LOGIN.test(reason ?? "");
}

/** Compacto: o que toca + controles, ou o botão de conectar. */
function Compacto({ data, status, reason }: TileView<MusicState>) {
  if (precisaLogin(status, reason)) {
    return (
      <div className="musica">
        <Conectar />
      </div>
    );
  }

  const np = data?.nowPlaying ?? null;
  if (!np) {
    return <p className="musica__vazio">toque para buscar e tocar</p>;
  }

  return (
    <div className="musica">
      {np.albumArt && <img className="musica__capa" src={np.albumArt} alt="" />}
      <p className="musica__track">{np.track}</p>
      <p className="musica__artist">{np.artist}</p>
      <Controles tocando={np.isPlaying} />
    </div>
  );
}

/** Tela cheia: busca, resultados, playlists e o que toca. É o Caminho A. */
/** Diz se o Eclipse já virou device do Spotify — sem isto "não toca" é mudo. */
function StatusDevice() {
  const status = useStatusPlayer();
  const texto = {
    off: null,
    carregando: "preparando o player…",
    pronto: "tocando pelo Eclipse",
    erro: "player indisponível (precisa de Premium)",
  }[status];

  if (!texto) return null;
  return <span className={`spotify-device spotify-device--${status}`}>{texto}</span>;
}

function Completa({ data, status, reason }: TileView<MusicState>) {
  const [termo, setTermo] = useState("");
  const logado = !precisaLogin(status, reason);

  // Ao abrir a tela cheia, já carrega as playlists do usuário.
  useEffect(() => {
    if (logado) dispatchAction(MUSIC, { acao: "playlists" });
  }, [logado]);

  if (!logado) {
    return (
      <div className="musica">
        <Conectar />
      </div>
    );
  }

  const submeter = (event: React.FormEvent) => {
    event.preventDefault();
    const q = termo.trim();
    if (q) dispatchAction(MUSIC, { acao: "buscar", termo: q });
  };

  // `ativarAudio` antes de mandar tocar: em navegador mobile o áudio só toca se
  // liberado dentro de um gesto do usuário. Sem isto a faixa é transferida para
  // o Eclipse mas fica pausada.
  const tocarFaixa = (event: MouseEvent, uri: string) => {
    event.stopPropagation();
    void ativarAudio();
    dispatchAction(MUSIC, { acao: "tocar", uri });
  };

  const tocarPlaylist = (event: MouseEvent, uri: string) => {
    event.stopPropagation();
    void ativarAudio();
    dispatchAction(MUSIC, { acao: "tocar_playlist", uri });
  };

  const np = data?.nowPlaying ?? null;
  const resultados = data?.resultados ?? [];
  const playlists = data?.playlists ?? [];
  // Sem busca ativa, mostra as playlists; com resultados, mostra a busca.
  const mostrarPlaylists = resultados.length === 0;

  return (
    <div className="spotify-full" onClick={(e) => e.stopPropagation()}>
      <form className="spotify-busca" onSubmit={submeter}>
        <input
          className="spotify-busca__campo"
          value={termo}
          onChange={(e) => setTermo(e.target.value)}
          placeholder="buscar música ou artista…"
          aria-label="Buscar no Spotify"
        />
        <button className="spotify-busca__botao" type="submit">
          buscar
        </button>
      </form>

      <StatusDevice />

      <div className="spotify-lista">
        {mostrarPlaylists
          ? playlists.map((p) => (
              <button key={p.uri} className="spotify-item" onClick={(e) => tocarPlaylist(e, p.uri)}>
                {p.albumArt && <img className="spotify-item__capa" src={p.albumArt} alt="" />}
                <span className="spotify-item__texto">
                  <span className="spotify-item__track">{p.nome}</span>
                  <span className="spotify-item__artist">playlist</span>
                </span>
              </button>
            ))
          : resultados.map((f) => (
              <button key={f.uri} className="spotify-item" onClick={(e) => tocarFaixa(e, f.uri)}>
                {f.albumArt && <img className="spotify-item__capa" src={f.albumArt} alt="" />}
                <span className="spotify-item__texto">
                  <span className="spotify-item__track">{f.track}</span>
                  <span className="spotify-item__artist">{f.artist}</span>
                </span>
              </button>
            ))}
        {mostrarPlaylists && playlists.length === 0 && (
          <p className="musica__vazio">busque uma música acima 👆</p>
        )}
      </div>

      {np && (
        <div className="spotify-tocando">
          <span className="spotify-item__texto">
            <span className="musica__track">{np.track}</span>
            <span className="musica__artist">{np.artist}</span>
          </span>
          <Controles tocando={np.isPlaying} />
        </div>
      )}
    </div>
  );
}

export const musicTile: AnyTileSpec = defineTile<MusicState>({
  id: "musica",
  module: MUSIC,
  title: "Spotify",
  area: "spotify",
  icon: <Music size="1em" />,
  Compact: (view) => <Compacto {...view} />,
  Expanded: (view) => <Completa {...view} />,
});
