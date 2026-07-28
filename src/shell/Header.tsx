import { useEffect, useState, type CSSProperties } from "react";

import { corBateria, corFuel, voltagemPct } from "../core/telemetria";
import type { ModuleStates, ObdReadings, Profile } from "../core/types";
import { BateriaIcon, GasolinaIcon } from "./indicadores";

interface Props {
  states: ModuleStates;
  profile: Profile;
  /** Clicar no nome abre a troca de perfil — herda o papel do antigo chip. */
  aoTrocar: () => void;
}

/**
 * A barra de status do painel.
 *
 * Carrega o que se olha de relance e não merece um quadro inteiro — hora,
 * bateria, gasolina — e o nome de quem dirige, à direita. A bateria e a gasolina
 * vêm do mesmo módulo `obd` dos mostradores; quando o adaptador cai, elas somem
 * junto (viram `--`), sem alarde.
 */
export function Header({ states, profile, aoTrocar }: Props) {
  const obd = states["obd"]?.data as ObdReadings | null | undefined;
  const voltage = obd?.voltage ?? null;
  const fuel = obd?.fuelPct ?? null;

  const hora = useHora();

  return (
    <header className="topbar">
      <div className="topbar__infos">
        <span className="topbar__item topbar__hora">{hora}</span>

        <span className="topbar__item" style={tom(corBateria(voltage))}>
          <BateriaIcon pct={voltagemPct(voltage)} className="topbar__icone" />
          {voltage === null ? "--" : `${voltage.toFixed(1)}V`}
        </span>

        <span className="topbar__item" style={tom(corFuel(fuel))}>
          <GasolinaIcon pct={fuel} className="topbar__icone" />
          {fuel === null ? "--" : `${fuel.toFixed(0)}%`}
        </span>
      </div>

      <button className="topbar__nome" onClick={aoTrocar}>
        <span
          className="topbar__ponto"
          style={{ background: profile.color }}
          aria-hidden
        />
        {profile.name}
      </button>
    </header>
  );
}

const tom = (cor: string): CSSProperties => ({ "--tom": cor } as CSSProperties);

/** Hora local, atualizada a cada 10 s — o bastante para os minutos não atrasarem. */
function useHora() {
  const [agora, setAgora] = useState(() => new Date());

  useEffect(() => {
    const t = setInterval(() => setAgora(new Date()), 10_000);
    return () => clearInterval(t);
  }, []);

  return agora.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
}
