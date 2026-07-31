import { useState, type ReactNode } from "react";

interface Props {
  rotulo: string;
  /** O valor que vale hoje, vindo do Rust. */
  inicial: number;
  unit: string;
  decimais?: number;
  /** Dois tamanhos de passo por lado: fino e grosso. */
  passos: [number, number];
  min: number;
  max: number;
  /** Chamado quando o usuário confirma o rascunho. */
  aoConfirmar?: (valor: number) => void;
  /** Legenda ao lado do rótulo. Por padrão, o valor que vale hoje. */
  nota?: string;
  /** Substitui o botão de confirmar — usado pelo "enchi o tanque", que é um hold. */
  render?: (valor: number, intocado: boolean) => ReactNode;
}

/**
 * Um seletor de número por toque, sem teclado.
 *
 * Teclado do sistema está fora de questão numa head unit: o IME do Android sobe
 * *sobre* o app, redimensiona a WebView (brigando com o `100dvh` e com a escala em
 * `rem`) e em muitos aparelhos é um teclado ruim de fábrica. Este painel é quiosque,
 * não formulário.
 *
 * O número embaixo dos `±` é **rascunho de entrada**, não projeção do carro — é a
 * única exceção à regra de que a tela nunca pinta o efeito de um toque. Ele só vira
 * verdade quando o usuário confirma e o Rust ecoa. Quem re-semeia o rascunho com a
 * verdade é o `key` de quem monta este componente.
 */
export function Passo({
  rotulo,
  inicial,
  unit,
  decimais = 0,
  passos,
  min,
  max,
  aoConfirmar,
  nota,
  render,
}: Props) {
  const [valor, setValor] = useState(inicial);
  const intocado = valor === inicial;

  const mover = (delta: number) =>
    setValor((v) => {
      const novo = Math.min(max, Math.max(min, v + delta));
      // Somar 0,5 repetidamente acumula lixo de ponto flutuante, e o rascunho
      // apareceria como 30,499999 na tela.
      return Number(novo.toFixed(decimais + 1));
    });

  const [fino, grosso] = passos;

  return (
    <div className="passo">
      <span className="passo__rotulo">
        {rotulo}
        <span className="passo__hoje">
          {nota ?? `${inicial.toFixed(decimais)}${unit} hoje`}
        </span>
      </span>

      <div className="passo__linha">
        <button type="button" className="passo__botao" onClick={() => mover(-grosso)}>
          −{grosso}
        </button>
        <button type="button" className="passo__botao" onClick={() => mover(-fino)}>
          −{fino}
        </button>
        <span className="passo__valor">
          {valor.toFixed(decimais)}
          <span className="dado__unit">{unit}</span>
        </span>
        <button type="button" className="passo__botao" onClick={() => mover(fino)}>
          +{fino}
        </button>
        <button type="button" className="passo__botao" onClick={() => mover(grosso)}>
          +{grosso}
        </button>
      </div>

      {render
        ? render(valor, intocado)
        : !intocado && (
            <button
              type="button"
              className="passo__aplicar"
              onClick={() => aoConfirmar?.(valor)}
            >
              aplicar
            </button>
          )}
    </div>
  );
}
