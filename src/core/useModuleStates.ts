import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ModuleStates, StateEnvelope } from "./types";

const EVENT_MODULE_STATE = "module-state";

/**
 * O estado de todos os módulos, vivo.
 *
 * Assina o barramento antes de pedir o snapshot: o contrário abriria uma janela
 * entre as duas chamadas onde um evento se perderia. Como cada envelope traz o
 * estado inteiro do módulo e um `seq` monotônico, um snapshot que chegue atrasado
 * é simplesmente descartado.
 */
export function useModuleStates(): ModuleStates {
  const [states, setStates] = useState<ModuleStates>({});

  useEffect(() => {
    let alive = true;

    const apply = (envelope: StateEnvelope) => {
      if (!alive) return;
      setStates((prev) => {
        const current = prev[envelope.module];
        if (current && current.seq >= envelope.seq) return prev;
        return { ...prev, [envelope.module]: envelope };
      });
    };

    const unlisten = listen<StateEnvelope>(EVENT_MODULE_STATE, (event) =>
      apply(event.payload),
    );

    unlisten
      .then(() => invoke<StateEnvelope[]>("get_snapshot"))
      .then((snapshot) => snapshot.forEach(apply))
      .catch((err) => console.error("[eclipse] falha ao carregar estado", err));

    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, []);

  return states;
}
