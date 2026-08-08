import { useEffect, useState, type CSSProperties } from "react";

import { shallowEqual, useModuleSelector } from "../core/moduleStore";
import { corBateria, corFuel, voltagemPct } from "../core/telemetria";
import type { ObdReadings } from "../core/types";
import type { Clima, MapaState } from "../modules/nav/tipos";
import { ClimaIcon } from "./clima";
import { BateriaIcon, GasolinaIcon } from "./indicadores";

/**
 * A faixa de status, por cima do mapa.
 *
 * Carrega o que se olha de relance e não merece um quadro inteiro: hora,
 * bateria, gasolina e o tempo lá fora.
 *
 * Antes era uma barra de sistema atravessando a tela inteira, e isso custava
 * caro duas vezes — comia uma faixa de altura do painel de ponta a ponta, e
 * ainda separava o painel em "sistema" e "conteúdo", como um computador.
 * Encolhida para cima do mapa, ela devolve a altura ao grid e passa a ler como
 * o que é: o cabeçalho da viagem, ao lado do cabeçalho do motorista.
 *
 * A bateria e a gasolina vêm do mesmo módulo `obd` dos mostradores; quando o
 * adaptador cai, elas viram `--` junto, sem alarde.
 */
export function BarraStatus() {
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

  // Do `nav`, e não de um módulo próprio: quem sabe "que tempo faz aqui" é
  // quem sabe onde é o "aqui". O `noite` vem de carona porque é ele que decide
  // entre sol e lua — e já estava sendo calculado ali para o tema do mapa.
  const { clima, noite } = useModuleSelector<
    MapaState,
    { clima: Clima | null; noite: boolean }
  >(
    "nav",
    (nav) => ({ clima: nav?.clima ?? null, noite: nav?.noite ?? false }),
    shallowEqual,
  );

  const hora = useHora();

  return (
    <div className="barra">
      <span className="barra__item barra__hora">{hora}</span>

      <span className="barra__item" style={tom(corBateria(voltage))}>
        <BateriaIcon pct={voltagemPct(voltage)} className="barra__icone" />
        {voltage === null ? "--" : `${voltage.toFixed(1)}V`}
      </span>

      {/* O desenho diz "quanto tem" e os dígitos dizem "até onde dá": o ícone
          enche com a porcentagem medida, e o número é a autonomia — que é a
          pergunta de verdade ("chego em casa?"). A cor continua vindo da
          porcentagem, e não da estimativa: o vermelho de reserva não pode
          depender de um número que o próprio dono calibra. */}
      <span className="barra__item" style={tom(corFuel(fuel))}>
        <GasolinaIcon pct={fuel} className="barra__icone" />
        {autonomia !== null
          ? `~${autonomia.toFixed(0)}km`
          : fuel !== null
            ? `${fuel.toFixed(0)}%`
            : "--"}
      </span>

      {/* Sem clima o chip não aparece: um `--°` ao lado do ícone de sol não
          informa nada e ainda ocupa a barra. Bateria e gasolina merecem o
          `--` porque a ausência delas é notícia — a do clima não é. */}
      {clima && (
        <span className="barra__item" title={clima.rotulo}>
          <ClimaIcon
            familia={clima.familia}
            noite={noite}
            className="barra__icone"
          />
          {`${clima.tempC.toFixed(0)}°`}
        </span>
      )}
    </div>
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
