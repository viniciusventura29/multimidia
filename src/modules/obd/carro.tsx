import { Gauge as GaugeIcon, Thermometer, Zap } from "lucide-react";

import {
  AZUL,
  RPM_MAX,
  TEMP_MAX,
  corBateria,
  corFuel,
  corTemp,
  voltagemPct,
} from "../../core/telemetria";
import type { ObdReadings, TileView } from "../../core/types";
import { Dado } from "../../shell/Dado";
import { Gauge } from "../../shell/Gauge";
import { BateriaIcon } from "../../shell/indicadores";
import { Ajustes } from "./ajustes";
import { CIANO } from "./cores";
import { NivelTanque } from "./tanque";

/** Como cada método de vazão se explica em uma linha. */
const FONTE: Record<ObdReadings["consumo"]["metodo"], string> = {
  direto: "consumo informado pelo carro",
  maf: "consumo medido pelo sensor de massa de ar",
  coletor: "consumo estimado pela pressão do coletor",
  carga: "consumo estimado pela carga do motor",
  indisponivel: "este carro não informa o que preciso para calcular consumo",
};

const fmtDuracao = (s: number) => {
  const horas = Math.floor(s / 3600);
  const minutos = Math.floor((s % 3600) / 60);
  return horas > 0 ? `${horas}h${String(minutos).padStart(2, "0")}` : `${minutos}min`;
};

/**
 * Tudo sobre o carro, em duas faixas: o que se lê de relance em cima, o que se ajusta
 * parado embaixo.
 *
 * Os números derivados vêm com `~` (ver `Dado`). A legenda aparece uma vez, no bloco
 * de consumo, para a convenção ser aprendível sem manual.
 */
export function Carro({ data }: TileView<ObdReadings>) {
  const consumo = data?.consumo ?? null;
  const tanque = data?.tanque ?? null;
  const viagem = data?.viagem ?? null;
  const estimado = !(consumo?.medido ?? false);

  return (
    <div className="carro">
      <div className="carro__leitura">
        <section className="carro__heroi">
          <Gauge
            value={data?.speedKmh ?? null}
            unit="km/h"
            icon={<Zap size="1em" />}
            tone={AZUL}
            size="grande"
          />
          <Dado
            rotulo="Autonomia"
            valor={tanque?.autonomiaKm ?? null}
            unit="km"
            forte
            estimado
            tone={corFuel(data?.fuelPct ?? null)}
          />
        </section>

        <section className="carro__motor">
          <Gauge
            value={data?.rpm ?? null}
            unit="rpm"
            max={RPM_MAX}
            tone={CIANO}
            icon={<GaugeIcon size="1em" />}
          />
          <Gauge
            value={data?.coolantC ?? null}
            unit="°C"
            max={TEMP_MAX}
            tone={corTemp(data?.coolantC ?? null)}
            icon={<Thermometer size="1em" />}
          />
          {/* Sem `max`: a janela útil da tensão é 11,8–14,4 V, não 0–14,4, então a
              barra do Gauge diria "quase cheia" sempre. O nível mora no ícone. */}
          <Gauge
            value={data?.voltage ?? null}
            unit="V"
            decimals={1}
            tone={corBateria(data?.voltage ?? null)}
            icon={<BateriaIcon pct={voltagemPct(data?.voltage ?? null)} />}
          />
        </section>

        <section className="carro__tanque">
          <NivelTanque
            pct={data?.fuelPct ?? null}
            litros={tanque?.litros ?? null}
            pctEstimada={tanque?.pct ?? null}
            medido={tanque?.medido ?? false}
          />
        </section>

        <section className="carro__consumo">
          <div className="carro__grade">
            <Dado
              rotulo="Consumo"
              valor={consumo?.instantaneoKmL ?? null}
              unit="km/l"
              decimais={1}
              estimado={estimado}
            />
            <Dado
              rotulo="Gasto"
              valor={consumo?.litrosHora ?? null}
              unit="L/h"
              decimais={1}
              estimado={estimado}
            />
            <Dado
              rotulo="Média da viagem"
              valor={viagem?.mediaKmL ?? null}
              unit="km/l"
              decimais={1}
              estimado
            />
            <Dado
              rotulo="Média do tanque"
              valor={tanque?.mediaTanqueKmL ?? null}
              unit="km/l"
              decimais={1}
              estimado
            />
          </div>

          <p className="carro__viagem">
            viagem <strong>{(viagem?.distanciaKm ?? 0).toFixed(1)} km</strong> ·{" "}
            {fmtDuracao(viagem?.duracaoS ?? 0)} · {(viagem?.litros ?? 0).toFixed(1)} L
          </p>
          <p className="carro__legenda">
            <span aria-hidden>~</span> estimado ·{" "}
            {consumo ? FONTE[consumo.metodo] : "aguardando o carro"}
          </p>
        </section>
      </div>

      <Ajustes data={data} />
    </div>
  );
}
