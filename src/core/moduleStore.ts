import { useRef, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { StateEnvelope, Status } from "./types";

const EVENT_MODULE_STATE = "module-state";

/**
 * O estado de todos os módulos, vivo — fora do React.
 *
 * Antes isto era um `useState` com um objeto único: cada evento de qualquer
 * módulo recriava o objeto inteiro e re-renderizava o painel todo, do App ao
 * último tile. Um tick de RPM redesenhava o mapa. Aqui cada módulo tem seu
 * próprio conjunto de assinantes, e um evento do OBD só acorda quem lê do OBD.
 */
const envelopes = new Map<string, StateEnvelope>();
const assinantes = new Map<string, Set<() => void>>();
// `useSyncExternalStore` reassina quando a função de subscribe muda de
// identidade — então ela é memorizada por módulo, uma para sempre.
const assinaturas = new Map<string, (avisar: () => void) => () => void>();

function apply(envelope: StateEnvelope) {
  const atual = envelopes.get(envelope.module);
  if (atual && atual.seq >= envelope.seq) return;
  envelopes.set(envelope.module, envelope);
  assinantes.get(envelope.module)?.forEach((avisar) => avisar());
}

let ligado = false;

/**
 * Liga o barramento. Chamado uma vez no `main.tsx`, antes do React montar:
 * o bus vive a vida do WebView, não a de um componente.
 *
 * Assina o barramento antes de pedir o snapshot: o contrário abriria uma janela
 * entre as duas chamadas onde um evento se perderia. Como cada envelope traz o
 * estado inteiro do módulo e um `seq` monotônico, um snapshot que chegue
 * atrasado é simplesmente descartado.
 */
export function startModuleBus(): void {
  if (ligado) return;
  ligado = true;

  listen<StateEnvelope>(EVENT_MODULE_STATE, (event) => apply(event.payload))
    .then(() => invoke<StateEnvelope[]>("get_snapshot"))
    .then((snapshot) => snapshot.forEach(apply))
    .catch((err) => console.error("[eclipse] falha ao carregar estado", err));
}

function assinatura(module: string) {
  let sub = assinaturas.get(module);
  if (!sub) {
    sub = (avisar: () => void) => {
      let set = assinantes.get(module);
      if (!set) {
        set = new Set();
        assinantes.set(module, set);
      }
      set.add(avisar);
      return () => set.delete(avisar);
    };
    assinaturas.set(module, sub);
  }
  return sub;
}

/** O envelope inteiro de um módulo. Re-renderiza a cada evento DESSE módulo. */
export function useModuleEnvelope(module: string): StateEnvelope | undefined {
  return useSyncExternalStore(assinatura(module), () => envelopes.get(module));
}

/**
 * Uma fatia derivada do estado de um módulo, com igualdade.
 *
 * Só re-renderiza quando o VALOR selecionado muda — um tile que lê a voltagem
 * não acorda porque o RPM subiu. `selector` deve ser puro e depender apenas de
 * `(data, status)`: nada de fechar sobre props ou estado do componente, porque
 * o resultado é cacheado por envelope.
 */
export function useModuleSelector<T, R>(
  module: string,
  selector: (data: T | null, status: Status) => R,
  equals: (a: R, b: R) => boolean = Object.is,
): R {
  const cache = useRef<{ env: StateEnvelope | undefined; value: R } | null>(null);

  // `getSnapshot` precisa devolver a MESMA referência enquanto nada mudou —
  // senão o `useSyncExternalStore` entra em loop de render.
  const getSnapshot = () => {
    const env = envelopes.get(module);
    if (cache.current && cache.current.env === env) return cache.current.value;
    const value = selector((env?.data ?? null) as T | null, env?.status ?? "loading");
    if (cache.current && equals(cache.current.value, value)) {
      cache.current.env = env;
      return cache.current.value;
    }
    cache.current = { env, value };
    return value;
  };

  return useSyncExternalStore(assinatura(module), getSnapshot);
}

/** Igualdade rasa para seletores que devolvem um objetinho de campos. */
export function shallowEqual<R>(a: R, b: R): boolean {
  if (Object.is(a, b)) return true;
  if (
    typeof a !== "object" ||
    typeof b !== "object" ||
    a === null ||
    b === null
  ) {
    return false;
  }
  const chavesA = Object.keys(a) as (keyof R)[];
  const chavesB = Object.keys(b) as (keyof R)[];
  if (chavesA.length !== chavesB.length) return false;
  return chavesA.every((k) => Object.is(a[k], b[k]));
}
