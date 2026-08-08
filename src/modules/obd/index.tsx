import { lazy } from "react";
import { Fuel, Gauge as GaugeIcon, Thermometer, Zap } from "lucide-react";

import { AZUL, RPM_MAX, VERDE, corFuel, corTemp } from "../../core/telemetria";
import { CIANO } from "./cores";
import { defineTile, type AnyTileSpec, type ObdReadings } from "../../core/types";
import { Dado } from "../../shell/Dado";
import { Gauge } from "../../shell/Gauge";
import { GasolinaIcon } from "../../shell/indicadores";

// A tela expandida (diagnóstico + ajustes de tanque) só existe quando alguém
// toca no quadro — não precisa estar no chunk do primeiro paint.
const Carro = lazy(() => import("./carro").then((m) => ({ default: m.Carro })));

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

      {/*
        O conta-giros, que morava só na tela cheia.

        O quadro tem espaço para três coisas e mostrava duas, com um vão morto no
        meio grande o bastante para parecer defeito. O RPM é o candidato óbvio a
        ocupá-lo: é a segunda coisa que se olha dirigindo, e é ele que diz se a
        marcha está certa — coisa que a velocidade sozinha não responde.

        Vem com barra (`max`) e a velocidade não: o giro tem fundo de escala de
        verdade, e a faixa é a informação (perto do corte importa mais que o
        número). Velocidade não tem teto que signifique algo num painel de rua.
      */}
      <div className="velo__giro">
        <Gauge
          value={data?.rpm ?? null}
          unit="rpm"
          max={RPM_MAX}
          tone={CIANO}
          icon={<GaugeIcon size="1em" />}
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
  area: "carro",
  Compact: ({ data }) => <Velocidade data={data} />,
  Expanded: Carro,
});

export const obdTiles: AnyTileSpec[] = [velocidadeTile];
