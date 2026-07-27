import { useEffect, useRef, useState } from "react";

import type { Fix } from "./tipos";

/**
 * Espera o motorista parar de digitar antes de perguntar ao Google.
 *
 * Sem isto, cada tecla vira uma requisição cobrada. Digitar "Av. Paulista" seriam
 * doze.
 */
const ESPERA_MS = 280;

/** Abaixo disto a sugestão é inútil e a requisição, desperdício. */
const MINIMO_DE_LETRAS = 3;

/** Raio em torno do carro para priorizar o que está perto. */
const RAIO_M = 30_000;

export interface Sugestao {
  placeId: string;
  /** O nome do lugar, com a parte que casou com o que foi digitado. */
  principal: string;
  /** Onde fica: rua, bairro, cidade. */
  complemento: string;
  /** Trechos de `principal` que casam com a busca, para destacar. */
  destaques: { inicio: number; fim: number }[];
}

interface Resposta {
  suggestions?: {
    placePrediction?: {
      placeId: string;
      structuredFormat?: {
        mainText?: {
          text: string;
          matches?: { startOffset?: number; endOffset?: number }[];
        };
        secondaryText?: { text: string };
      };
    };
  }[];
}

/**
 * Sugestões de destino enquanto se digita.
 *
 * Enviesado pela posição do carro: procurar "posto" tem que trazer o da esquina,
 * não um do outro estado. Sem posição ainda, busca pelo Brasil inteiro.
 *
 * O token de sessão agrupa as requisições de uma mesma digitação numa cobrança
 * só. Ele é renovado quando um destino é escolhido ou a busca é abandonada —
 * mantê-lo vivo entre buscas diferentes seria cobrança errada e resultados piores.
 */
export function useSugestoes(
  texto: string,
  apiKey: string,
  fix: Fix | null,
): { sugestoes: Sugestao[]; limpar: () => void } {
  const [sugestoes, setSugestoes] = useState<Sugestao[]>([]);
  const sessao = useRef<string>(crypto.randomUUID());

  const limpar = () => {
    setSugestoes([]);
    sessao.current = crypto.randomUUID();
  };

  useEffect(() => {
    const busca = texto.trim();
    if (busca.length < MINIMO_DE_LETRAS) {
      setSugestoes([]);
      return;
    }

    // `abortado` cobre o caso de a resposta chegar depois de o componente
    // sumir ou de a busca ter mudado — senão a lista pisca com resultado velho.
    let abortado = false;

    const timer = setTimeout(async () => {
      try {
        const resposta = await fetch(
          "https://places.googleapis.com/v1/places:autocomplete",
          {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
              "X-Goog-Api-Key": apiKey,
            },
            body: JSON.stringify({
              input: busca,
              languageCode: "pt-BR",
              regionCode: "BR",
              sessionToken: sessao.current,
              ...(fix && {
                locationBias: {
                  circle: {
                    center: { latitude: fix.lat, longitude: fix.lon },
                    radius: RAIO_M,
                  },
                },
              }),
            }),
          },
        );

        if (abortado) return;

        const dados: Resposta = await resposta.json();
        setSugestoes(
          (dados.suggestions ?? [])
            .map((s) => s.placePrediction)
            .filter((p) => p?.placeId)
            .map((p) => ({
              placeId: p!.placeId,
              principal: p!.structuredFormat?.mainText?.text ?? "",
              complemento: p!.structuredFormat?.secondaryText?.text ?? "",
              destaques: (p!.structuredFormat?.mainText?.matches ?? []).map(
                (m) => ({ inicio: m.startOffset ?? 0, fim: m.endOffset ?? 0 }),
              ),
            })),
        );
      } catch (err) {
        if (!abortado) {
          console.error("[eclipse] falha ao buscar sugestões", err);
          setSugestoes([]);
        }
      }
    }, ESPERA_MS);

    return () => {
      abortado = true;
      clearTimeout(timer);
    };
  }, [texto, apiKey, fix?.lat, fix?.lon]);

  return { sugestoes, limpar };
}

/** Quebra o texto nos trechos que casam com a busca, para destacá-los. */
export function destacar(
  texto: string,
  destaques: { inicio: number; fim: number }[],
): { trecho: string; forte: boolean }[] {
  if (destaques.length === 0) return [{ trecho: texto, forte: false }];

  const partes: { trecho: string; forte: boolean }[] = [];
  let cursor = 0;

  for (const { inicio, fim } of destaques) {
    if (inicio > cursor) {
      partes.push({ trecho: texto.slice(cursor, inicio), forte: false });
    }
    partes.push({ trecho: texto.slice(inicio, fim), forte: true });
    cursor = fim;
  }

  if (cursor < texto.length) {
    partes.push({ trecho: texto.slice(cursor), forte: false });
  }

  return partes;
}
