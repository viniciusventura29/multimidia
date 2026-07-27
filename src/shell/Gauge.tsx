interface Props {
  value: number | null;
  unit?: string;
  /** Fundo de escala. Sem isto, o mostrador vira só número, sem barra. */
  max?: number;
  decimals?: number;
  size?: "normal" | "grande";
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
export function Gauge({ value, unit, max, decimals = 0, size = "normal" }: Props) {
  const vazio = value === null || Number.isNaN(value);
  const preenchimento =
    vazio || !max ? 0 : Math.max(0, Math.min(100, (value / max) * 100));

  return (
    <div className={`gauge gauge--${size}`}>
      <p className="gauge__value">
        {vazio ? "--" : value.toFixed(decimals)}
        {unit && <span className="gauge__unit">{unit}</span>}
      </p>

      {max !== undefined && (
        <div className="gauge__track">
          <div className="gauge__fill" style={{ width: `${preenchimento}%` }} />
        </div>
      )}
    </div>
  );
}
