import { useEffect, useRef, useState, type MouseEvent } from "react";
import { Marker } from "maplibre-gl";

import { useMapa } from "./mapaContexto";

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
  const map = useMapa();
  const [categoria, setCategoria] = useState<Categoria | null>(null);
  const [buscando, setBuscando] = useState(false);
  const [lugares, setLugares] = useState<Lugar[]>([]);
  const marcadores = useRef<Marker[]>([]);

  // Redesenha os pinos quando a lista muda. Marcador do MapLibre é elemento do
  // DOM, então — ao contrário das camadas — ele atravessa a troca de tema sem
  // nenhum cuidado especial.
  useEffect(() => {
    marcadores.current.forEach((m) => m.remove());
    marcadores.current = [];
    if (!map) return;

    marcadores.current = lugares.map((lugar) => {
      const pino = document.createElement("div");
      pino.className = "mapa__pino";
      pino.title = lugar.nome;
      return new Marker({ element: pino }).setLngLat([lugar.lng, lugar.lat]).addTo(map);
    });
  }, [map, lugares]);

  useEffect(() => () => marcadores.current.forEach((m) => m.remove()), []);

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
