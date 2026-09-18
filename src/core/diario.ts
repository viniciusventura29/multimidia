import { invoke } from "@tauri-apps/api/core";

/**
 * A ponte do painel para o diário de bordo do Rust.
 *
 * O que morre em JavaScript não passa por `tracing` nenhum: a tela branca do
 * React acontece inteira dentro do WebView, e o Rust nunca fica sabendo. Como o
 * carro não tem console nem quem olhe, esse tipo de falha hoje só existe como
 * "deu um negócio estranho ontem" contado de memória.
 */

type Nivel = "debug" | "info" | "aviso" | "erro";

/** Anota uma linha. Falha em silêncio de propósito — ver `ligarDiario`. */
export function anotar(
  nivel: Nivel,
  onde: string,
  msg: string,
  dados?: Record<string, unknown>,
): void {
  invoke("anotar_do_painel", { nivel, onde, msg, dados }).catch(() => {});
}

/**
 * Liga a captura de erro do WebView. Chamado uma vez no `main.tsx`.
 *
 * Nada aqui pode lançar nem logar de volta: um relator de erro que erra vira um
 * laço que se alimenta do próprio erro, e aí o carro passa a gravar a falha do
 * logger em vez da falha que interessa.
 */
export function ligarDiario(): void {
  window.addEventListener("error", (evento) => {
    // `error` também dispara para recurso que não carregou (img, script), e aí
    // não há `error.message` — vale registrar mesmo assim: no carro, "a fonte
    // não baixou" explica uma tela torta melhor que silêncio.
    const erro = evento.error as Error | undefined;
    anotar("erro", "painel", erro?.message ?? evento.message ?? "erro sem mensagem", {
      pilha: erro?.stack?.slice(0, 2000),
      arquivo: evento.filename,
      linha: evento.lineno,
    });
  });

  // Promise rejeitada sem `catch` é como morrem as chamadas ao Rust e os
  // `fetch` do mapa — sem nenhum aviso na tela.
  window.addEventListener("unhandledrejection", (evento) => {
    const motivo = evento.reason as unknown;
    const erro = motivo instanceof Error ? motivo : undefined;
    anotar("erro", "painel", erro?.message ?? String(motivo), {
      pilha: erro?.stack?.slice(0, 2000),
    });
  });
}
