import { createContext, useContext } from "react";
import type { Map } from "maplibre-gl";

/**
 * A instância do mapa, para quem desenha por cima dele.
 *
 * O wrapper React do Google dava isto de graça (`useMap`). O MapLibre é uma
 * biblioteca imperativa e não tem opinião sobre React, então o contexto é
 * nosso — o que também é o motivo de a troca ter sido barata: os componentes
 * que pintam em cima do mapa (carro, rota, POIs) já falavam com uma instância
 * imperativa, e continuam falando.
 *
 * `null` enquanto o mapa não terminou de carregar o estilo: antes disso não dá
 * para adicionar fonte nem camada, e tentar é erro em tempo de execução.
 */
export const MapaContexto = createContext<Map | null>(null);

export function useMapa(): Map | null {
  return useContext(MapaContexto);
}

/**
 * Prefixo de tudo que é nosso dentro do estilo.
 *
 * Trocar de estilo (dia/noite) substitui o documento inteiro e levaria junto a
 * rota e o rastro. O `transformStyle` do `setStyle` reconduz para o estilo novo
 * exatamente o que tiver este prefixo — ver `mapa.tsx`. É a mesma armadilha do
 * `colorScheme` do Google, que destruía a instância inteira; aqui ela tem
 * conserto declarado em um lugar só.
 */
export const NOSSO = "eclipse-";
