import { corFuel } from "../../core/telemetria";
import { GasolinaIcon } from "../../shell/indicadores";
import type { CSSProperties } from "react";

interface Props {
  /** Porcentagem **medida** pelo carro — é ela que manda na cor. */
  pct: number | null;
  /** Litros estimados, que é o número que o motorista quer. */
  litros: number | null;
  /** Porcentagem derivada dos litros — é ela que manda no tamanho da barra. */
  pctEstimada: number | null;
  /** O nível vem do PID e o filtro convergiu: número sem ressalva. */
  medido: boolean;
}

/**
 * O nível do tanque: barra gorda, litros e porcentagem.
 *
 * O `Gauge` não serve aqui por duas razões. Precisa de barra larga, que é o que se lê
 * de relance com sol na tela, e precisa de dois números ao mesmo tempo (litros e %).
 * As marcas de E / ½ / F são o que torna uma barra de combustível legível sem ler
 * número nenhum — é assim em todo carro, e por bom motivo.
 *
 * A cor vem da porcentagem medida e o comprimento da estimada, de propósito: o
 * vermelho de reserva não pode depender de um número que o próprio usuário calibra.
 */
export function NivelTanque({ pct, litros, pctEstimada, medido }: Props) {
  const cheio = Math.max(0, Math.min(100, pctEstimada ?? 0));

  return (
    <div className="tanque" style={{ "--tom": corFuel(pct) } as CSSProperties}>
      <span className="tanque__rotulo">
        <GasolinaIcon pct={pct} className="tanque__icone" />
        Tanque
      </span>

      <span className="tanque__valor">
        {!medido && litros !== null && (
          <span className="dado__til" aria-hidden>
            ~
          </span>
        )}
        {litros === null ? "--" : litros.toFixed(1)}
        <span className="dado__unit">L</span>
        {/* A porcentagem do PID vai sem til: ela é medida, e os dois lado a lado
            ensinam a convenção melhor que qualquer legenda. */}
        <span className="tanque__pct">{pct === null ? "--" : `${pct.toFixed(0)}%`}</span>
      </span>

      <div className="tanque__trilho">
        <div className="tanque__nivel" style={{ width: `${cheio}%` }} />
        <span className="tanque__marca" style={{ left: "25%" }} />
        <span className="tanque__marca" style={{ left: "50%" }} />
        <span className="tanque__marca" style={{ left: "75%" }} />
      </div>

      <span className="tanque__escala">
        <span>E</span>
        <span>½</span>
        <span>F</span>
      </span>
    </div>
  );
}
