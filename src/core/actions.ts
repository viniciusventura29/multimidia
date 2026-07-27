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

/**
 * Abre o fluxo de autorização do Spotify para o perfil ativo.
 *
 * O navegador abre, o usuário aprova, e o Spotify devolve o código para um
 * servidor de uma requisição só rodando no próprio aparelho. A promessa só
 * resolve quando isso termina — pode demorar o tempo do usuário pensar.
 */
export async function conectarSpotify(): Promise<void> {
  const perfil = await invoke<{ id: string } | null>("active_profile");
  if (!perfil) return;
  await invoke("connect_spotify", { id: perfil.id });
}
