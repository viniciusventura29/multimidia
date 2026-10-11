/**
 * A ponte do carro, lado da tela. Só existe com o Vite servindo
 * (`import.meta.hot`): no build de produção o `main.tsx` nem chega a importar
 * este arquivo.
 *
 * O par dela é `scripts/ponte-do-carro.ts`. Aqui o console vai para o Mac, e
 * os comandos do Mac (`scripts/carro.mjs`) são executados na tela — que, no
 * Eclipse DEV, é a da central.
 */

type Comando =
  | { id: string; tipo: "eval"; arg: string }
  | { id: string; tipo: "click"; arg: string }
  | { id: string; tipo: "toque"; x: number; y: number }
  | { id: string; tipo: "texto" }
  | { id: string; tipo: "print" };

/** Um valor qualquer virando uma linha legível de log. */
function texto(valor: unknown): string {
  if (typeof valor === "string") return valor;
  if (valor instanceof Error) return `${valor.name}: ${valor.message}\n${valor.stack ?? ""}`;
  try {
    return JSON.stringify(valor);
  } catch {
    return String(valor);
  }
}

/** Toque de verdade num elemento: ponteiro, mouse e clique, nessa ordem. */
function tocar(alvo: Element, x: number, y: number): void {
  const base = { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0 };
  const ponteiro = { ...base, pointerId: 1, pointerType: "touch", isPrimary: true };
  alvo.dispatchEvent(new PointerEvent("pointerdown", ponteiro));
  alvo.dispatchEvent(new MouseEvent("mousedown", base));
  alvo.dispatchEvent(new PointerEvent("pointerup", ponteiro));
  alvo.dispatchEvent(new MouseEvent("mouseup", base));
  alvo.dispatchEvent(new MouseEvent("click", base));
}

function centro(alvo: Element): [number, number] {
  const r = alvo.getBoundingClientRect();
  return [r.left + r.width / 2, r.top + r.height / 2];
}

async function executar(cmd: Comando): Promise<unknown> {
  switch (cmd.tipo) {
    case "eval":
      // Eval indireto: roda no escopo global, como o console do DevTools.
      return await (0, eval)(cmd.arg);
    case "click": {
      const alvo = document.querySelector(cmd.arg);
      if (!alvo) throw new Error(`nada casa com ${cmd.arg}`);
      tocar(alvo, ...centro(alvo));
      return alvo.outerHTML.slice(0, 200);
    }
    case "toque": {
      const alvo = document.elementFromPoint(cmd.x, cmd.y);
      if (!alvo) throw new Error(`nada em ${cmd.x},${cmd.y}`);
      tocar(alvo, cmd.x, cmd.y);
      return alvo.outerHTML.slice(0, 200);
    }
    case "texto":
      return document.body.innerText;
    case "print": {
      // O canvas do mapa sai preto: o MapLibre não guarda o buffer do WebGL
      // depois de desenhar. O resto da tela sai como está.
      const { toPng } = await import("html-to-image");
      return await toPng(document.body, {
        width: innerWidth,
        height: innerHeight,
        pixelRatio: 1,
      });
    }
  }
}

export function ligarPonte(hot: NonNullable<ImportMeta["hot"]>): void {
  const enviar = (nivel: string, args: unknown[]) => {
    try {
      hot.send("carro:log", { nivel, texto: args.map(texto).join(" ") });
    } catch {
      // Sem socket não há para onde mandar, e logar a falha daria um laço.
    }
  };

  for (const nivel of ["log", "info", "warn", "error", "debug"] as const) {
    const original = console[nivel].bind(console);
    console[nivel] = (...args: unknown[]) => {
      original(...args);
      enviar(nivel, args);
    };
  }
  window.addEventListener("error", (e) =>
    enviar("ERRO", [e.error ?? e.message, `${e.filename}:${e.lineno}`]),
  );
  window.addEventListener("unhandledrejection", (e) =>
    enviar("ERRO", ["promise rejeitada:", e.reason]),
  );

  hot.on("carro:cmd", async (cmd: Comando) => {
    try {
      const valor = await executar(cmd);
      hot.send("carro:resp", { id: cmd.id, ok: true, valor: texto(valor) });
    } catch (e) {
      hot.send("carro:resp", { id: cmd.id, ok: false, erro: texto(e) });
    }
  });

  enviar("info", ["ponte ligada", `${innerWidth}x${innerHeight}`, navigator.userAgent]);
}
