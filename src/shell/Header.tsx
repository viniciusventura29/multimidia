import { useEffect, useState, type CSSProperties } from "react";

import { shallowEqual, useModuleSelector } from "../core/moduleStore";
import { corBateria, corFuel, voltagemPct } from "../core/telemetria";
import type { ObdReadings, Profile } from "../core/types";
import { BateriaIcon, GasolinaIcon } from "./indicadores";

interface Props {
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
export function Header({ profile, aoTrocar }: Props) {
  // Fatia com igualdade: um tick de RPM não re-renderiza a barra — só quando
  // voltagem, nível ou autonomia mudam de verdade.
  const { voltage, fuel, autonomia } = useModuleSelector<
    ObdReadings,
    { voltage: number | null; fuel: number | null; autonomia: number | null }
  >(
    "obd",
    (obd) => ({
      voltage: obd?.voltage ?? null,
      fuel: obd?.fuelPct ?? null,
      autonomia: obd?.tanque?.autonomiaKm ?? null,
    }),
    shallowEqual,
  );

  const hora = useHora();

  return (
    <header className="topbar">
      <div className="topbar__infos">
        <span className="topbar__item topbar__hora">{hora}</span>

        <span className="topbar__item" style={tom(corBateria(voltage))}>
          <BateriaIcon pct={voltagemPct(voltage)} className="topbar__icone" />
          {voltage === null ? "--" : `${voltage.toFixed(1)}V`}
        </span>

        {/* O desenho diz "quanto tem" e os dígitos dizem "até onde dá": o ícone
            enche com a porcentagem medida, e o número é a autonomia — que é a
            pergunta de verdade ("chego em casa?"). A cor continua vindo da
            porcentagem, e não da estimativa: o vermelho de reserva não pode
            depender de um número que o próprio dono calibra. */}
        <span className="topbar__item" style={tom(corFuel(fuel))}>
          <GasolinaIcon pct={fuel} className="topbar__icone" />
          {autonomia !== null
            ? `~${autonomia.toFixed(0)}km`
            : fuel !== null
              ? `${fuel.toFixed(0)}%`
              : "--"}
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
