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
 * Exige Spotify Premium (planos "mobile only" não valem, é regra do Spotify) e
 * DRM na WebView — verificado no Android: Widevine presente e `createMediaKeys`
 * funcionando.
 */

/** Precisa ser idêntico ao `NOME_DEVICE` do `spotify.rs`. */
const NOME_DEVICE = "Eclipse OS";

const SDK_URL = "https://sdk.scdn.co/spotify-player.js";

interface SpotifyPlayer {
  connect(): Promise<boolean>;
  disconnect(): void;
  addListener(evento: string, cb: (payload: never) => void): boolean;
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

/** O player vivo, para o gesto de ativação poder alcançá-lo de outro componente. */
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
    console.log("[eclipse-player] áudio ativado pelo gesto");
  } catch (err) {
    console.error("[eclipse-player] activateElement falhou", err);
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

/**
 * O status também é publicado aqui para o tile mostrar na tela — sem isto o
 * usuário não tem como saber se o Eclipse já virou device do Spotify, e "não
 * toca" fica indistinguível de "ainda subindo".
 */
let statusAtual: StatusPlayer = "off";
const ouvintes = new Set<(s: StatusPlayer) => void>();

function publicar(s: StatusPlayer) {
  statusAtual = s;
  ouvintes.forEach((cb) => cb(s));
}

export function useStatusPlayer(): StatusPlayer {
  const [status, setStatus] = useState(statusAtual);
  useEffect(() => {
    ouvintes.add(setStatus);
    setStatus(statusAtual);
    return () => {
      ouvintes.delete(setStatus);
    };
  }, []);
  return status;
}

/**
 * Monta o player. Deve ficar no App (uma instância só) — o tile de música monta
 * duas vezes (compacto e expandido) e dois players brigariam pelo mesmo device.
 */
export function useSpotifyPlayer(perfilId: string | null, logado: boolean): StatusPlayer {
  const [status, setStatus] = useState<StatusPlayer>("off");

  useEffect(() => {
    if (!perfilId || !logado) {
      setStatus("off");
      publicar("off");
      return;
    }

    let player: SpotifyPlayer | null = null;
    let vivo = true;
    setStatus("carregando");
    publicar("carregando");

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
            publicar("pronto");
          }
        }) as never);

        player.addListener("not_ready", (() => {
          console.log("[eclipse-player] device saiu do ar");
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
              publicar("erro");
            }
          }) as never);
        }

        atual = player;
        const conectou = await player.connect();
        console.log(`[eclipse-player] connect() => ${conectou}`);
      } catch (err) {
        console.error("[eclipse-player] não subiu", err);
        if (vivo) {
          setStatus("erro");
          publicar("erro");
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
      }
    };
  }, [perfilId, logado]);

  return status;
}
