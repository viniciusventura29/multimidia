import { memo, Suspense, useCallback, useState } from "react";

import { useModuleEnvelope } from "../core/moduleStore";
import type { AnyTileSpec, Status } from "../core/types";
import { Barreira } from "./Barreira";
import { TILES } from "./registry";
import { Tile } from "./Tile";

/**
 * O corpo de um tile cujo chunk ainda não chegou. A moldura (título, ícone,
 * área do grid) já está pintada — o metadado é eager —, então isto é só o miolo
 * respirando por um instante, na mesma linguagem do estado `loading`, sem
 * pulo de layout (as áreas do grid são fixas no CSS).
 */
const TileSkeleton = () => <div className="tile__skeleton" aria-hidden />;

/**
 * Um quadro do painel, assinado direto no módulo do qual ele lê.
 *
 * Cada tile acorda só com eventos do próprio módulo: um tick de RPM redesenha
 * os mostradores do OBD e mais nada — o mapa e a música nem ficam sabendo. É o
 * `memo` + assinatura por módulo que garantem isso; antes, o estado inteiro
 * descia por props e qualquer evento re-renderizava o painel todo.
 */
const TileHost = memo(function TileHost({
  spec,
  coberto,
  aoExpandir,
}: {
  spec: AnyTileSpec;
  /** A tela cheia deste mesmo tile está aberta por cima. */
  coberto: boolean;
  aoExpandir: (id: string) => void;
}) {
  const envelope = useModuleEnvelope(spec.module);
  const status: Status =
    envelope?.status ?? (spec.estatico ? "ready" : "loading");
  const reason = envelope?.reason ?? null;
  const Conteudo = spec.Compact;

  return (
    <Tile
      title={spec.title}
      area={spec.area}
      icon={spec.icon}
      status={status}
      reason={reason}
      onExpand={spec.Expanded ? () => aoExpandir(spec.id) : undefined}
    >
      <Barreira titulo={spec.title}>
        <Suspense fallback={<TileSkeleton />}>
          <Conteudo
            data={envelope?.data ?? null}
            status={status}
            reason={reason}
            coberto={coberto}
          />
        </Suspense>
      </Barreira>
    </Tile>
  );
});

function Expandido({
  spec,
  aoFechar,
}: {
  spec: AnyTileSpec;
  aoFechar: () => void;
}) {
  const envelope = useModuleEnvelope(spec.module);
  const status: Status =
    envelope?.status ?? (spec.estatico ? "ready" : "loading");
  const reason = envelope?.reason ?? null;
  const Conteudo = spec.Expanded!;

  return (
    <div className="overlay" onClick={aoFechar}>
      <section
        className={`overlay__painel overlay__painel--${status}`}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="overlay__head">
          <h2 className="overlay__titulo">
            {spec.icon && (
              <span className="tile__icone" aria-hidden>
                {spec.icon}
              </span>
            )}
            {spec.title}
          </h2>
          <button className="overlay__fechar" onClick={aoFechar}>
            fechar
          </button>
        </header>

        <div className="overlay__body">
          <Barreira titulo={spec.title}>
            <Suspense fallback={<TileSkeleton />}>
              <Conteudo data={envelope?.data ?? null} status={status} reason={reason} coberto={false} />
            </Suspense>
          </Barreira>
        </div>

        {reason && <p className="tile__reason">{reason}</p>}
      </section>
    </div>
  );
}

export function Dashboard() {
  const [expandido, setExpandido] = useState<string | null>(null);

  const aoExpandir = useCallback((id: string) => setExpandido(id), []);
  const aoFechar = useCallback(() => setExpandido(null), []);

  const aberto = TILES.find((spec) => spec.id === expandido);

  return (
    <>
      <main className="dashboard">
        {TILES.map((spec) => (
          <TileHost
            key={spec.id}
            spec={spec}
            coberto={expandido === spec.id}
            aoExpandir={aoExpandir}
          />
        ))}
      </main>

      {aberto && <Expandido spec={aberto} aoFechar={aoFechar} />}
    </>
  );
}
