import type { KeyboardEvent, ReactNode } from "react";
import type { Status } from "../core/types";

interface Props {
  title: string;
  status: Status;
  reason: string | null;
  area?: string;
  icon?: ReactNode;
  onExpand?: () => void;
  children: ReactNode;
}

/**
 * A moldura comum de todo quadro do painel.
 *
 * Ela é quem traduz `status` em aparência — e a regra é: degradado não esvazia.
 * O conteúdo continua na tela, esmaecido, com o motivo embaixo, para o motorista
 * saber que aquele número está velho em vez de achar que zerou.
 */
export function Tile({ title, status, reason, area, icon, onExpand, children }: Props) {
  const clicavel = Boolean(onExpand);

  const aoTeclar = (event: KeyboardEvent<HTMLElement>) => {
    if (!onExpand) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onExpand();
    }
  };

  return (
    <section
      className={`tile tile--${status}${clicavel ? " tile--clicavel" : ""}`}
      style={area ? { gridArea: area } : undefined}
      onClick={onExpand}
      onKeyDown={aoTeclar}
      role={clicavel ? "button" : undefined}
      tabIndex={clicavel ? 0 : undefined}
    >
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

      <div className="tile__body">{children}</div>

      {reason && <p className="tile__reason">{reason}</p>}
    </section>
  );
}
