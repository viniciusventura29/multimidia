import { Gauge } from "../shell/Gauge";
import { defineTile, type AnyTileSpec, type ObdReadings } from "../core/types";

const OBD = "obd";

/**
 * Os cinco mostradores.
 *
 * Todos leem do mesmo módulo, porque é uma conexão só com o carro: um adaptador
 * ELM327, um barramento. Isso é o que faz os cinco escurecerem juntos quando o
 * adaptador cai, sem contaminar música nem navegação.
 */
function mostrador(
  id: string,
  title: string,
  area: string,
  ler: (r: ObdReadings) => number | null,
  opcoes: { unit?: string; max?: number; decimals?: number } = {},
): AnyTileSpec {
  const valor = (data: ObdReadings | null) => (data ? ler(data) : null);

  return defineTile<ObdReadings>({
    id,
    module: OBD,
    title,
    area,
    Compact: ({ data }) => <Gauge value={valor(data)} {...opcoes} />,
    Expanded: ({ data }) => <Gauge value={valor(data)} {...opcoes} size="grande" />,
  });
}

export const obdTiles: AnyTileSpec[] = [
  mostrador("rpm", "RPM", "rpm", (r) => r.rpm, { max: 7000 }),
  mostrador("temp", "Temperatura", "temp", (r) => r.coolantC, {
    unit: "°C",
    max: 120,
  }),
  mostrador("combustivel", "Combustível", "combustivel", (r) => r.fuelPct, {
    unit: "%",
    max: 100,
  }),
  mostrador("voltagem", "Voltagem", "voltagem", (r) => r.voltage, {
    unit: "V",
    decimals: 1,
  }),
  mostrador("velocidade", "Velocidade", "velocidade", (r) => r.speedKmh, {
    unit: "km/h",
    max: 200,
  }),
];
