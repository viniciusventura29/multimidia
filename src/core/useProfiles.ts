import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { useModuleSelector } from "./moduleStore";
import type { MapaState } from "../modules/nav/tipos";
import type { Profile } from "./types";

const EVENT_PROFILE = "profile-changed";

export interface Perfis {
  profiles: Profile[];
  active: Profile | null;
  carregando: boolean;
  criar: (name: string, color: string) => Promise<void>;
  selecionar: (id: string) => Promise<void>;
  remover: (id: string) => Promise<void>;
}

/**
 * Os perfis e quem está dirigindo.
 *
 * O Rust é a fonte da verdade: cada operação vai até ele e a lista é relida da
 * resposta dele. A tela não mantém uma cópia própria que possa divergir do disco.
 */
export function useProfiles(): Perfis {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [active, setActive] = useState<Profile | null>(null);
  const [carregando, setCarregando] = useState(true);

  const recarregar = useCallback(async () => {
    const [lista, ativo] = await Promise.all([
      invoke<Profile[]>("list_profiles"),
      invoke<Profile | null>("active_profile"),
    ]);
    setProfiles(lista);
    setActive(ativo);
  }, []);

  useEffect(() => {
    let alive = true;

    // O perfil também pode mudar sem passar por esta tela (na Fase 5, um token
    // vencido pode forçar troca), então vale ouvir o evento além de recarregar.
    const unlisten = listen<Profile | null>(EVENT_PROFILE, (event) => {
      if (alive) setActive(event.payload);
    });

    recarregar()
      .catch((err) => console.error("[eclipse] falha ao carregar perfis", err))
      .finally(() => {
        if (alive) setCarregando(false);
      });

    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, [recarregar]);

  const comando = useCallback(
    async (nome: string, args: Record<string, unknown>) => {
      try {
        await invoke(nome, args);
        await recarregar();
      } catch (err) {
        console.error(`[eclipse] ${nome} falhou`, err);
      }
    },
    [recarregar],
  );

  return {
    profiles,
    active,
    carregando,
    criar: (name, color) => comando("create_profile", { name, color }),
    selecionar: (id) => comando("select_profile", { id }),
    remover: (id) => comando("delete_profile", { id }),
  };
}

/**
 * Pinta o app com a cor do perfil ativo.
 *
 * Escreve `--accent-perfil` (a cor CRUA escolhida), e não `--accent`. Quem lê a
 * crua e decide o que o painel usa é o CSS: no tema escuro `--accent` é ela
 * mesma; no claro, uma versão escurecida dela, porque as cores do seletor foram
 * escolhidas para brilhar sobre quase-preto e somem sobre fundo claro. Ver o
 * comentário do `--accent` no `App.css`.
 */
export function useTema(active: Profile | null): void {
  useEffect(() => {
    document.documentElement.style.setProperty(
      "--accent-perfil",
      active?.color ?? "#3ddc97",
    );
  }, [active]);
}

/**
 * Claro de dia, escuro de noite.
 *
 * O sinal vem do módulo `nav`, e é o MESMO que já troca o estilo do mapa: quem
 * decide é a elevação do sol na posição do carro (`crates/eclipse-gps/src/sol.rs`,
 * NOAA simplificada), não a hora do relógio — que erraria uma hora e meia entre
 * junho e dezembro. O Rust reavalia de minuto em minuto mesmo sem fix novo, então
 * o painel vira sozinho no pôr do sol com o carro parado na garagem.
 *
 * Escreve num atributo do `<html>` em vez de numa classe de componente porque o
 * tema tem de valer para o que mora FORA da árvore do React: o `color-scheme`
 * (barra de rolagem, `input`), e as camadas do MapLibre.
 *
 * **O padrão é escuro** quando o `nav` ainda não respondeu (`?? true`). Não é
 * arbitrário: piscar branco na cara de quem está dirigindo à noite é bem pior que
 * meio segundo de escuro num dia claro. É também o padrão que o mapa já usa, em
 * `modules/nav/mapa.tsx` — os dois precisam concordar, senão o painel abre claro
 * com um mapa escuro dentro.
 */
export function useTemaDoDia(): void {
  const noite = useModuleSelector<MapaState, boolean>(
    "nav",
    (nav) => nav?.noite ?? true,
  );

  useEffect(() => {
    document.documentElement.dataset.tema = noite ? "escuro" : "claro";
  }, [noite]);
}
