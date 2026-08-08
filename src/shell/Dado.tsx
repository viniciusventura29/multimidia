import type { CSSProperties, ReactNode } from "react";

interface Props {
  rotulo: string;
  valor: number | null;
  unit?: string;
  decimais?: number;
  /** Cor do número. Entra como `--tom`, nunca em `color` direto — ver `Gauge`. */
  tone?: string;
  /**
   * O número foi derivado, não medido: ganha um `~` antes e um véu leve.
   *
   * É a convenção de honestidade do painel. Til e não cor porque cor já está
   * reservada para limiar — pintar estimativa de amarelo mataria o "amarelo =
   * atenção" — e porque o til compõe com o esmaecido de degradado em vez de brigar
   * com ele.
   */
  estimado?: boolean;
  /** Célula grande: a autonomia na tela cheia. */
  forte?: boolean;
  /** Este dado assumiu o lugar de outro porque algo está errado. */
  alerta?: boolean;
  icon?: ReactNode;
}

/**
 * Rótulo, número e unidade.
 *
 * Nasceu como o chip do rodapé da velocidade e virou primitivo quando a tela do carro
 * precisou da mesma anatomia doze vezes. Segue as regras da casa: `null` vira `--` e
 * nunca `0`, e a unidade é um `<span>` separado em vez de texto concatenado.
 */
export function Dado({
  rotulo,
  valor,
  unit,
  decimais = 0,
  tone,
  estimado,
  forte,
  alerta,
  icon,
}: Props) {
  const vazio = valor === null || Number.isNaN(valor);
  const classes = [
    "dado",
    forte && "dado--forte",
    estimado && "dado--estimado",
    alerta && "dado--alerta",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={classes} style={tone ? ({ "--tom": tone } as CSSProperties) : undefined}>
      {/* O ícone saiu de dentro do rótulo e virou um selo redondo à parte.
          Colado no texto minúsculo do rótulo ele era só mais um caractere; num
          disco próprio, ele vira a âncora que o olho encontra antes de ler —
          que é o serviço que um ícone tem para prestar num painel de carro. */}
      {icon && (
        <span className="dado__selo" aria-hidden>
          {icon}
        </span>
      )}
      <span className="dado__rotulo">{rotulo}</span>
      <span className="dado__valor">
        {/* Sem valor não há o que ressalvar: `~ --` só faria barulho. */}
        {estimado && !vazio && (
          <span className="dado__til" aria-hidden>
            ~
          </span>
        )}
        {vazio ? "--" : valor.toFixed(decimais)}
        {unit && <span className="dado__unit">{unit}</span>}
      </span>
    </div>
  );
}
