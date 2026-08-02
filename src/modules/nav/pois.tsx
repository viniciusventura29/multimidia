import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useMap } from "@vis.gl/react-google-maps";

import type { Fix } from "./tipos";

/** Raio de busca em torno do carro. Mais que isso não é "perto de mim". */
const RAIO_M = 5_000;

/** Dez pinos já enchem um mapa de carro; mais vira poluição. */
const MAXIMO = 10;

/**
 * O que um motorista procura sem digitar: combustível e vaga. Os tipos são os
 * da Places API; os rótulos, o que cabe num botão.
 */
const CATEGORIAS = {
  gas_station: "postos",
  parking: "estacionar",
} as const;

type Categoria = keyof typeof CATEGORIAS;

interface Lugar {
  nome: string;
  lat: number;
  lng: number;
}

interface Resposta {
  places?: {
    displayName?: { text?: string };
    location?: { latitude?: number; longitude?: number };
  }[];
}

/**
 * Postos e estacionamentos por perto, sob demanda.
 *
 * Cada toque é **uma** requisição à Places API (New) — nada roda sozinho,
 * porque busca contínua seria cobrança contínua (mesma consciência de cota do
 * autocomplete em `sugestoes.ts`). Tocar de novo só limpa os pinos, de graça.
 */
export function Pois({ fix, apiKey }: { fix: Fix | null; apiKey: string }) {
  const map = useMap();
  const [categoria, setCategoria] = useState<Categoria | null>(null);
  const [buscando, setBuscando] = useState(false);
  const [lugares, setLugares] = useState<Lugar[]>([]);
  const marcadores = useRef<google.maps.Marker[]>([]);

  // Redesenha os pinos quando a lista muda — e quando o mapa é recriado pela
  // troca de tema (dia/noite), que abandonaria marcadores presos ao antigo.
  useEffect(() => {
    marcadores.current.forEach((m) => m.setMap(null));
    if (!map) return;

    marcadores.current = lugares.map(
      (lugar) =>
        new google.maps.Marker({
          map,
          position: { lat: lugar.lat, lng: lugar.lng },
          title: lugar.nome,
        }),
    );
  }, [map, lugares]);

  useEffect(() => () => marcadores.current.forEach((m) => m.setMap(null)), []);

  const alternar = async (event: MouseEvent, alvo: Categoria) => {
    event.stopPropagation();

    if (categoria === alvo) {
      setCategoria(null);
      setLugares([]);
      return;
    }
    if (!fix || buscando) return;

    setBuscando(true);
    try {
      const resposta = await fetch(
        "https://places.googleapis.com/v1/places:searchNearby",
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-Goog-Api-Key": apiKey,
            // Máscara mínima — só nome e posição — porque é ela que decide o
            // tier de cobrança da requisição.
            "X-Goog-FieldMask": "places.displayName,places.location",
          },
          body: JSON.stringify({
            includedTypes: [alvo],
            maxResultCount: MAXIMO,
            languageCode: "pt-BR",
            locationRestriction: {
              circle: {
                center: { latitude: fix.lat, longitude: fix.lon },
                radius: RAIO_M,
              },
            },
          }),
        },
      );

      const dados: Resposta = await resposta.json();
      setLugares(
        (dados.places ?? []).flatMap((p) =>
          p.location?.latitude != null && p.location.longitude != null
            ? [
                {
                  nome: p.displayName?.text ?? "",
                  lat: p.location.latitude,
                  lng: p.location.longitude,
                },
              ]
            : [],
        ),
      );
      setCategoria(alvo);
    } catch (err) {
      console.error("[eclipse] falha ao buscar lugares próximos", err);
    } finally {
      setBuscando(false);
    }
  };

  // Só os botões: quem dá o lugar deles na tela é a coluna de ferramentas
  // do mapa, junto com zoom e recentrar.
  return (
    <>
      {(Object.keys(CATEGORIAS) as Categoria[]).map((alvo) => (
        <button
          key={alvo}
          className={`mapa__botao${categoria === alvo ? " mapa__botao--ativo" : ""}`}
          disabled={!fix || buscando}
          onClick={(e) => void alternar(e, alvo)}
        >
          {CATEGORIAS[alvo]}
        </button>
      ))}
    </>
  );
}
