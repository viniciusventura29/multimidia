import { Sparkles } from "lucide-react";

import { defineTile, type AnyTileSpec } from "../core/types";

/**
 * O painel do assistente.
 *
 * Por enquanto é um espaço reservado. A ideia é ligar aqui uma IA que traga, ao
 * vivo, o que for relevante do trajeto, do carro e do dia — com liberdade para
 * escrever o que quiser neste quadro, puxando dados via MCPs. Como não lê de
 * módulo nenhum ainda, é `estatico`: nasce pronto, não fica "carregando".
 */
function Assistente() {
  return (
    <div className="ia">
      <Sparkles className="ia__brilho" size="1em" />
      <p className="ia__titulo">Assistente</p>
      <p className="ia__texto">
        Em breve: dicas do trajeto, do carro e do seu dia — aqui, ao vivo.
      </p>
    </div>
  );
}

export const assistenteTile: AnyTileSpec = defineTile<unknown>({
  id: "assistente",
  module: "assistente",
  title: "Assistente",
  area: "ia",
  estatico: true,
  icon: <Sparkles size="1em" />,
  Compact: () => <Assistente />,
});
