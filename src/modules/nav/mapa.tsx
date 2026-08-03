import { useEffect, useRef, useState, type MouseEvent } from "react";
import { APIProvider, Map, useMap } from "@vis.gl/react-google-maps";

import type { TileView } from "../../core/types";
import { Manobra } from "./manobra";
import { Pois } from "./pois";
import { BuscarRota } from "./rota";
import { falar } from "./voz";
import type { Fix, MapaState, Rota } from "./tipos";

/** Enquanto o GPS não fixa, o mapa nasce olhando para São Paulo. */
const CENTRO_PADRAO = { lat: -23.5505, lng: -46.6333 };

/** Quantos pontos do caminho já andado manter desenhados. */
const RASTRO_MAXIMO = 120;

/** De quanto em quanto tempo chega uma posição nova. */
const INTERVALO_GPS_MS = 1000;

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

/** Distância aproximada em metros entre dois pontos (equirretangular — a
 *  precisão sobra para distinguir jitter de GPS de movimento real). */
function metros(a: Fix, b: Fix): number {
  const R = 6_371_000;
  const rad = Math.PI / 180;
  const dLat = (b.lat - a.lat) * rad;
  const dLon = (b.lon - a.lon) * rad * Math.cos(((a.lat + b.lat) / 2) * rad);
  return R * Math.hypot(dLat, dLon);
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
 * Fica num componente separado porque só quem está dentro do `APIProvider`
 * consegue pegar a instância do mapa com `useMap`.
 *
 * A câmera é sempre de cima (sem `tilt` — 3D atrapalhava mais que ajudava) e
 * não impõe `zoom`: o nível é do motorista, ajustado pelos botões, e forçá-lo
 * a cada quadro desfaria o ajuste. `heading` só tem efeito em mapa
 * **vetorial**, que exige um Map ID configurado como tal.
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
  const map = useMap();
  const trecho = useRef<{ de: Fix; para: Fix; inicio: number } | null>(null);
  const rastro = useRef<google.maps.Polyline | null>(null);
  // O carro é um marcador ancorado no MUNDO, não um enfeite no centro da
  // tela: com o seguimento solto (dedo arrastando), ele fica parado na
  // geografia enquanto o mapa desliza por baixo — mexer a tela não pode
  // mexer o carro.
  const carro = useRef<google.maps.Marker | null>(null);
  const pontos = useRef<google.maps.LatLngLiteral[]>([]);
  // O quadro agendado, ou 0 = loop dormindo. O loop só roda enquanto há trecho
  // a percorrer: com o carro parado não chega trecho novo (zona morta) e o rAF
  // se auto-encerra — numa head unit, 60 moveCamera/s à toa é o maior gasto
  // contínuo de CPU do painel.
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

    const posicao = {
      lat: atual.de.lat + (atual.para.lat - atual.de.lat) * t,
      lng: atual.de.lon + (atual.para.lon - atual.de.lon) * t,
    };
    const rumo = interpolarRumo(atual.de.heading, atual.para.heading, t);

    // O marcador anda sempre — é o carro no mundo. A rotação do símbolo é
    // relativa ao norte do mapa, então funciona igual girando ou não.
    carro.current?.setPosition(posicao);
    const icone = carro.current?.getIcon() as google.maps.Symbol | undefined;
    if (icone && icone.rotation !== rumo) {
      carro.current?.setIcon({ ...icone, rotation: rumo });
    }

    // A câmera só acompanha se o motorista não estiver segurando o mapa —
    // mexer a câmera durante o gesto seria brigar com o dedo.
    if (seguindoRef.current) {
      map.moveCamera({
        center: posicao,
        ...(navegandoRef.current ? { heading: rumo } : {}),
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

    pontos.current = [...pontos.current, { lat: fix.lat, lng: fix.lon }].slice(
      -RASTRO_MAXIMO,
    );
    rastro.current?.setPath(pontos.current);
    acordar();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fix]);

  // Trocar de modo redesenha uma vez (heading muda) mesmo parado.
  useEffect(() => {
    navegandoRef.current = navegando;
    // Sair do modo seguir endireita o norte na hora — sem isto o mapa ficava
    // travado girado com o rumo da última leitura, parecendo quebrado.
    if (!navegando) map?.moveCamera({ heading: 0, tilt: 0 });
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

    rastro.current ??= new google.maps.Polyline({
      strokeColor: "#a06bff",
      strokeOpacity: 0.9,
      strokeWeight: 6,
    });

    carro.current ??= new google.maps.Marker({
      // A seta dos navegadores, na cor de destaque do painel, por cima da
      // rota e do rastro.
      icon: {
        path: google.maps.SymbolPath.FORWARD_CLOSED_ARROW,
        scale: 7,
        fillColor: "#3ddc97",
        fillOpacity: 1,
        strokeColor: "#07090d",
        strokeWeight: 2,
        rotation: 0,
      },
      zIndex: 3,
    });

    // Religa em vez de criar preso ao mapa: a troca de tema (dia/noite)
    // destrói e recria a instância, e os overlays sobrevivem por fora — sem
    // isto o rastro e o carro sumiriam no primeiro pôr do sol.
    rastro.current.setMap(map);
    rastro.current.setPath(pontos.current);
    carro.current.setMap(map);
    const conhecido = trecho.current?.para;
    if (conhecido) carro.current.setPosition({ lat: conhecido.lat, lng: conhecido.lon });

    acordar();
    return () => {
      cancelAnimationFrame(quadro.current);
      quadro.current = 0;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map]);

  useEffect(
    () => () => {
      rastro.current?.setMap(null);
      carro.current?.setMap(null);
    },
    [],
  );

  return null;
}

/**
 * Botões de câmera: zoom, visão geral da rota e recentrar.
 *
 * Existem porque o mapa nasce com `disableDefaultUI` — os controles do Google
 * são pequenos demais para dedo de motorista — e porque arrastar o mapa agora
 * solta o seguimento (senão a câmera brigava com o dedo): alguém precisa
 * oferecer o caminho de volta.
 */
function Controles({
  seguindo,
  temFix,
  rota,
  aoRecentrar,
  aoSoltar,
}: {
  seguindo: boolean;
  temFix: boolean;
  rota: Rota | null;
  aoRecentrar: () => void;
  aoSoltar: () => void;
}) {
  const map = useMap();
  if (!map) return null;

  const ajustarZoom = (event: MouseEvent, delta: number) => {
    event.stopPropagation();
    map.setZoom((map.getZoom() ?? 18) + delta);
  };

  const verRota = (event: MouseEvent) => {
    event.stopPropagation();
    if (!rota) return;
    // Enquadrar solta o seguimento — senão o próximo quadro devolveria a
    // câmera para cima do carro e o enquadramento duraria um piscar.
    aoSoltar();
    const limites = new google.maps.LatLngBounds();
    for (const [lat, lng] of rota.pontos) limites.extend({ lat, lng });
    map.fitBounds(limites, 48);
  };

  const recentrar = (event: MouseEvent) => {
    event.stopPropagation();
    // Devolve o zoom de rua junto: quem recentra quer voltar a dirigir com o
    // mapa, não continuar no zoom em que largou o enquadramento.
    map.moveCamera({ zoom: 18 });
    aoRecentrar();
  };

  // Só os botões: quem dá o lugar deles na tela é a coluna de ferramentas.
  return (
    <>
      <button className="mapa__botao" onClick={(e) => ajustarZoom(e, 1)} aria-label="Aproximar">
        +
      </button>
      <button className="mapa__botao" onClick={(e) => ajustarZoom(e, -1)} aria-label="Afastar">
        −
      </button>
      {rota && (
        <button className="mapa__botao" onClick={verRota}>
          rota
        </button>
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
 * O mapa.
 *
 * Como a UI roda num WebView, ele é um elemento comum da página: o mesmo
 * componente serve de widget e de tela cheia, e a transição é só o CSS mudando
 * de tamanho. Era exatamente isso que o SDK nativo do Android não permitiria.
 *
 * A guiagem (rota, manobras, voz, recálculo) é própria — ver `guia.rs` e
 * `rota.tsx`. O que continua fora de alcance é orientação de faixa e trânsito
 * ao vivo desviando a rota: isso é o Navigation SDK, que é enterprise.
 */
export function Mapa({ data, status }: TileView<MapaState>) {
  const [navegando, setNavegando] = useState(true);
  // A câmera está colada no carro? Arrastar o mapa solta; "recentrar" volta.
  // Cada instância do tile (grid e tela cheia) tem a sua — são câmeras
  // independentes, e é isso que se quer.
  const [seguindo, setSeguindo] = useState(true);

  // O Rust decide o que e quando falar; aqui só se pronuncia.
  useEffect(() => {
    falar(data?.fala ?? null);
  }, [data?.fala]);

  if (!data?.apiKey) {
    return (
      <div className="mapa">
        <span className="mapa__marca">mapa</span>
      </div>
    );
  }

  const modoNavegacao = navegando && Boolean(data.mapId);

  return (
    <div className={`mapa mapa--vivo${status === "degraded" ? " mapa--sem-sinal" : ""}`}>
      <APIProvider apiKey={data.apiKey} language="pt-BR" region="BR">
        <Map
          className="mapa__canvas"
          defaultCenter={CENTRO_PADRAO}
          defaultZoom={18}
          mapId={data.mapId ?? undefined}
          // O tema acompanha o sol (calculado no Rust — ver `sol.rs`). Só o
          // canvas clareia de dia; o resto do painel continua escuro.
          colorScheme={data.noite ? "DARK" : "LIGHT"}
          disableDefaultUI
          gestureHandling="greedy"
          reuseMaps
          // Segurar o mapa é assumir a câmera: o seguimento solta na hora e
          // o botão "recentrar" vira o caminho de volta.
          onDragstart={() => setSeguindo(false)}
        />
        <SeguirCarro fix={data.fix} navegando={modoNavegacao} seguindo={seguindo} />
        {/* Uma coluna só para todos os botões de mapa: empilhados não
            disputam lugar com a manobra (topo) nem com a busca (embaixo). */}
        <div className="mapa__ferramentas" onClick={(e) => e.stopPropagation()}>
          <Pois fix={data.fix} apiKey={data.apiKey} />
          <Controles
            seguindo={seguindo}
            temFix={Boolean(data.fix)}
            rota={data.rota}
            aoRecentrar={() => setSeguindo(true)}
            aoSoltar={() => setSeguindo(false)}
          />
        </div>
        <BuscarRota
          fix={data.fix}
          rota={data.rota}
          recalcular={data.progresso?.recalcular ?? false}
          apiKey={data.apiKey}
        />
      </APIProvider>

      {data.progresso && <Manobra progresso={data.progresso} />}

      {data.mapId ? (
        <button
          className="mapa__modo"
          onClick={(e) => {
            e.stopPropagation();
            setNavegando((v) => !v);
          }}
        >
          {navegando ? "norte no topo" : "girar com o carro"}
        </button>
      ) : (
        // Sem Map ID não há como girar o mapa. Dizer isso é melhor que
        // deixar o usuário achar que o modo navegação está quebrado.
        <span className="mapa__aviso">sem Map ID vetorial — mapa chapado</span>
      )}
    </div>
  );
}
