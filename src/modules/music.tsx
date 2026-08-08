import { useEffect, useRef, useState, type MouseEvent, type PointerEvent } from "react";
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
  marcarPosicaoOtimista,
  marcarTocandoOtimista,
  useEstadoLocal,
  type EstadoLocal,
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
async function transporte(acao: string, tocandoAgora: boolean, arg?: number): Promise<void> {
  await ativarAudio();
  // Otimista: a tela reflete o toque na hora e o evento do SDK confirma depois.
  if (acao === "toggle") marcarTocandoOtimista(!tocandoAgora);
  if (acao === "seek" && arg != null) marcarPosicaoOtimista(arg);
  if (await controlarLocal(acao, arg)) return;
  dispatchAction(MUSIC, acao === "seek" ? { acao, posicaoMs: arg } : { acao });
}

/** "m:ss" a partir de milissegundos. */
function formatarTempo(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const minutos = Math.floor(total / 60);
  const segundos = total % 60;
  return `${minutos}:${segundos.toString().padStart(2, "0")}`;
}

/**
 * Posição/duração da faixa, normalizadas: o SDK local usa `positionMs`, o estado
 * do Rust usa `progressMs` — a barra não precisa saber de onde veio.
 */
function progresso(
  local: EstadoLocal | null,
  data: MusicState | null,
): { posicaoMs: number | null; duracaoMs: number | null } {
  if (local) return { posicaoMs: local.positionMs, duracaoMs: local.durationMs };
  const np = data?.nowPlaying;
  return { posicaoMs: np?.progressMs ?? null, duracaoMs: np?.durationMs ?? null };
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

/**
 * A linha de progresso: mostra o andamento e deixa arrastar para pular (seek).
 *
 * O evento do SDK só chega quando algo muda, não a cada segundo — então a barra
 * interpola sozinha enquanto toca, e reancora quando um estado novo chega (do SDK
 * ou do poll do Rust). Enquanto o dedo arrasta, para de interpolar para não brigar
 * com o toque.
 */
function ProgressoTocando({
  posicaoMs,
  duracaoMs,
  tocando,
  onSeek,
}: {
  posicaoMs: number | null;
  duracaoMs: number | null;
  tocando: boolean;
  onSeek: (ms: number) => void;
}) {
  const [pos, setPos] = useState(posicaoMs ?? 0);
  const trilhoRef = useRef<HTMLDivElement>(null);
  const arrastando = useRef(false);

  // Reancora quando chega posição nova — a não ser que o dedo esteja arrastando.
  useEffect(() => {
    if (!arrastando.current) setPos(posicaoMs ?? 0);
  }, [posicaoMs]);

  // Interpola por segundo enquanto toca; zera o timer quando o estado muda.
  // 1 s basta: o texto só mostra segundos, e o deslizar visual da barra quem
  // faz é a transition do CSS — de graça, no compositor, sem acordar o React.
  useEffect(() => {
    if (!tocando || duracaoMs == null) return;
    const base = posicaoMs ?? 0;
    const inicio = Date.now();
    const id = setInterval(() => {
      if (arrastando.current) return;
      setPos(Math.min(base + (Date.now() - inicio), duracaoMs));
    }, 1000);
    return () => clearInterval(id);
  }, [tocando, posicaoMs, duracaoMs]);

  if (duracaoMs == null || duracaoMs <= 0) return null;

  const frac = Math.max(0, Math.min(1, pos / duracaoMs));

  const posDeEvento = (clientX: number): number => {
    const el = trilhoRef.current;
    if (!el) return pos;
    const r = el.getBoundingClientRect();
    const f = r.width > 0 ? Math.max(0, Math.min(1, (clientX - r.left) / r.width)) : 0;
    return Math.round(f * duracaoMs);
  };

  const aoApertar = (e: PointerEvent<HTMLDivElement>) => {
    e.stopPropagation();
    arrastando.current = true;
    e.currentTarget.setPointerCapture(e.pointerId);
    setPos(posDeEvento(e.clientX));
  };
  const aoMover = (e: PointerEvent<HTMLDivElement>) => {
    if (!arrastando.current) return;
    e.stopPropagation();
    setPos(posDeEvento(e.clientX));
  };
  const aoSoltar = (e: PointerEvent<HTMLDivElement>) => {
    if (!arrastando.current) return;
    e.stopPropagation();
    arrastando.current = false;
    const ms = posDeEvento(e.clientX);
    setPos(ms);
    onSeek(ms);
  };

  return (
    <div className="progresso">
      <span className="progresso__tempo">{formatarTempo(pos)}</span>
      <div
        className={`progresso__trilho${arrastando.current ? " progresso__trilho--arrastando" : ""}`}
        ref={trilhoRef}
        onPointerDown={aoApertar}
        onPointerMove={aoMover}
        onPointerUp={aoSoltar}
        onPointerCancel={aoSoltar}
        // O `click` sintetizado depois do toque borbulharia e abriria a tela cheia.
        onClick={(e) => e.stopPropagation()}
      >
        <div className="progresso__feito" style={{ width: `${frac * 100}%` }} />
        <div className="progresso__knob" style={{ left: `${frac * 100}%` }} />
      </div>
      <span className="progresso__tempo">{formatarTempo(duracaoMs)}</span>
    </div>
  );
}

/**
 * Compacto: o que toca + controles, ou o caminho para resolver o problema.
 *
 * A capa preenche o quadro e derrete (fade) num painel escuro embaixo, onde ficam
 * nome, artista, o progresso e os controles — sobrepostos na imagem.
 */
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

  const { posicaoMs, duracaoMs } = progresso(local, data);

  return (
    <div className="player">
      {np.albumArt ? (
        <img className="player__capa" src={np.albumArt} alt="" />
      ) : (
        <div className="player__capa player__capa--vazia" />
      )}
      <div className="player__fade" />
      <div className="player__info">
        <div className="player__texto">
          <p className="player__track">{np.track}</p>
          <p className="player__artist">{np.artist}</p>
        </div>
        <ProgressoTocando
          posicaoMs={posicaoMs}
          duracaoMs={duracaoMs}
          tocando={np.isPlaying}
          onSeek={(ms) => void transporte("seek", np.isPlaying, ms)}
        />
        <Controles tocando={np.isPlaying} />
      </div>
    </div>
  );
}

/**
 * O lugar das faixas enquanto elas não chegam.
 *
 * Linhas do tamanho das de verdade, não um "carregando…" centralizado: o
 * layout não pula quando o conteúdo entra, e a tela já mostra a forma do que
 * está vindo.
 */
function Esqueleto() {
  return (
    <div className="sp-esqueleto" aria-hidden>
      {Array.from({ length: 6 }, (_, i) => (
        <div key={i} className="sp-esqueleto__linha" />
      ))}
    </div>
  );
}

/** Tela cheia: busca, playlists, e entrar em playlist/álbum para escolher faixa. */
function Completa({ data }: TileView<MusicState>) {
  const [termo, setTermo] = useState("");
  // A playlist/álbum em que se acabou de tocar, antes de as faixas chegarem.
  //
  // O toque abria um pedido ao Spotify e a tela ficava **igual** até ele voltar
  // — segundos olhando a mesma lista, sem saber se pegou. Como o nome e a capa
  // já estão aqui (vieram na lista), dá para entrar na hora e preencher as
  // faixas quando chegarem. É o que todo app de música faz.
  const [entrando, setEntrando] = useState<{
    uri: string;
    nome: string;
    subtitulo: string;
    albumArt: string | null;
  } | null>(null);
  // Só dá para concluir "falhou" depois de ter visto o pedido começar; sem
  // isto, o estado inicial (nada em voo) seria lido como fim e a tela voltaria
  // sozinha no mesmo quadro em que entrou.
  const comecou = useRef(false);
  const problema = data?.problema ?? null;
  const precisaLogin = problema?.tipo === "precisaLogin";

  const busca = data?.busca ?? { faixas: [], albuns: [] };
  const playlists = data?.playlists ?? [];
  const contexto = data?.contexto ?? null;
  const carregando = data?.carregando ?? null;

  // Ao abrir a tela cheia, já carrega as playlists do usuário.
  useEffect(() => {
    if (!precisaLogin) dispatchAction(MUSIC, { acao: "playlists" });
  }, [precisaLogin]);

  // O contexto de verdade chegou (ou o pedido morreu): larga o otimismo.
  //
  // ⚠️ Este efeito precisa ficar ACIMA do `return` de `precisaLogin`. Ele vivia
  // embaixo, e aí o número de hooks do componente mudava com o estado do login:
  // na hora em que o token vencia (ou em que o login terminava), o React batia
  // em "rendered more hooks than during the previous render" e a `Barreira`
  // trocava a tela cheia da música por "este quadro parou".
  useEffect(() => {
    if (!entrando) {
      comecou.current = false;
      return;
    }
    if (contexto?.uri === entrando.uri) {
      setEntrando(null);
      return;
    }
    if (carregando === "abrindo") {
      comecou.current = true;
      return;
    }
    // Começou, terminou e não trouxe o contexto: deu errado. Voltar para a
    // lista é melhor que deixar o motorista numa tela que nunca preenche.
    if (comecou.current) setEntrando(null);
  }, [contexto?.uri, carregando, entrando]);

  if (precisaLogin) {
    return (
      <div className="musica">
        <Conectar />
        {problema && <p className="tile__reason">{problema.detalhe}</p>}
      </div>
    );
  }

  // O cabeçalho pode vir do otimismo; as faixas, só do Rust.
  const aberto = contexto ?? entrando;
  const faixasCarregando = Boolean(entrando) && !contexto;
  const temBusca = busca.faixas.length > 0 || busca.albuns.length > 0;

  const submeter = (event: React.FormEvent) => {
    event.preventDefault();
    const q = termo.trim();
    if (q) dispatchAction(MUSIC, { acao: "buscar", termo: q });
  };

  // `contexto` é a playlist/álbum de onde a faixa veio: tocá-la dentro dele monta
  // a fila, e é isso que faz "próxima/anterior" andarem em vez de parar.
  const tocar = (event: MouseEvent, opts: { uri?: string; contexto?: string }) => {
    event.stopPropagation();
    void ativarAudio();
    dispatchAction(MUSIC, { acao: "tocar", ...opts });
  };

  const abrir = (
    event: MouseEvent,
    alvo: { uri: string; nome: string; subtitulo: string; albumArt: string | null },
  ) => {
    event.stopPropagation();
    setEntrando(alvo);
    dispatchAction(MUSIC, { acao: "abrir", uri: alvo.uri });
  };

  const voltar = (event: MouseEvent) => {
    event.stopPropagation();
    setEntrando(null);
    if (contexto) dispatchAction(MUSIC, { acao: "fechar" });
    else {
      setTermo("");
      dispatchAction(MUSIC, { acao: "limpar_busca" });
    }
  };

  return (
    <div className="sp" onClick={(e) => e.stopPropagation()}>
      {/* Cabeçalho: busca, ou o contexto aberto com voltar. */}
      {aberto ? (
        <header className="sp-topo">
          <button className="sp-voltar" onClick={voltar} aria-label="Voltar">
            <ChevronLeft size="1.2em" />
          </button>
          {aberto.albumArt && <img className="sp-topo__capa" src={aberto.albumArt} alt="" />}
          <span className="sp-linha__texto">
            <span className="sp-topo__nome">{aberto.nome}</span>
            <span className="sp-linha__sub">
              {aberto.subtitulo}
              {contexto ? ` · ${contexto.faixas.length} faixas` : " · carregando…"}
            </span>
          </span>
          {/* Tocar tudo não espera as faixas: o URI da playlist basta, e é o
              Spotify que monta a fila. */}
          <button className="sp-tocar-tudo" onClick={(e) => tocar(e, { contexto: aberto.uri })}>
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
        {faixasCarregando || carregando === "buscando" ? (
          <Esqueleto />
        ) : contexto ? (
          // Dentro de uma playlist/álbum: escolher a faixa.
          contexto.faixas.map((f, i) => (
            <Linha
              key={`${f.uri}-${i}`}
              capa={f.albumArt}
              titulo={f.track}
              subtitulo={f.artist}
              onClick={(e) => tocar(e, { uri: f.uri, contexto: contexto.uri })}
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
                onClick={(e) =>
                  abrir(e, {
                    uri: a.uri,
                    nome: a.nome,
                    subtitulo: a.artist,
                    albumArt: a.albumArt,
                  })
                }
              />
            ))}
            {busca.faixas.length > 0 && <p className="sp-secao">Músicas</p>}
            {busca.faixas.map((f) => (
              <Linha
                key={f.uri}
                capa={f.albumArt}
                titulo={f.track}
                subtitulo={f.artist}
                onClick={(e) => tocar(e, { uri: f.uri })}
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
                onClick={(e) =>
                  abrir(e, {
                    uri: p.uri,
                    nome: p.nome,
                    subtitulo: "playlist",
                    albumArt: p.albumArt,
                  })
                }
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
  Compact: Compacto,
  Expanded: Completa,
});
