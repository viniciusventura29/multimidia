import { useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Entrega o destino ao app do Google Maps.
 *
 * O painel não navega. Já tentou: rota própria, manobras calculadas contra o
 * GPS, aviso de desvio — funcionava, e mesmo assim era uma imitação pior de um
 * app que roda nativo na mesma head unit. Sem trânsito ao vivo, sem orientação
 * de faixa, sem voz. Passar a bola dá ao motorista a coisa de verdade.
 */
export function Destino() {
  const [destino, setDestino] = useState("");
  const [erro, setErro] = useState<string | null>(null);

  const abrir = async (event: FormEvent) => {
    event.preventDefault();
    event.stopPropagation();

    const alvo = destino.trim();
    if (!alvo) return;

    try {
      await invoke("open_navigation", { destino: alvo });
      setDestino("");
      setErro(null);
    } catch (err) {
      console.error("[eclipse] não consegui abrir o Maps", err);
      setErro("não consegui abrir o Maps");
    }
  };

  return (
    <form className="rota__busca" onSubmit={abrir} onClick={(e) => e.stopPropagation()}>
      <input
        className="rota__campo"
        value={destino}
        onChange={(e) => setDestino(e.target.value)}
        placeholder={erro ?? "para onde?"}
        aria-label="Destino"
      />
      <button className="rota__ir" type="submit" disabled={!destino.trim()}>
        guiar
      </button>
    </form>
  );
}
