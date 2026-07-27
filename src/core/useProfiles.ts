import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

/** Pinta o app com a cor do perfil ativo. */
export function useTema(active: Profile | null): void {
  useEffect(() => {
    document.documentElement.style.setProperty(
      "--accent",
      active?.color ?? "#3ddc97",
    );
  }, [active]);
}
