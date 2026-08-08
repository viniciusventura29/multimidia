import { useEffect, useRef, useState, type FormEvent, type MouseEvent } from "react";
import { useMap, useMapsLibrary } from "@vis.gl/react-google-maps";

import { dispatchAction } from "../../core/actions";
import { destacar, useSugestoes } from "./sugestoes";
import { calarSe } from "./voz";
import type { Fix, Rota } from "./tipos";

const NAV = "nav";

/**
 * Separa a instrução do complemento.
 *
 * O Directions manda HTML: a manobra em texto corrido e, quando há, uma
 * referência visual dentro de um `<div>` — "Você verá a Drogaria à esquerda".
 * Colar as duas numa frase só dá o troço ilegível que aparecia antes. A manobra
 * é o que o motorista precisa; a referência é apoio.
 */
function separarInstrucao(html: string): { instrucao: string; detalhe: string | null } {
  const limpar = (t: string) =>
    t.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();

  const corte = html.indexOf("<div");
  if (corte === -1) return { instrucao: limpar(html), detalhe: null };

  const detalhe = limpar(html.slice(corte));
  return {
    instrucao: limpar(html.slice(0, corte)),
    detalhe: detalhe || null,
  };
}



/**
 * Pedir uma rota ao Google e entregá-la ao Rust.
 *
 * O `DirectionsService` mora aqui porque é parte do SDK do mapa. Mas a rota não
 * fica aqui: assim que chega, vai para o Rust, que é onde a posição vive. É lá
 * que dá para responder "quanto falta" e "saí do caminho" — perguntas que
 * precisam da rota e do GPS ao mesmo tempo.
 *
 * Num hook porque dois componentes precisam buscar: quem digita o destino, e
 * quem obedece à ordem de recalcular vinda do Rust.
 */
function useBuscaDeRota() {
  const biblioteca = useMapsLibrary("routes");
  const [buscando, setBuscando] = useState(false);
  const [erro, setErro] = useState<string | null>(null);

  const buscar = async (
    alvo: string | google.maps.Place | { placeId: string },
    origem: Fix,
  ) => {
    if (!biblioteca) return;

    setBuscando(true);
    setErro(null);

    try {
      const servico = new biblioteca.DirectionsService();
      const resposta = await servico.route({
        origin: { lat: origem.lat, lng: origem.lon },
        destination: alvo,
        travelMode: google.maps.TravelMode.DRIVING,
        // Pede a duração já considerando o trânsito do momento. Sem isto o
        // tempo estimado é o de uma via vazia, que ninguém encontra.
        drivingOptions: {
          departureTime: new Date(),
          trafficModel: google.maps.TrafficModel.BEST_GUESS,
        },
      });

      const perna = resposta.routes[0].legs[0];

      dispatchAction(NAV, {
        acao: "rota",
        rota: {
          destino: perna.end_address ?? (typeof alvo === "string" ? alvo : ""),
          pontos: resposta.routes[0].overview_path.map((p) => [p.lat(), p.lng()]),
          passos: perna.steps.map((passo) => {
            const { instrucao, detalhe } = separarInstrucao(passo.instructions ?? "");
            return {
              instrucao,
              detalhe,
              distanciaM: passo.distance?.value ?? 0,
              manobra: passo.maneuver ?? null,
            };
          }),
          distanciaTotalM: perna.distance?.value ?? 0,
          // `duration_in_traffic` só vem quando o Google tem dados de trânsito
          // para o horário; fora disso, a duração normal.
          duracaoTotalS: perna.duration_in_traffic?.value ?? perna.duration?.value ?? 0,
        },
      });
    } catch {
      // A mensagem do Google é técnica demais para um painel de carro.
      setErro("não achei esse endereço");
    } finally {
      setBuscando(false);
    }
  };

  return { buscar, buscando, erro };
}

/**
 * A rota desenhada no mapa, e a obediência ao pedido de recálculo.
 *
 * Separado da busca porque os dois têm tempos de vida diferentes: o campo de
 * destino só existe em tela cheia, mas a linha e o recálculo precisam continuar
 * funcionando com o mapa encolhido no canto do painel — é justamente aí que o
 * motorista está dirigindo.
 *
 * Não desenha nada de HTML: é só um overlay imperativo com um lugar para morar.
 */
export function RotaViva({
  fix,
  rota,
  recalcular,
}: {
  fix: Fix | null;
  rota: Rota | null;
  /** O Rust pediu outra rota. Só a instância compacta obedece — ver `mapa.tsx`. */
  recalcular: boolean;
}) {
  const map = useMap();
  const { buscar, buscando } = useBuscaDeRota();
  const desenho = useRef<google.maps.Polyline | null>(null);
  // Que traçado já está desenhado. O envelope do Rust chega a cada leitura de
  // GPS com um objeto novinho em folha, então comparar identidade acusaria
  // mudança 1x por segundo e redesenharia a rota inteira à toa — numa head unit
  // isso é gasto contínuo de CPU para pintar exatamente os mesmos pixels.
  const desenhado = useRef<string | null>(null);

  // Desenha a rota recebida de volta do Rust — não a que acabou de ser buscada.
  // Assim o que aparece na tela é sempre o que o Rust está usando para guiar.
  useEffect(() => {
    if (!map) return;

    desenho.current ??= new google.maps.Polyline({
      strokeColor: "#4da3ff",
      strokeOpacity: 0.75,
      strokeWeight: 9,
      zIndex: 1,
    });

    // Religa em vez de criar presa ao mapa: a troca de tema (dia/noite)
    // destrói e recria a instância, e a rota precisa sobreviver a isso.
    desenho.current.setMap(map);

    const assinatura = rota ? `${rota.destino}:${rota.pontos.length}` : null;
    if (assinatura === desenhado.current) return;
    desenhado.current = assinatura;

    desenho.current.setPath(
      (rota?.pontos ?? []).map(([lat, lng]) => ({ lat, lng })),
    );
  }, [map, rota]);

  useEffect(() => () => desenho.current?.setMap(null), []);

  // O Rust avisa quando saímos do caminho tempo suficiente. A busca refeita
  // parte de onde o carro está agora, para o mesmo destino.
  useEffect(() => {
    if (!recalcular || !rota || !fix || buscando) return;
    void buscar(rota.destino, fix);
    // `buscando` fora das dependências de propósito: incluí-lo dispararia uma
    // segunda busca assim que a primeira terminasse.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recalcular, rota?.destino]);

  return null;
}

/**
 * O campo de destino, as sugestões e a rota em curso.
 *
 * Só existe em tela cheia: digitar endereço num tile do tamanho de um cartão
 * é briga perdida, e era metade da poluição do mapa pequeno.
 */
export function BuscarRota({
  fix,
  rota,
  apiKey,
}: {
  fix: Fix | null;
  rota: Rota | null;
  apiKey: string;
}) {
  const [destino, setDestino] = useState("");
  const { buscar, buscando, erro } = useBuscaDeRota();
  const { sugestoes, limpar } = useSugestoes(destino, apiKey, fix);

  const tracar = async (event: FormEvent) => {
    event.preventDefault();
    event.stopPropagation();

    // Enter sem escolher nada aproveita a primeira sugestão: é o que o motorista
    // espera, e o lugar exato dá rota melhor que o texto solto.
    const primeira = sugestoes[0];
    if (primeira) {
      await escolher(primeira.placeId);
      return;
    }

    const alvo = destino.trim();
    if (!alvo || !fix) return;

    await buscar(alvo, fix);
    setDestino("");
    limpar();
  };


  const escolher = async (placeId: string) => {
    if (!fix) return;
    // Vai pelo `placeId`, não pelo texto: é o lugar exato que o motorista tocou,
    // sem o Google ter que adivinhar de novo a partir da frase.
    await buscar({ placeId }, fix);
    setDestino("");
    limpar();
  };

  const cancelar = (event: MouseEvent) => {
    event.stopPropagation();
    calarSe();
    dispatchAction(NAV, { acao: "cancelar" });
  };

  if (rota) {
    return (
      <div className="rota__ativa" onClick={(e) => e.stopPropagation()}>
        <span className="rota__destino">{rota.destino}</span>
        <button className="rota__cancelar" onClick={cancelar}>
          encerrar
        </button>
      </div>
    );
  }

  return (
    <div className="rota__caixa" onClick={(e) => e.stopPropagation()}>
      {sugestoes.length > 0 && (
        <ul className="sugestoes">
          {sugestoes.map((s) => (
            <li key={s.placeId}>
              <button className="sugestao" onClick={() => void escolher(s.placeId)}>
                <span className="sugestao__pino" aria-hidden>
                  ⌖
                </span>
                <span className="sugestao__texto">
                  <span className="sugestao__principal">
                    {destacar(s.principal, s.destaques).map((parte, i) =>
                      parte.forte ? (
                        <strong key={i}>{parte.trecho}</strong>
                      ) : (
                        <span key={i}>{parte.trecho}</span>
                      ),
                    )}
                  </span>
                  <span className="sugestao__complemento">{s.complemento}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}

      <form className="rota__busca" onSubmit={tracar}>
        <input
          className="rota__campo"
          value={destino}
          onChange={(e) => setDestino(e.target.value)}
          placeholder={erro ?? "para onde?"}
          aria-label="Destino"
        />
        <button
          className="rota__ir"
          type="submit"
          disabled={!destino.trim() || buscando || !fix}
        >
          {buscando ? "…" : "ir"}
        </button>
      </form>
    </div>
  );
}

