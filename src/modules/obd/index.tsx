import { Fuel, Thermometer, Zap } from "lucide-react";

import { AZUL, VERDE, corFuel, corTemp } from "../../core/telemetria";
import { defineTile, type AnyTileSpec, type ObdReadings } from "../../core/types";
import { Dado } from "../../shell/Dado";
import { Gauge } from "../../shell/Gauge";
import { GasolinaIcon } from "../../shell/indicadores";
import { Carro } from "./carro";

const OBD = "obd";

/**
 * O alerta que merece roubar um lugar no quadro compacto.
 *
 * Temperatura é o único hoje, e é o que justifica a regra: o carro tem 25 anos, e
 * superaquecer é a única falha que custa motor. Em vez de ocupar um mostrador
 * permanente — num quadro que só tem dois — ela aparece **quando está errada**, e aí
 * toma o lugar do consumo. Sacrifica-se o consumo e não a autonomia porque autonomia
 * responde "chego?", a pergunta que não pode ficar sem resposta.
 *
 * O limiar continua único em `core/telemetria`; aqui só mora a prioridade de
 * apresentação.
 */
function alertaDoCarro(data: ObdReadings | null) {
  const temp = data?.coolantC ?? null;
  const cor = corTemp(temp);
  if (temp === null || cor === VERDE) return null;

  return {
    rotulo: "Temp",
    valor: temp,
    unit: "°C",
    tone: cor,
    icon: <Thermometer size="1em" />,
  };
}

/**
 * A velocidade é o mostrador herói, e no rodapé mora o que se pergunta dirigindo:
 * quanto o carro está fazendo e quantos km ainda dá.
 *
 * Parado, km/l não existe (o Rust manda `null`) e o chip troca para L/h — a mesma
 * grandeza de outro jeito, em vez de um `--` que não informa nada.
 */
function Velocidade({ data }: { data: ObdReadings | null }) {
  const consumo = data?.consumo ?? null;
  const usaLitrosHora =
    consumo?.instantaneoKmL == null && consumo?.litrosHora != null;
  const alerta = alertaDoCarro(data);

  return (
    <div className="velo">
      <div className="velo__medidor">
        <Gauge
          value={data?.speedKmh ?? null}
          unit="km/h"
          icon={<Zap size="1em" />}
          tone={AZUL}
        />
      </div>
      <div className="velo__chips">
        {alerta ? (
          <Dado {...alerta} alerta />
        ) : (
          <Dado
            rotulo={usaLitrosHora ? "Gasto" : "Consumo"}
            valor={
              usaLitrosHora ? consumo.litrosHora : (consumo?.instantaneoKmL ?? null)
            }
            unit={usaLitrosHora ? "L/h" : "km/l"}
            decimais={1}
            estimado={!(consumo?.medido ?? false)}
            icon={<Fuel size="1em" />}
          />
        )}
        <Dado
          rotulo="Autonomia"
          valor={data?.tanque?.autonomiaKm ?? null}
          unit="km"
          tone={corFuel(data?.fuelPct ?? null)}
          estimado
          icon={<GasolinaIcon pct={data?.fuelPct ?? null} />}
        />
      </div>
    </div>
  );
}

const velocidadeTile: AnyTileSpec = defineTile<ObdReadings>({
  id: "velocidade",
  module: OBD,
  title: "Carro",
  area: "velocidade",
  Compact: ({ data }) => <Velocidade data={data} />,
  Expanded: Carro,
});

export const obdTiles: AnyTileSpec[] = [velocidadeTile];
