import { useState } from "react";

import type { AnyTileSpec, ModuleStates, TileView } from "../core/types";
import { TILES } from "./registry";
import { Tile } from "./Tile";

/** O estado do módulo do qual um tile lê, ou `loading` se ele ainda não falou. */
function viewDe(states: ModuleStates, spec: AnyTileSpec): TileView<unknown> {
  const envelope = states[spec.module];
  return {
    data: envelope?.data ?? null,
    status: envelope?.status ?? (spec.estatico ? "ready" : "loading"),
    reason: envelope?.reason ?? null,
  };
}

function Expandido({
  spec,
  view,
  aoFechar,
}: {
  spec: AnyTileSpec;
  view: TileView<unknown>;
  aoFechar: () => void;
}) {
  const Conteudo = spec.Expanded!;

  return (
    <div className="overlay" onClick={aoFechar}>
      <section
        className={`overlay__painel overlay__painel--${view.status}`}
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
          <Conteudo {...view} />
        </div>

        {view.reason && <p className="tile__reason">{view.reason}</p>}
      </section>
    </div>
  );
}

export function Dashboard({ states }: { states: ModuleStates }) {
  const [expandido, setExpandido] = useState<string | null>(null);

  const aberto = TILES.find((spec) => spec.id === expandido);

  return (
    <>
      <main className="dashboard">
        {TILES.map((spec) => {
          const view = viewDe(states, spec);
          const Conteudo = spec.Compact;

          return (
            <Tile
              key={spec.id}
              title={spec.title}
              area={spec.area}
              icon={spec.icon}
              status={view.status}
              reason={view.reason}
              onExpand={spec.Expanded ? () => setExpandido(spec.id) : undefined}
            >
              <Conteudo {...view} />
            </Tile>
          );
        })}
      </main>

      {aberto && (
        <Expandido
          spec={aberto}
          view={viewDe(states, aberto)}
          aoFechar={() => setExpandido(null)}
        />
      )}
    </>
  );
}
