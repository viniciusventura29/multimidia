/**
 * Indicadores que "enchem".
 *
 * O lucide desenha ícones de contorno fixo — bateria cheia, bateria vazia — mas
 * não um preenchimento proporcional. Aqui bateria e gasolina são SVGs próprios
 * cujo miolo escala com o nível, para o motorista ver de relance "quanto tem"
 * sem ler o número. Ambos herdam a cor via `currentColor`, então o tom por
 * faixa definido no `Gauge` pinta o traço e o preenchimento juntos.
 */

interface Props {
  /** Nível de 0 a 100. `null` = ainda não li → casco vazio. */
  pct: number | null;
  className?: string;
}

const nivel = (pct: number | null) =>
  pct === null || Number.isNaN(pct) ? 0 : Math.max(0, Math.min(100, pct));

/** Bateria deitada; o miolo cresce da esquerda para a direita. */
export function BateriaIcon({ pct, className }: Props) {
  const cheio = nivel(pct);
  // Área interna útil: x de 4 a 16 (12 de largura).
  const largura = (12 * cheio) / 100;

  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="2" y="7" width="16" height="10" rx="2" />
      <path d="M20 10.5v3" />
      {cheio > 0 && (
        <rect
          x="4"
          y="9"
          width={largura}
          height="6"
          rx="1"
          fill="currentColor"
          stroke="none"
        />
      )}
    </svg>
  );
}

/** Tanque de combustível; o líquido sobe de baixo para cima. */
export function GasolinaIcon({ pct, className }: Props) {
  const cheio = nivel(pct);
  // Área interna útil: y de 6 a 18 (12 de altura).
  const altura = (12 * cheio) / 100;
  const topo = 18 - altura;

  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      {/* Tampa do tanque. */}
      <path d="M9 3h4" />
      <rect x="5" y="5" width="12" height="16" rx="2" />
      {/* Bico, para ler como "combustível" e não uma caixa qualquer. */}
      <path d="M17 9h2.5a1.5 1.5 0 0 1 1.5 1.5V15" />
      {cheio > 0 && (
        <rect
          x="7"
          y={topo}
          width="8"
          height={altura}
          rx="1"
          fill="currentColor"
          stroke="none"
        />
      )}
    </svg>
  );
}
