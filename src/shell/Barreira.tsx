import { Component, type ErrorInfo, type ReactNode } from "react";

/**
 * O supervisor da tela.
 *
 * No Rust, um módulo que entra em pânico é reiniciado sozinho e os vizinhos nem
 * ficam sabendo (ver `supervisor.rs`). Do lado do React não havia nada
 * equivalente: um erro em qualquer tile derrubava a árvore inteira e o painel
 * virava uma tela branca — sem velocímetro, sem música, sem nada, dirigindo.
 *
 * Isto é a mesma ideia da moldura de tile: **degradado não esvazia**. O quadro
 * que quebrou some sozinho e diz por quê; o resto do painel continua de pé.
 */
interface Props {
  /** Aparece na mensagem, para o motorista saber qual quadro caiu. */
  titulo: string;
  children: ReactNode;
}

interface State {
  erro: Error | null;
}

export class Barreira extends Component<Props, State> {
  state: State = { erro: null };

  static getDerivedStateFromError(erro: Error): State {
    return { erro };
  }

  componentDidCatch(erro: Error, info: ErrorInfo) {
    // O console do WebView é o único lugar onde a pilha inteira sobrevive —
    // a tela mostra só a mensagem, que é o que cabe num painel de carro.
    console.error(`[eclipse] o quadro "${this.props.titulo}" caiu`, erro, info);
  }

  render() {
    if (!this.state.erro) return this.props.children;

    return (
      <div className="barreira">
        <span className="barreira__aviso">este quadro parou</span>
        <span className="barreira__motivo">{this.state.erro.message}</span>
      </div>
    );
  }
}
