import { useEffect, useState, type MouseEvent } from "react";
import { ChevronLeft, Disc3, ListMusic, Music, Pause, Play, SkipBack, SkipForward } from "lucide-react";

import { conectarSpotify, dispatchAction } from "../core/actions";
import {
  defineTile,
  type AnyTileSpec,
  type MusicState,
  type TileView,
} from "../core/types";
import {
  ativarAudio,
  controlarLocal,
  marcarTocandoOtimista,
  useEstadoLocal,
  useStatusPlayer,
} from "./spotifyPlayer";

const MUSIC = "music";

/**
 * Dispara uma ação de transporte.
 *
 * Tenta primeiro o SDK local: quando o som sai pelo Eclipse (o caso normal),
 * pausar é uma chamada em memória e o estado volta por evento — instantâneo. Só
 * cai no Rust (Web API, com ida e volta de rede) quando o som está em outro
 * aparelho. Antes tudo ia pelo Rust, e o toque somava DUAS viagens de internet
 * antes de a tela mudar: era o delay que se sentia.
 */
async function transporte(acao: string, tocandoAgora: boolean): Promise<void> {
  await ativarAudio();
  // Otimista: a tela reflete o toque na hora e o evento do SDK confirma depois.
  if (acao === "toggle") marcarTocandoOtimista(!tocandoAgora);
  if (await controlarLocal(acao)) return;
  dispatchAction(MUSIC, { acao });
}

function Controles({ tocando, grande }: { tocando: boolean; grande?: boolean }) {
  // O tile compacto expande ao toque, então os botões seguram o clique deles —
  // senão apertar "play" abriria a tela cheia junto.
  const acionar = (event: MouseEvent, acao: string) => {
    event.stopPropagation();
    void transporte(acao, tocando);
  };

  return (
    <div className={`controles${grande ? " controles--grande" : ""}`}>
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

/** Uma linha da lista: capa, título, subtítulo. Serve faixa, álbum e playlist. */
function Linha({
  capa,
  titulo,
  subtitulo,
  icone,
  onClick,
}: {
  capa: string | null;
  titulo: string;
  subtitulo: string;
  icone?: React.ReactNode;
  onClick: (e: MouseEvent) => void;
}) {
  return (
    <button className="sp-linha" onClick={onClick}>
      {capa ? (
        <img className="sp-linha__capa" src={capa} alt="" loading="lazy" />
      ) : (
        <span className="sp-linha__capa sp-linha__capa--vazia">{icone ?? <Music size="1em" />}</span>
      )}
      <span className="sp-linha__texto">
        <span className="sp-linha__titulo">{titulo}</span>
        <span className="sp-linha__sub">{subtitulo}</span>
      </span>
    </button>
  );
}

/**
 * O que toca agora + controles, fixo no rodapé da tela cheia.
 *
 * Prefere o estado do SDK ao do Rust: o do SDK chega no instante da mudança, o
 * do Rust só no próximo poll (até 3s depois).
 */
function BarraTocando({ data }: { data: MusicState | null }) {
  const local = useEstadoLocal();
  const np = local ?? data?.nowPlaying ?? null;
  if (!np) return null;

  return (
    <div className="sp-tocando">
      {np.albumArt && <img className="sp-tocando__capa" src={np.albumArt} alt="" />}
      <span className="sp-linha__texto">
        <span className="sp-linha__titulo">{np.track}</span>
        <span className="sp-linha__sub">{np.artist}</span>
      </span>
      <Controles tocando={np.isPlaying} grande />
    </div>
  );
}

/** Diz se o Eclipse já virou device do Spotify — sem isto "não toca" é mudo. */
function StatusDevice() {
  const status = useStatusPlayer();
  const texto = {
    off: null,
    carregando: "preparando o player…",
    pronto: null, // pronto é o normal: não precisa avisar
    erro: "player indisponível — precisa de Spotify Premium",
  }[status];

  if (!texto) return null;
  return <span className={`sp-device sp-device--${status}`}>{texto}</span>;
}

/** Compacto: o que toca + controles, ou o caminho para resolver o problema. */
function Compacto({ data }: TileView<MusicState>) {
  const local = useEstadoLocal();
  const problema = data?.problema ?? null;

  if (problema?.tipo === "precisaLogin") {
    return (
      <div className="musica">
        <Conectar />
      </div>
    );
  }

  const np = local ?? data?.nowPlaying ?? null;
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

/** Tela cheia: busca, playlists, e entrar em playlist/álbum para escolher faixa. */
function Completa({ data }: TileView<MusicState>) {
  const [termo, setTermo] = useState("");
  const problema = data?.problema ?? null;
  const precisaLogin = problema?.tipo === "precisaLogin";

  // Ao abrir a tela cheia, já carrega as playlists do usuário.
  useEffect(() => {
    if (!precisaLogin) dispatchAction(MUSIC, { acao: "playlists" });
  }, [precisaLogin]);

  if (precisaLogin) {
    return (
      <div className="musica">
        <Conectar />
        {problema && <p className="tile__reason">{problema.detalhe}</p>}
      </div>
    );
  }

  const busca = data?.busca ?? { faixas: [], albuns: [] };
  const playlists = data?.playlists ?? [];
  const contexto = data?.contexto ?? null;
  const temBusca = busca.faixas.length > 0 || busca.albuns.length > 0;

  const submeter = (event: React.FormEvent) => {
    event.preventDefault();
    const q = termo.trim();
    if (q) dispatchAction(MUSIC, { acao: "buscar", termo: q });
  };

  const tocar = (event: MouseEvent, uri: string) => {
    event.stopPropagation();
    void ativarAudio();
    dispatchAction(MUSIC, { acao: "tocar", uri });
  };

  const abrir = (event: MouseEvent, uri: string) => {
    event.stopPropagation();
    dispatchAction(MUSIC, { acao: "abrir", uri });
  };

  const voltar = (event: MouseEvent) => {
    event.stopPropagation();
    if (contexto) dispatchAction(MUSIC, { acao: "fechar" });
    else {
      setTermo("");
      dispatchAction(MUSIC, { acao: "limpar_busca" });
    }
  };

  return (
    <div className="sp" onClick={(e) => e.stopPropagation()}>
      {/* Cabeçalho: busca, ou o contexto aberto com voltar. */}
      {contexto ? (
        <header className="sp-topo">
          <button className="sp-voltar" onClick={voltar} aria-label="Voltar">
            <ChevronLeft size="1.2em" />
          </button>
          {contexto.albumArt && <img className="sp-topo__capa" src={contexto.albumArt} alt="" />}
          <span className="sp-linha__texto">
            <span className="sp-topo__nome">{contexto.nome}</span>
            <span className="sp-linha__sub">
              {contexto.subtitulo} · {contexto.faixas.length} faixas
            </span>
          </span>
          <button className="sp-tocar-tudo" onClick={(e) => tocar(e, contexto.uri)}>
            <Play size="1em" fill="currentColor" /> tocar tudo
          </button>
        </header>
      ) : (
        <form className="sp-busca" onSubmit={submeter}>
          {temBusca && (
            <button className="sp-voltar" onClick={voltar} type="button" aria-label="Limpar busca">
              <ChevronLeft size="1.2em" />
            </button>
          )}
          <input
            className="sp-busca__campo"
            value={termo}
            onChange={(e) => setTermo(e.target.value)}
            placeholder="buscar música, artista ou álbum…"
            aria-label="Buscar no Spotify"
          />
          <button className="sp-busca__botao" type="submit">
            buscar
          </button>
        </form>
      )}

      <StatusDevice />
      {problema && problema.tipo !== "semDispositivo" && (
        <p className="tile__reason">{problema.detalhe}</p>
      )}

      <div className="sp-lista">
        {contexto ? (
          // Dentro de uma playlist/álbum: escolher a faixa.
          contexto.faixas.map((f, i) => (
            <Linha
              key={`${f.uri}-${i}`}
              capa={f.albumArt}
              titulo={f.track}
              subtitulo={f.artist}
              onClick={(e) => tocar(e, f.uri)}
            />
          ))
        ) : temBusca ? (
          <>
            {busca.albuns.length > 0 && <p className="sp-secao">Álbuns</p>}
            {busca.albuns.map((a) => (
              <Linha
                key={a.uri}
                capa={a.albumArt}
                titulo={a.nome}
                subtitulo={a.artist}
                icone={<Disc3 size="1em" />}
                onClick={(e) => abrir(e, a.uri)}
              />
            ))}
            {busca.faixas.length > 0 && <p className="sp-secao">Músicas</p>}
            {busca.faixas.map((f) => (
              <Linha
                key={f.uri}
                capa={f.albumArt}
                titulo={f.track}
                subtitulo={f.artist}
                onClick={(e) => tocar(e, f.uri)}
              />
            ))}
          </>
        ) : (
          <>
            {playlists.length > 0 && <p className="sp-secao">Suas playlists</p>}
            {playlists.map((p) => (
              <Linha
                key={p.uri}
                capa={p.albumArt}
                titulo={p.nome}
                subtitulo="playlist"
                icone={<ListMusic size="1em" />}
                onClick={(e) => abrir(e, p.uri)}
              />
            ))}
            {playlists.length === 0 && (
              <p className="musica__vazio">busque uma música acima 👆</p>
            )}
          </>
        )}
      </div>

      <BarraTocando data={data} />
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
