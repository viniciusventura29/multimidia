import { useEffect, useRef, useState, type MouseEvent } from "react";
import {
  config as configMapLibre,
  LngLatBounds,
  Map as MapaGL,
  Marker,
  type GeoJSONSource,
} from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
// O worker do MapLibre é um arquivo à parte desde a v6, e a biblioteca o
// procura em `new URL("./maplibre-gl-worker.mjs", import.meta.url)` — que
// resolve para o diretório do bundle, onde ele não está. O worker 404 **em
// silêncio**: nenhum tile é decodificado, o estilo nunca termina de carregar e
// nenhum erro é emitido. Um mapa preto sem explicação nenhuma.
//
// `?worker&url` e não `?url`: o worker **importa** `maplibre-gl-shared.mjs`, e
// `?url` copiaria só o arquivo apontado, sem seguir os imports dele. Em dev
// isso passa (o Vite serve `node_modules` sob demanda) e no APK quebra, porque
// o protocolo do Tauri devolve o `index.html` para o arquivo que falta e o
// carregador de módulo recusa por MIME. Com `?worker&url` o Vite **empacota** o
// worker inteiro, dependências dentro, e devolve a URL do resultado.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";

configMapLibre.WORKER_URL = workerUrl;

import type { TileView } from "../../core/types";
import { metros } from "./geo";
import { Manobra } from "./manobra";
import { MapaContexto, NOSSO, useMapa } from "./mapaContexto";
import { Pois } from "./pois";
import { BuscarRota, RotaDesenhada } from "./rota";
import { falar } from "./voz";
import type { Fix, MapaState, Rota } from "./tipos";

/** Enquanto o GPS não fixa, o mapa nasce olhando para São Paulo. */
const CENTRO_PADRAO: [number, number] = [-46.6333, -23.5505];

/**
 * Os estilos do OpenFreeMap — sem chave, sem cota, sem cadastro.
 *
 * Trocar de fornecedor de tile agora é trocar estas duas linhas, e é isso que
 * abre a porta do mapa offline: um arquivo `.pmtiles` na memória do aparelho
 * entra aqui do mesmo jeito, e aí o mapa funciona em túnel e garagem.
 */
const ESTILOS = {
  dia: "https://tiles.openfreemap.org/styles/positron",
  noite: "https://tiles.openfreemap.org/styles/dark",
} as const;

/** Quantos pontos do caminho já andado manter desenhados. */
const RASTRO_MAXIMO = 120;

/** De quanto em quanto tempo chega uma posição nova. */
const INTERVALO_GPS_MS = 1000;

/** O nível de zoom de quem está dirigindo. */
const ZOOM_DE_RUA = 17;

/**
 * Interpola dois rumos pelo caminho mais curto.
 *
 * Sem isto, ir de 359° para 1° faria o mapa girar 358° para trás em vez de 2°
 * para a frente — o carro daria um pião na tela toda vez que cruzasse o norte.
 */
function interpolarRumo(de: number, para: number, t: number): number {
  const delta = ((para - de + 540) % 360) - 180;
  return (de + delta * t + 360) % 360;
}

/** A seta do carro. Um elemento do DOM, não um ícone do estilo — assim ela
 *  sobrevive à troca de tema sem nenhum cuidado especial. */
function setaDoCarro(): HTMLElement {
  const el = document.createElement("div");
  el.className = "mapa__carro";
  el.innerHTML =
    '<svg viewBox="0 0 24 24" width="30" height="30" aria-hidden="true">' +
    '<path d="M12 2 L20 21 L12 16.5 L4 21 Z" fill="#3ddc97" stroke="#07090d" ' +
    'stroke-width="1.6" stroke-linejoin="round"/></svg>';
  return el;
}

/**
 * Faz o mapa seguir o carro.
 *
 * O GPS entrega **uma posição por segundo**. Mandar a câmera direto para cada
 * uma faz o mapa teleportar 1x por segundo — era isso que dava a sensação de
 * travado. Aqui a câmera é redesenhada a cada quadro, interpolando entre a
 * leitura anterior e a atual. É o que todo navegador faz, e é a diferença entre
 * "um mapa que atualiza" e "um mapa que anda".
 *
 * A câmera é sempre de cima (sem inclinação — 3D atrapalhava mais que ajudava)
 * e não impõe zoom: o nível é do motorista, ajustado pelos botões, e forçá-lo a
 * cada quadro desfaria o ajuste.
 */
function SeguirCarro({
  fix,
  navegando,
  seguindo,
}: {
  fix: Fix | null;
  navegando: boolean;
  seguindo: boolean;
}) {
  const map = useMapa();
  const trecho = useRef<{ de: Fix; para: Fix; inicio: number } | null>(null);
  // O carro é um marcador ancorado no MUNDO, não um enfeite no centro da
  // tela: com o seguimento solto (dedo arrastando), ele fica parado na
  // geografia enquanto o mapa desliza por baixo — mexer a tela não pode
  // mexer o carro.
  const carro = useRef<Marker | null>(null);
  const pontos = useRef<[number, number][]>([]);
  // O quadro agendado, ou 0 = loop dormindo. O loop só roda enquanto há trecho
  // a percorrer: com o carro parado não chega trecho novo (zona morta) e o rAF
  // se auto-encerra — numa head unit, 60 movimentos de câmera por segundo à toa
  // é o maior gasto contínuo de CPU do painel.
  const quadro = useRef(0);
  const navegandoRef = useRef(navegando);
  const seguindoRef = useRef(seguindo);

  const desenhar = () => {
    const atual = trecho.current;
    if (!map || !atual) {
      quadro.current = 0;
      return;
    }

    // Trava em 1 quando a próxima leitura atrasa. Deixar passar continuaria
    // extrapolando o carro para longe do que se sabe; parar e esperar é
    // honesto — é exatamente o que o aparelho conhece.
    const t = Math.min(1, (performance.now() - atual.inicio) / INTERVALO_GPS_MS);

    const lng = atual.de.lon + (atual.para.lon - atual.de.lon) * t;
    const lat = atual.de.lat + (atual.para.lat - atual.de.lat) * t;
    const rumo = interpolarRumo(atual.de.heading, atual.para.heading, t);

    // O marcador anda sempre — é o carro no mundo. Com `rotationAlignment` no
    // mapa, a rotação é relativa ao norte geográfico, então funciona igual com
    // o mapa girando ou parado.
    mostrarCarro(map, carro, [lng, lat], rumo);

    // A câmera só acompanha se o motorista não estiver segurando o mapa —
    // mexer a câmera durante o gesto seria brigar com o dedo.
    if (seguindoRef.current) {
      map.jumpTo({
        center: [lng, lat],
        ...(navegandoRef.current ? { bearing: rumo } : {}),
      });
    }

    if (t >= 1) {
      // Chegou onde o aparelho conhece: dorme até o próximo fix acordar.
      quadro.current = 0;
      return;
    }
    quadro.current = requestAnimationFrame(desenhar);
  };

  const acordar = () => {
    if (!quadro.current) quadro.current = requestAnimationFrame(desenhar);
  };

  // Cada leitura abre um trecho novo a ser percorrido até a próxima chegar.
  useEffect(() => {
    if (!fix) return;

    // Zona morta: parado, o GPS oscila alguns metros a cada leitura. Sem isto,
    // cada oscilação abre um trecho novo e a câmera fica indo e voltando sob o
    // carro — o "samba". Só reposiciona a partir de ~6 m, que é movimento real.
    const anterior = trecho.current?.para;
    if (anterior && metros(anterior, fix) < 6) return;

    trecho.current = {
      de: anterior ?? fix,
      para: fix,
      inicio: performance.now(),
    };

    const ponto: [number, number] = [fix.lon, fix.lat];
    pontos.current = [...pontos.current, ponto].slice(-RASTRO_MAXIMO);
    rastro(map, pontos.current);
    acordar();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fix]);

  // Trocar de modo redesenha uma vez (o rumo muda) mesmo parado.
  useEffect(() => {
    navegandoRef.current = navegando;
    // Sair do modo seguir endireita o norte na hora — sem isto o mapa ficava
    // travado girado com o rumo da última leitura, parecendo quebrado.
    if (!navegando) map?.jumpTo({ bearing: 0, pitch: 0 });
    acordar();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [navegando]);

  // Voltar a seguir redesenha uma vez — é o que o botão "recentrar" faz.
  useEffect(() => {
    seguindoRef.current = seguindo;
    if (seguindo) acordar();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seguindo]);

  useEffect(() => {
    if (!map) return;

    const conhecido = trecho.current?.para;
    if (conhecido) {
      mostrarCarro(map, carro, [conhecido.lon, conhecido.lat], conhecido.heading);
    }
    rastro(map, pontos.current);

    acordar();
    return () => {
      cancelAnimationFrame(quadro.current);
      quadro.current = 0;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map]);

  useEffect(
    () => () => {
      carro.current?.remove();
    },
    [],
  );

  return null;
}

/**
 * Põe (ou move) o carro no mapa.
 *
 * O marcador só entra no mapa **depois** de ter uma posição, e é por isso que
 * ele não é criado junto com o mapa: um `Marker` do MapLibre lê o próprio
 * `LngLat` ao ser anexado, e anexar sem posição estoura. Com o Google isso
 * passava — o marcador nascia sem lugar nenhum e ficava invisível. Aqui não
 * passa, e o resultado prático é melhor: sem posição, sem seta. Um carro
 * desenhado num centro padrão seria uma mentira do tamanho de uma cidade.
 */
function mostrarCarro(
  map: MapaGL | null,
  carro: { current: Marker | null },
  onde: [number, number],
  rumo: number,
) {
  if (!map) return;

  carro.current ??= new Marker({
    element: setaDoCarro(),
    rotationAlignment: "map",
  });
  carro.current.setLngLat(onde).setRotation(rumo);
  carro.current.addTo(map);
}

/** Redesenha o caminho já andado. */
function rastro(map: MapaGL | null, pontos: [number, number][]) {
  const fonte = map?.getSource<GeoJSONSource>(`${NOSSO}rastro`);
  if (!fonte) return;

  fonte.setData({
    type: "Feature",
    properties: {},
    geometry: { type: "LineString", coordinates: pontos },
  });
}

/**
 * Botões de câmera: zoom, visão geral da rota e recentrar.
 *
 * Existem porque o mapa nasce sem controle nenhum — os padrões são pequenos
 * demais para dedo de motorista — e porque arrastar o mapa solta o seguimento
 * (senão a câmera brigava com o dedo): alguém precisa oferecer o caminho de
 * volta.
 *
 * No tile pequeno sobra só o recentrar, e mesmo ele só aparece depois de o dedo
 * arrastar o mapa: dirigindo, um mapinha do tamanho de um cartão com seis
 * botões em cima não é controle, é obstáculo. O resto está a um toque de
 * distância, na tela cheia.
 */
function Controles({
  seguindo,
  temFix,
  rota,
  expandido,
  aoRecentrar,
  aoSoltar,
}: {
  seguindo: boolean;
  temFix: boolean;
  rota: Rota | null;
  expandido: boolean;
  aoRecentrar: () => void;
  aoSoltar: () => void;
}) {
  const map = useMapa();
  if (!map) return null;

  const ajustarZoom = (event: MouseEvent, delta: number) => {
    event.stopPropagation();
    map.setZoom(map.getZoom() + delta);
  };

  const verRota = (event: MouseEvent) => {
    event.stopPropagation();
    if (!rota?.pontos.length) return;
    // Enquadrar solta o seguimento — senão o próximo quadro devolveria a
    // câmera para cima do carro e o enquadramento duraria um piscar.
    aoSoltar();
    const limites = new LngLatBounds();
    for (const [lat, lng] of rota.pontos) limites.extend([lng, lat]);
    map.fitBounds(limites, { padding: 48, bearing: 0 });
  };

  const recentrar = (event: MouseEvent) => {
    event.stopPropagation();
    // Devolve o zoom de rua junto: quem recentra quer voltar a dirigir com o
    // mapa, não continuar no zoom em que largou o enquadramento.
    map.setZoom(ZOOM_DE_RUA);
    aoRecentrar();
  };

  // Só os botões: quem dá o lugar deles na tela é a coluna de ferramentas.
  return (
    <>
      {expandido && (
        <>
          <button
            className="mapa__botao"
            onClick={(e) => ajustarZoom(e, 1)}
            aria-label="Aproximar"
          >
            +
          </button>
          <button
            className="mapa__botao"
            onClick={(e) => ajustarZoom(e, -1)}
            aria-label="Afastar"
          >
            −
          </button>
          {rota && (
            <button className="mapa__botao" onClick={verRota}>
              rota
            </button>
          )}
        </>
      )}
      {!seguindo && temFix && (
        <button className="mapa__recentrar" onClick={recentrar}>
          recentrar
        </button>
      )}
    </>
  );
}

/**
 * Cria o mapa e o mantém vivo.
 *
 * A troca de tema é `setStyle`, que substitui o documento de estilo inteiro e
 * levaria junto a rota e o rastro. O `transformStyle` reconduz para o estilo
 * novo tudo que for nosso (prefixo `eclipse-`) — é a resposta certa para a
 * mesma armadilha que o `colorScheme` do Google criava, e que ali só tinha
 * remendo. Marcadores (carro, POIs) são DOM e não passam por isso.
 */
function useMapaGL(noite: boolean, aoArrastar: () => void) {
  const caixa = useRef<HTMLDivElement | null>(null);
  const [map, setMap] = useState<MapaGL | null>(null);
  // Qual estilo já está no mapa. Sem isto o efeito de tema disparava um
  // `setStyle` completo no primeiro render — recarregando um estilo idêntico
  // ao que o construtor acabou de aplicar.
  const estiloAtual = useRef(noite ? ESTILOS.noite : ESTILOS.dia);
  const arrastar = useRef(aoArrastar);
  arrastar.current = aoArrastar;

  useEffect(() => {
    if (!caixa.current) return;

    const mapa = new MapaGL({
      container: caixa.current,
      style: noite ? ESTILOS.noite : ESTILOS.dia,
      center: CENTRO_PADRAO,
      zoom: ZOOM_DE_RUA,
      // Sem 3D e sem rotação pelo dedo: quem gira o mapa é o rumo do carro.
      pitch: 0,
      pitchWithRotate: false,
      dragRotate: false,
      // A atribuição do OpenStreetMap não é enfeite, é a licença — fica.
      attributionControl: { compact: true },
    });

    mapa.on("dragstart", () => arrastar.current());
    // Falha de tile ou de estilo não pode ser silenciosa: sem isto, um mapa
    // que não pinta é indistinguível de um mapa vazio.
    mapa.on("error", (e) => console.error("[eclipse] maplibre", e.error ?? e));

    mapa.on("load", () => {
      mapa.addSource(`${NOSSO}rastro`, {
        type: "geojson",
        data: { type: "FeatureCollection", features: [] },
      });
      mapa.addLayer({
        id: `${NOSSO}rastro`,
        type: "line",
        source: `${NOSSO}rastro`,
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#a06bff",
          "line-width": 6,
          "line-opacity": 0.9,
        },
      });
      setMap(mapa);
    });

    return () => {
      setMap(null);
      mapa.remove();
    };
    // O estilo inicial é só o de partida; daí em diante quem troca é o efeito
    // abaixo. Recriar o mapa a cada pôr do sol é exatamente o que se quer evitar.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // O tema acompanha o sol (calculado no Rust — ver `sol.rs`).
  useEffect(() => {
    const alvo = noite ? ESTILOS.noite : ESTILOS.dia;
    if (!map || alvo === estiloAtual.current) return;
    estiloAtual.current = alvo;

    map.setStyle(alvo, {
      transformStyle: (anterior, novo) => {
        if (!anterior) return novo;

        const fontes = Object.fromEntries(
          Object.entries(anterior.sources).filter(([id]) => id.startsWith(NOSSO)),
        );
        const camadas = anterior.layers.filter((c) => c.id.startsWith(NOSSO));

        return {
          ...novo,
          sources: { ...novo.sources, ...fontes },
          layers: [...novo.layers, ...camadas],
        };
      },
    });
  }, [map, noite]);

  return { caixa, map };
}

/**
 * O mapa.
 *
 * Como a UI roda num WebView, ele é um elemento comum da página: o mesmo
 * componente serve de widget e de tela cheia, e a transição é só o CSS mudando
 * de tamanho. Era exatamente isso que o SDK nativo do Android não permitiria.
 *
 * Quem desenha é o MapLibre sobre tiles do OpenStreetMap: sem chave, sem cota,
 * e com o tema sendo um documento que a gente controla em vez de dois valores
 * que o Google aceita. O Google continua no que faz melhor aqui — buscar
 * endereço, traçar a rota com trânsito e listar postos.
 *
 * O que muda entre os dois tamanhos não é o mapa, é quanta tralha vive por
 * cima dele — ver `Controles`.
 */
function Painel({
  data,
  status,
  expandido,
}: TileView<MapaState> & { expandido: boolean }) {
  const [navegando, setNavegando] = useState(true);
  // A câmera está colada no carro? Arrastar o mapa solta; "recentrar" volta.
  // Cada instância do tile (grid e tela cheia) tem a sua — são câmeras
  // independentes, e é isso que se quer.
  const [seguindo, setSeguindo] = useState(true);

  // Segurar o mapa é assumir a câmera: o seguimento solta na hora e o botão
  // "recentrar" vira o caminho de volta.
  const { caixa, map } = useMapaGL(data?.noite ?? true, () => setSeguindo(false));

  // O Rust decide o que e quando falar; aqui só se pronuncia.
  useEffect(() => {
    falar(data?.fala ?? null);
  }, [data?.fala]);

  return (
    <div className={`mapa mapa--vivo${status === "degraded" ? " mapa--sem-sinal" : ""}`}>
      <div className="mapa__canvas" ref={caixa} />

      <MapaContexto.Provider value={map}>
        <SeguirCarro fix={data?.fix ?? null} navegando={navegando} seguindo={seguindo} />
        <RotaDesenhada rota={data?.rota ?? null} />

        {/* Uma coluna só para todos os botões de mapa: empilhados não
            disputam lugar com a manobra (topo) nem com a busca (embaixo). */}
        <div
          className="mapa__ferramentas"
          onClick={(e) => e.stopPropagation()}
        >
          {expandido && data?.apiKey && (
            <Pois fix={data.fix} apiKey={data.apiKey} />
          )}
          <Controles
            seguindo={seguindo}
            temFix={Boolean(data?.fix)}
            rota={data?.rota ?? null}
            expandido={expandido}
            aoRecentrar={() => setSeguindo(true)}
            aoSoltar={() => setSeguindo(false)}
          />
          {expandido && (
            <button
              className="mapa__botao"
              onClick={(e) => {
                e.stopPropagation();
                setNavegando((v) => !v);
              }}
            >
              {navegando ? "norte no topo" : "girar com o carro"}
            </button>
          )}
        </div>
      </MapaContexto.Provider>

      {expandido &&
        data &&
        (data.apiKey ? (
          <BuscarRota
            fix={data.fix}
            rota={data.rota}
            apiKey={data.apiKey}
            buscando={data.buscando}
            erro={data.erro}
          />
        ) : (
          // O mapa não precisa de chave nenhuma; a busca de endereço precisa.
          // Dizer isso é melhor que deixar o motorista procurar um campo que
          // não existe.
          <span className="mapa__aviso">
            sem chave do Google — mapa e posição sim, busca de destino não
          </span>
        ))}

      {expandido && data?.progresso && <Manobra progresso={data.progresso} />}
    </div>
  );
}

/**
 * O mapa no painel: só o caminho, o carro e — depois de arrastar — o recentrar.
 */
export function Mapa(props: TileView<MapaState>) {
  return <Painel {...props} expandido={false} />;
}

/** O mapa em tela cheia: aqui cabe tudo — busca, POIs, zoom e manobra. */
export function MapaCheio(props: TileView<MapaState>) {
  return <Painel {...props} expandido />;
}
