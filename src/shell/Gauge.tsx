import type { CSSProperties, ReactNode } from "react";

interface Props {
  value: number | null;
  unit?: string;
  /** Fundo de escala. Sem isto, o mostrador vira só número, sem barra. */
  max?: number;
  decimals?: number;
  size?: "normal" | "grande";
  /** Ícone à esquerda do número — lucide ou um indicador que enche. */
  icon?: ReactNode;
  /**
   * Cor da barra e do ícone. Entra como custom property (`--tom`), e não em
   * `background` direto, para não atropelar o override de degradado do CSS
   * (`.tile--degraded .gauge__fill`), que precisa vencer na cascata.
   */
  tone?: string;
}

/**
 * Um mostrador.
 *
 * `null` vira `--` em vez de `0`: no barramento lento do Eclipse, "ainda não li"
 * e "li e deu zero" são coisas diferentes, e confundir as duas mostraria o carro
 * parado quando ele está a 100.
 *
 * A barra tem transição longa de propósito. As leituras chegam a cada ~0,9 s;
 * sem isso o preenchimento andaria em degraus e o painel pareceria travado.
 */
export function Gauge({
  value,
  unit,
  max,
  decimals = 0,
  size = "normal",
  icon,
  tone,
}: Props) {
  const vazio = value === null || Number.isNaN(value);
  const preenchimento =
    vazio || !max ? 0 : Math.max(0, Math.min(100, (value / max) * 100));

  const estilo = tone ? ({ "--tom": tone } as CSSProperties) : undefined;

  return (
    <div className={`gauge gauge--${size}`} style={estilo}>
      <div className="gauge__linha">
        {icon && <span className="gauge__icone">{icon}</span>}
        <p className="gauge__value">
          {vazio ? "--" : value.toFixed(decimals)}
          {unit && <span className="gauge__unit">{unit}</span>}
        </p>
      </div>

      {max !== undefined && (
        <div className="gauge__track">
          <div className="gauge__fill" style={{ width: `${preenchimento}%` }} />
        </div>
      )}
    </div>
  );
}
