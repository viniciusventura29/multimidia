import { useEffect, useRef, useState } from "react";

interface Props {
  rotulo: string;
  /** Quanto tempo de dedo parado confirma. */
  ms?: number;
  aoConfirmar: () => void;
}

/**
 * Um botão que só dispara depois de o dedo ficar parado nele.
 *
 * Protege o que apaga número em que o motorista confia: "enchi o tanque" e "zerar
 * viagem". Escolhido em vez do duplo-toque do seletor de perfis porque o acidente
 * típico num carro sacudindo é exatamente *dois* contatos no mesmo lugar — o bounce
 * do solavanco. Contato sustentado é algo que buraco não produz.
 *
 * A barra de preenchimento não é enfeite: é ela que ensina que existe um tempo a
 * cumprir. Sem ela, segurar um botão que não reage parece um botão quebrado.
 */
export function Segurar({ rotulo, ms = 800, aoConfirmar }: Props) {
  const [armado, setArmado] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  const soltar = () => {
    setArmado(false);
    window.clearTimeout(timer.current);
  };

  const pegar = () => {
    setArmado(true);
    timer.current = window.setTimeout(() => {
      setArmado(false);
      aoConfirmar();
    }, ms);
  };

  // Um desmonte com o dedo ainda na tela deixaria o timer disparando no vazio.
  useEffect(() => () => window.clearTimeout(timer.current), []);

  return (
    <button
      type="button"
      className={`segurar${armado ? " segurar--armado" : ""}`}
      onPointerDown={pegar}
      onPointerUp={soltar}
      // Os dois abortam quando o dedo escorrega com o carro balançando — sem eles,
      // um toque que desliza para fora do botão ainda confirmaria.
      onPointerCancel={soltar}
      onPointerLeave={soltar}
    >
      <span
        className="segurar__progresso"
        style={{ transitionDuration: `${ms}ms` }}
        aria-hidden
      />
      <span className="segurar__rotulo">{rotulo}</span>
      <span className="segurar__dica">segure</span>
    </button>
  );
}
