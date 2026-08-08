import type { KeyboardEvent, ReactNode } from "react";
import type { Status, TileSpec } from "../core/types";

interface Props {
  title: string;
  status: Status;
  reason: string | null;
  area?: string;
  icon?: ReactNode;
  chrome?: TileSpec<unknown>["chrome"];
  onExpand?: () => void;
  children: ReactNode;
}

/**
 * A moldura de todo quadro do painel.
 *
 * Ela é quem traduz `status` em aparência — e a regra é: degradado não esvazia.
 * O conteúdo continua na tela, esmaecido, com o motivo embaixo, para o motorista
 * saber que aquele número está velho em vez de achar que zerou.
 *
 * Também é quem decide entre card e célula nua (ver `chrome` no `TileSpec`).
 * Nua ela some inteira — sem fundo, sem padding, sem cabeçalho — mas as regras
 * de estado ficam: o aviso de "sem sinal" e o motivo continuam por cima do
 * conteúdo, porque um mapa velho precisa dizer que está velho tanto quanto um
 * número velho precisa.
 */
export function Tile({
  title,
  status,
  reason,
  area,
  icon,
  chrome = "card",
  onExpand,
  children,
}: Props) {
  const clicavel = Boolean(onExpand);
  const nu = chrome === "nu";

  const aoTeclar = (event: KeyboardEvent<HTMLElement>) => {
    if (!onExpand) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onExpand();
    }
  };

  return (
    <section
      className={`tile tile--${status}${nu ? " tile--nu" : ""}${
        clicavel ? " tile--clicavel" : ""
      }`}
      style={area ? { gridArea: area } : undefined}
      onClick={onExpand}
      onKeyDown={aoTeclar}
      role={clicavel ? "button" : undefined}
      tabIndex={clicavel ? 0 : undefined}
      aria-label={nu ? title : undefined}
    >
      {nu ? (
        // Sem cabeçalho, o aviso de degradado não tem onde morar — então ele
        // flutua por cima do conteúdo em vez de sumir. É a mesma informação, no
        // único lugar que sobrou.
        status === "degraded" && <span className="tile__badge tile__badge--solto">sem sinal</span>
      ) : (
        <header className="tile__head">
          <span className="tile__rotulo">
            {icon && (
              <span className="tile__icone" aria-hidden>
                {icon}
              </span>
            )}
            <span className="tile__title">{title}</span>
          </span>
          {status === "degraded" && <span className="tile__badge">sem sinal</span>}
        </header>
      )}

      <div className="tile__body">{children}</div>

      {reason && <p className="tile__reason">{reason}</p>}
    </section>
  );
}
