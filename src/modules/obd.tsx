import { Gauge as GaugeIcon, Thermometer, Zap } from "lucide-react";

import { Gauge } from "../shell/Gauge";
import { AZUL, corTemp } from "../core/telemetria";
import { defineTile, type AnyTileSpec, type ObdReadings } from "../core/types";

const OBD = "obd";

const CIANO = "#4dd8e0";

/** Um mini-mostrador que mora encaixado no quadro da velocidade. */
function Chip({
  icon,
  rotulo,
  valor,
  unit,
  tone,
}: {
  icon: React.ReactNode;
  rotulo: string;
  valor: number | null;
  unit?: string;
  tone: string;
}) {
  return (
    <div className="velo__chip">
      <span className="velo__chip-titulo">
        {icon} {rotulo}
      </span>
      <span className="velo__chip-valor" style={{ color: tone }}>
        {valor === null ? "--" : valor.toFixed(0)}
        {unit && <span className="velo__chip-unit">{unit}</span>}
      </span>
    </div>
  );
}

/**
 * A velocidade é o mostrador herói: número grande, e o RPM e a temperatura
 * encaixados em mini-mostradores no rodapé — os três vêm da mesma leitura do
 * carro, então moram juntos.
 */
function Velocidade({ data }: { data: ObdReadings | null }) {
  const speed = data?.speedKmh ?? null;
  const rpm = data?.rpm ?? null;
  const temp = data?.coolantC ?? null;

  return (
    <div className="velo">
      <div className="velo__medidor">
        <Gauge value={speed} unit="km/h" icon={<Zap size="1em" />} tone={AZUL} />
      </div>
      <div className="velo__chips">
        <Chip
          icon={<GaugeIcon size="1em" />}
          rotulo="RPM"
          valor={rpm}
          tone={CIANO}
        />
        <Chip
          icon={<Thermometer size="1em" />}
          rotulo="Temp"
          valor={temp}
          unit="°C"
          tone={corTemp(temp)}
        />
      </div>
    </div>
  );
}

const velocidadeTile: AnyTileSpec = defineTile<ObdReadings>({
  id: "velocidade",
  module: OBD,
  title: "Velocidade",
  area: "velocidade",
  Compact: ({ data }) => <Velocidade data={data} />,
  Expanded: ({ data }) => (
    <Gauge
      value={data?.speedKmh ?? null}
      unit="km/h"
      max={200}
      icon={<Zap size="1em" />}
      tone={AZUL}
      size="grande"
    />
  ),
});

export const obdTiles: AnyTileSpec[] = [velocidadeTile];
