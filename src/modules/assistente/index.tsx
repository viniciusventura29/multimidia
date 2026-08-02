import { useEffect, useState } from "react";
import { Sparkles } from "lucide-react";

import { defineTile, type AnyTileSpec, type TileView } from "../../core/types";
import { Carrinho } from "./carrinho";
import { CartaoView } from "./cartoes";
import { envelheceu, VALIDADE_MS, type AssistenteState } from "./tipos";

/**
 * O painel do assistente.
 *
 * Uma IA proativa: ninguém digita nem fala com ela — não há entrada. Ela é
 * acionada por acontecimentos (o carro ligou, uma rota foi traçada, o motor
 * esquentou) e escreve nesta coluna sozinha, puxando dados do carro e da
 * internet por ferramentas em formato MCP. O laço fica no `eclipse-ia`; aqui só
 * se desenha o que ele pintou.
 *
 * Quando não há nada para dizer — que é a maior parte do tempo —, a coluna vira
 * o [`Carrinho`], que reage à telemetria de verdade.
 */

/**
 * Reavalia periodicamente se o quadro envelheceu.
 *
 * O estado do módulo não muda quando o tempo passa, então sem este empurrão um
 * quadro das oito da manhã continuaria na tela às seis da tarde.
 */
function useRelogio(intervaloMs: number): number {
  const [agora, setAgora] = useState(() => Date.now());

  useEffect(() => {
    const id = setInterval(() => setAgora(Date.now()), intervaloMs);
    return () => clearInterval(id);
  }, [intervaloMs]);

  return agora;
}

function Assistente({ data, status }: TileView<AssistenteState>) {
  // Um minuto: a validade é de vinte e cinco, então checar mais miúdo que isso
  // seria acordar o React à toa dentro de um carro.
  const agora = useRelogio(60_000);

  const cartoes = data?.cartoes ?? [];
  // Degradado mantém o último quadro na tela (é o que o `Bus` preserva), mas um
  // quadro velho não vale mais que a animação.
  const velho = envelheceu(data?.geradoEm ?? null, agora);
  const mostrarQuadro = cartoes.length > 0 && !velho;

  if (data?.pensando && !mostrarQuadro) {
    return (
      <div className="ia ia--pensando">
        <Sparkles className="ia__brilho" size="1em" />
        <p className="ia__texto">vendo o que tem de novo…</p>
      </div>
    );
  }

  if (!mostrarQuadro) {
    return <Carrinho />;
  }

  return (
    <div className="ia">
      {data?.pensando && (
        <span className="ia__pulso" title="procurando novidade" aria-hidden />
      )}
      <div className="ia__cartoes">
        {cartoes.map((cartao, i) => (
          <CartaoView key={`${cartao.tipo}-${i}`} cartao={cartao} />
        ))}
      </div>
      {status === "degraded" && cartoes.length > 0 && (
        <p className="ia__nota">quadro anterior</p>
      )}
    </div>
  );
}

/**
 * A versão de tela cheia: os mesmos cartões com folga, mais o carrinho embaixo.
 *
 * Continua sem entrada nenhuma — o assistente é proativo por desenho, porque o
 * motorista está dirigindo. Isto aqui é para ler com calma no semáforo.
 */
function AssistenteCompleto(view: TileView<AssistenteState>) {
  const agora = useRelogio(60_000);
  const cartoes = view.data?.cartoes ?? [];
  const velho = envelheceu(view.data?.geradoEm ?? null, agora);

  if (cartoes.length === 0) {
    return (
      <div className="ia-completa ia-completa--vazia">
        <Carrinho />
        <p className="ia__texto">
          Nada novo agora. Eu apareço sozinha quando houver — ao ligar o carro,
          ao traçar uma rota, ou se o carro pedir atenção.
        </p>
      </div>
    );
  }

  return (
    <div className="ia-completa">
      <div className="ia-completa__cartoes">
        {cartoes.map((cartao, i) => (
          <CartaoView key={`${cartao.tipo}-${i}`} cartao={cartao} />
        ))}
      </div>
      {velho && (
        <p className="ia__nota">
          escrito há mais de {Math.round(VALIDADE_MS / 60_000)} minutos
        </p>
      )}
    </div>
  );
}

export const assistenteTile: AnyTileSpec = defineTile<AssistenteState>({
  id: "assistente",
  module: "assistente",
  title: "Assistente",
  area: "ia",
  icon: <Sparkles size="1em" />,
  Compact: Assistente,
  Expanded: AssistenteCompleto,
});
