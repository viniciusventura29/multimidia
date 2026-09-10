import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Quanto esperar depois do boot antes da primeira pergunta.
 *
 * Não é folga de estilo. Na ignição a head unit está pareando o ELM327,
 * fixando GPS, subindo o Web Playback SDK e compilando os shaders do carro 3D —
 * e, o que mais pesa, **a rede em geral ainda não existe**: o tethering do
 * celular associa alguns segundos depois. Perguntar aos 0 s é quase garantir
 * uma falha, e a falha custa quinze minutos de espera até a próxima tentativa.
 */
const ATRASO_INICIAL_MS = 60_000;

/**
 * Entre duas perguntas que deram certo.
 *
 * Uma viagem típica é de 20-40 min, então na prática é uma pergunta por
 * ignição. As seis horas são para o carro que fica ligado a tarde inteira: ele
 * fica sabendo no mesmo dia sem martelar o GitHub.
 */
const DESCANSO_MS = 6 * 60 * 60 * 1000;

/**
 * Depois de uma pergunta que não deu.
 *
 * Cadência própria, e não o mesmo descanso: "sem rede na ignição, com rede três
 * minutos depois" é o caso NORMAL do carro, não a exceção. Com um intervalo só
 * eu teria que escolher entre martelar o GitHub o dia inteiro ou perder a
 * janela em que a rede finalmente subiu.
 */
const RETENTAR_MS = 15 * 60 * 1000;

/** O que o Rust devolve. A URL fica lá — ver `baixar`. */
export interface Atualizacao {
  versionCode: number;
}

/**
 * "Tem versão nova?", e o toque que leva ao download.
 *
 * SILÊNCIO ABSOLUTO QUANDO FALHA. O `catch` não mexe no estado — nem para
 * limpar. Carro sem rede não vê nada: nem pílula, nem chip apagando, nem
 * `console.error` (que num aparelho sem SIM pareceria defeito onde não há).
 * Não limpar também evita o chip piscar: quem já sabe que existe a 43 continua
 * sabendo depois de entrar num túnel.
 *
 * O `StrictMode` monta duas vezes em desenvolvimento, mas o primeiro cleanup
 * derruba o timer bem antes dos 60 s — nenhuma pergunta duplicada sai daqui.
 */
export function useAtualizacao(): {
  nova: Atualizacao | null;
  baixar: () => void;
} {
  const [nova, setNova] = useState<Atualizacao | null>(null);

  useEffect(() => {
    let vivo = true;
    let timer: ReturnType<typeof setTimeout>;

    const perguntar = async () => {
      // O padrão é o intervalo curto: só uma resposta de verdade compra as seis
      // horas de descanso.
      let proximo = RETENTAR_MS;
      try {
        const resposta = await invoke<Atualizacao | null>("checar_atualizacao");
        if (!vivo) return;
        setNova(resposta);
        proximo = DESCANSO_MS;
      } catch (err) {
        console.debug("[eclipse] não deu para checar atualização", err);
      }
      if (vivo) timer = setTimeout(() => void perguntar(), proximo);
    };

    timer = setTimeout(() => void perguntar(), ATRASO_INICIAL_MS);
    return () => {
      vivo = false;
      clearTimeout(timer);
    };
  }, []);

  // Quem abre o navegador é o Rust, com a URL que ele mesmo peneirou — a tela
  // pede o efeito de um toque, não o inventa (mesmo princípio do
  // `dispatch_action`).
  const baixar = useCallback(() => {
    void invoke("baixar_atualizacao").catch((err) =>
      console.debug("[eclipse] não deu para abrir o navegador", err),
    );
  }, []);

  return { nova, baixar };
}
