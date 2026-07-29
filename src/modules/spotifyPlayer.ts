import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * O Eclipse como tocador de Spotify.
 *
 * Este é o caminho que dispensa o app oficial do Spotify no aparelho: o Web
 * Playback SDK registra **o próprio Eclipse** como um device do Spotify Connect,
 * e o áudio sai daqui de dentro. O Rust então manda tocar nesse device (ver
 * `NOME_DEVICE` em `spotify.rs` — os dois nomes precisam bater).
 *
 * Além de tocar, o SDK é o que dá **resposta imediata**: pausar por aqui é uma
 * chamada local (sem rede) e o estado chega por evento, em vez de esperar o
 * próximo poll do Rust. Era a soma dessas duas idas à internet que fazia o
 * play/pause parecer travado.
 *
 * Exige Spotify Premium (planos "mobile only" não valem, é regra do Spotify) e
 * DRM na WebView — verificado no Android: Widevine com `createMediaKeys` OK.
 */

/** Precisa ser idêntico ao `NOME_DEVICE` do `spotify.rs`. */
const NOME_DEVICE = "Eclipse OS";

const SDK_URL = "https://sdk.scdn.co/spotify-player.js";

/** O que o SDK informa sobre a reprodução local. */
export interface EstadoLocal {
  track: string;
  artist: string;
  albumArt: string | null;
  isPlaying: boolean;
  uri: string;
}

interface WebPlaybackState {
  paused: boolean;
  track_window?: {
    current_track?: {
      name?: string;
      uri?: string;
      artists?: { name?: string }[];
      album?: { images?: { url?: string }[] };
    };
  };
}

interface SpotifyPlayer {
  connect(): Promise<boolean>;
  disconnect(): void;
  addListener(evento: string, cb: (payload: never) => void): boolean;
  togglePlay(): Promise<void>;
  nextTrack(): Promise<void>;
  previousTrack(): Promise<void>;
  /** Em mobile o áudio só começa após um gesto do usuário. */
  activateElement(): Promise<void>;
}

declare global {
  interface Window {
    Spotify?: {
      Player: new (opts: {
        name: string;
        getOAuthToken: (cb: (token: string) => void) => void;
        volume?: number;
      }) => SpotifyPlayer;
    };
    onSpotifyWebPlaybackSDKReady?: () => void;
  }
}

/** O player vivo, para controle e gesto alcançarem-no de outro componente. */
let atual: SpotifyPlayer | null = null;
let ativado = false;

/**
 * Libera o áudio. Precisa ser chamado de dentro de um toque do usuário — é
 * exigência de navegador mobile, não capricho: sem o gesto, a transferência de
 * playback chega mas fica pausada.
 */
export async function ativarAudio(): Promise<void> {
  if (!atual || ativado) return;
  try {
    await atual.activateElement();
    ativado = true;
  } catch (err) {
    console.error("[eclipse-player] activateElement falhou", err);
  }
}

/**
 * Controla a reprodução **localmente**, sem rede. Devolve `false` quando não há
 * player aqui (o som está em outro aparelho) — aí quem chama cai no caminho do
 * Rust, que comanda pela Web API.
 */
export async function controlarLocal(acao: string): Promise<boolean> {
  if (!atual) return false;
  try {
    if (acao === "toggle") await atual.togglePlay();
    else if (acao === "next") await atual.nextTrack();
    else if (acao === "prev") await atual.previousTrack();
    else return false;
    return true;
  } catch (err) {
    console.error(`[eclipse-player] ${acao} local falhou`, err);
    return false;
  }
}

function carregarSdk(): Promise<void> {
  if (window.Spotify) return Promise.resolve();
  return new Promise((resolve, reject) => {
    // O SDK chama este global quando termina de carregar; precisa existir antes.
    window.onSpotifyWebPlaybackSDKReady = () => resolve();
    const existente = document.querySelector(`script[src="${SDK_URL}"]`);
    if (existente) return;
    const script = document.createElement("script");
    script.src = SDK_URL;
    script.async = true;
    script.onerror = () => reject(new Error("não consegui carregar o SDK do Spotify"));
    document.head.appendChild(script);
  });
}

export type StatusPlayer = "off" | "carregando" | "pronto" | "erro";

/* ------------------------------------------------------------------ */
/* Publicação de status e de estado — para os tiles lerem              */
/* ------------------------------------------------------------------ */

let statusAtual: StatusPlayer = "off";
const ouvintesStatus = new Set<(s: StatusPlayer) => void>();

function publicarStatus(s: StatusPlayer) {
  statusAtual = s;
  ouvintesStatus.forEach((cb) => cb(s));
}

/** Diz se o Eclipse já virou device do Spotify — sem isto "não toca" é mudo. */
export function useStatusPlayer(): StatusPlayer {
  const [status, setStatus] = useState(statusAtual);
  useEffect(() => {
    ouvintesStatus.add(setStatus);
    setStatus(statusAtual);
    return () => {
      ouvintesStatus.delete(setStatus);
    };
  }, []);
  return status;
}

let estadoAtual: EstadoLocal | null = null;
const ouvintesEstado = new Set<(e: EstadoLocal | null) => void>();

function publicarEstado(e: EstadoLocal | null) {
  estadoAtual = e;
  ouvintesEstado.forEach((cb) => cb(e));
}

/**
 * O que está tocando, **empurrado pelo SDK**. Chega no instante em que muda, em
 * vez de esperar o poll de 3s do Rust — é o que tira a sensação de UI atrasada.
 * `null` quando o som não está saindo pelo Eclipse.
 */
export function useEstadoLocal(): EstadoLocal | null {
  const [estado, setEstado] = useState(estadoAtual);
  useEffect(() => {
    ouvintesEstado.add(setEstado);
    setEstado(estadoAtual);
    return () => {
      ouvintesEstado.delete(setEstado);
    };
  }, []);
  return estado;
}

/** Reflete o toque na hora, antes de o SDK confirmar. */
export function marcarTocandoOtimista(tocando: boolean): void {
  if (estadoAtual) publicarEstado({ ...estadoAtual, isPlaying: tocando });
}

/* ------------------------------------------------------------------ */

/**
 * Monta o player. Deve ficar no App (uma instância só) — o tile de música monta
 * duas vezes (compacto e expandido) e dois players brigariam pelo mesmo device.
 */
export function useSpotifyPlayer(perfilId: string | null, logado: boolean): StatusPlayer {
  const [status, setStatus] = useState<StatusPlayer>("off");

  useEffect(() => {
    if (!perfilId || !logado) {
      setStatus("off");
      publicarStatus("off");
      return;
    }

    let player: SpotifyPlayer | null = null;
    let vivo = true;
    setStatus("carregando");
    publicarStatus("carregando");

    const subir = async () => {
      try {
        await carregarSdk();
        if (!vivo || !window.Spotify) return;

        player = new window.Spotify.Player({
          name: NOME_DEVICE,
          // O SDK chama isto no início e a cada renovação: o token vem do Rust,
          // que já tem o refresh token do perfil no cofre.
          getOAuthToken: (cb) => {
            void invoke<string>("spotify_access_token", { id: perfilId })
              .then(cb)
              .catch((err) =>
                console.error("[eclipse-player] falha ao pegar o token", err),
              );
          },
          volume: 0.8,
        });

        player.addListener("ready", ((p: { device_id: string }) => {
          console.log(`[eclipse-player] PRONTO device_id=${p.device_id}`);
          if (vivo) {
            setStatus("pronto");
            publicarStatus("pronto");
          }
        }) as never);

        player.addListener("not_ready", (() => {
          if (vivo) publicarEstado(null);
        }) as never);

        // O estado vem por evento: é daqui que a UI aprende o que toca, sem poll.
        player.addListener("player_state_changed", ((s: WebPlaybackState | null) => {
          if (!vivo) return;
          const faixa = s?.track_window?.current_track;
          if (!s || !faixa) {
            publicarEstado(null);
            return;
          }
          publicarEstado({
            track: faixa.name ?? "",
            artist: (faixa.artists ?? []).map((a) => a.name ?? "").filter(Boolean).join(", "),
            albumArt: faixa.album?.images?.[0]?.url ?? null,
            isPlaying: !s.paused,
            uri: faixa.uri ?? "",
          });
        }) as never);

        for (const erro of [
          "initialization_error",
          "authentication_error",
          "account_error",
          "playback_error",
        ]) {
          player.addListener(erro, ((p: { message: string }) => {
            console.error(`[eclipse-player] ${erro}: ${p?.message}`);
            // `account_error` é quase sempre conta sem Premium (ou Premium
            // "mobile only", que o SDK recusa) — não é falha de código.
            if (vivo && erro !== "playback_error") {
              setStatus("erro");
              publicarStatus("erro");
            }
          }) as never);
        }

        atual = player;
        await player.connect();
      } catch (err) {
        console.error("[eclipse-player] não subiu", err);
        if (vivo) {
          setStatus("erro");
          publicarStatus("erro");
        }
      }
    };

    void subir();

    return () => {
      vivo = false;
      player?.disconnect();
      if (atual === player) {
        atual = null;
        ativado = false;
        publicarEstado(null);
      }
    };
  }, [perfilId, logado]);

  return status;
}
