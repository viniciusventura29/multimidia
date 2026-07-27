import { invoke } from "@tauri-apps/api/core";

/**
 * Manda uma ação para o módulo dono dela.
 *
 * Não retorna o resultado de propósito: quem responde é o próximo estado que o
 * módulo publicar. A tela nunca pinta o efeito de um toque por conta própria —
 * ela espera o Rust confirmar.
 */
export function dispatchAction(module: string, payload: unknown): void {
  void invoke("dispatch_action", { module, payload }).catch((err) =>
    console.error(`[eclipse] ação falhou em ${module}`, err),
  );
}
