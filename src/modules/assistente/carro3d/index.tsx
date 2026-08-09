import { useEffect, useRef, useState } from "react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";

import { useTelemetriaDoCarro } from "../telemetria";
import { montarCena, type Cena, type EstadoDaCena } from "./cena";

/**
 * O quadro onde o carro 3D vive.
 *
 * ## O risco que este arquivo administra
 *
 * O painel **já tem** um contexto WebGL: o mapa. Abrir um segundo numa
 * Mali-G52 não é proibido, mas é o tipo de coisa que, quando dá errado, dá
 * errado feio — o navegador derruba o contexto mais antigo para atender o novo,
 * e quem some é o mapa, que é o que ninguém pode perder. Daí as três defesas
 * daqui:
 *
 * 1. **Área pequena.** O herói tem ~500x280; com o teto de pixel ratio, a cena
 *    inteira cabe em menos pixels do que um sexto do mapa.
 * 2. **Nada de desenhar escondido.** Coberto pela tela cheia de outro quadro,
 *    ou com a aba em segundo plano, o laço para. É justamente quando o mapa
 *    está grande que o carro deixa de gastar.
 * 3. **Queda com rede.** Se o contexto cair — ou se nem der para criar —, o
 *    componente devolve `null` e quem chama volta para o desenho em SVG, que
 *    não usa GPU nenhuma. O painel perde volume, não perde o carro.
 *
 * ## O laço
 *
 * `requestAnimationFrame` com trava de 30 fps. A alternativa — `@keyframes`,
 * como no SVG — não existe aqui: cena 3D precisa de um quadro desenhado por
 * alguém. 30 fps é metade do custo de 60 e, num carro girando devagar e numa
 * roda que já passa do limite de amostragem, é indistinguível.
 */

/** Acima disto o dedo estava girando o carro, não tocando no quadro. */
const LIMIAR_DE_ARRASTO = 6;

/** ~30 fps. Em milissegundos, que é a moeda do `requestAnimationFrame`. */
const PERIODO = 1000 / 30;

/** De quanto em quanto tempo reler a cor do perfil. Ver `lerAcento`. */
const RELOGIO_DO_ACENTO = 1000;

/**
 * A cor do perfil ativo, lida do CSS.
 *
 * Vem de `getComputedStyle` e não de props porque quem manda nela é o `useTema`,
 * que a escreve como custom property no elemento raiz — perguntar ao CSS é
 * perguntar à fonte. Relida de segundo em segundo em vez de observada: trocar de
 * perfil é raro, um `getComputedStyle` por segundo não custa nada, e um
 * `MutationObserver` no `<html>` custaria mais em código do que em CPU.
 */
function lerAcento(): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue("--accent")
    .trim();
  return v || "#3ddc97";
}

interface Props {
  /** A tela cheia de algum quadro está por cima. Ver defesa 2 acima. */
  coberto?: boolean;
  /** Chamado quando o 3D não vai dar — quem chama volta para o SVG. */
  aoFalhar: () => void;
}

export function Carro3D({ coberto = false, aoFalhar }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const cenaRef = useRef<Cena | null>(null);
  const [vivo, setVivo] = useState(true);

  const telemetria = useTelemetriaDoCarro();
  // O laço lê a telemetria por referência: recriá-lo a cada leitura do OBD
  // (uma por segundo) descartaria e remontaria a cena inteira.
  const telemetriaRef = useRef(telemetria);
  telemetriaRef.current = telemetria;

  const cobertoRef = useRef(coberto);
  cobertoRef.current = coberto;

  /*
   * Quanto o dedo já girou o carro. Em `ref` e não em estado porque quem lê
   * isto é o laço de desenho, trinta vezes por segundo: virar estado do React
   * seria trinta renders do painel por segundo para mover um número.
   */
  const arrastoRef = useRef(0);
  const arrastando = useRef<{ id: number; x: number; andou: number } | null>(null);
  const naoAbrir = useRef(false);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const menosMovimento = window.matchMedia(
      "(prefers-reduced-motion: reduce)",
    ).matches;

    let cena: Cena;
    try {
      cena = montarCena(canvas, lerAcento(), undefined, aoFalhar);
    } catch (err) {
      // Sem WebGL, ou sem contexto sobrando. Não é erro de programa — é um
      // aparelho dizendo que não dá, e existe um plano B.
      console.warn("[eclipse] o carro 3D não subiu; fica o desenho", err);
      setVivo(false);
      aoFalhar();
      return;
    }
    cenaRef.current = cena;

    const medir = () => {
      const { clientWidth, clientHeight } = canvas;
      if (clientWidth > 0 && clientHeight > 0) {
        cena.redimensionar(clientWidth, clientHeight);
      }
    };
    medir();

    const observador = new ResizeObserver(medir);
    observador.observe(canvas);

    const perdeuContexto = (e: Event) => {
      // `preventDefault` pediria restauração; aqui não vale a pena tentar
      // reerguer uma cena 3D num aparelho que acabou de ficar sem contexto —
      // o SVG é mais barato e resolve.
      e.preventDefault();
      console.warn("[eclipse] contexto WebGL do carro caiu; fica o desenho");
      setVivo(false);
      aoFalhar();
    };
    canvas.addEventListener("webglcontextlost", perdeuContexto);

    let quadro = 0;
    let anterior = performance.now();
    let ultimoDesenho = 0;
    let acento = lerAcento();
    let ultimaLeituraDoAcento = anterior;

    const laco = (agora: number) => {
      quadro = requestAnimationFrame(laco);

      if (agora - ultimoDesenho < PERIODO) return;
      const dt = Math.min(0.25, (agora - anterior) / 1000);
      anterior = agora;
      ultimoDesenho = agora;

      // Escondido não desenha. É a defesa que mais importa: com a tela cheia do
      // mapa aberta, o carro sai inteiro da conta da GPU.
      if (cobertoRef.current || document.hidden) return;

      if (agora - ultimaLeituraDoAcento > RELOGIO_DO_ACENTO) {
        acento = lerAcento();
        ultimaLeituraDoAcento = agora;
      }

      const t = telemetriaRef.current;
      const estado: EstadoDaCena = {
        velocidade: t.velocidade,
        rpm: t.rpm,
        aceleracao: t.aceleracao,
        curva: t.curva,
        parado: t.parado,
        acento,
        menosMovimento,
        arrasto: arrastoRef.current,
      };

      cena.atualizar(estado, dt);
      cena.desenhar();
    };
    quadro = requestAnimationFrame(laco);

    return () => {
      cancelAnimationFrame(quadro);
      observador.disconnect();
      canvas.removeEventListener("webglcontextlost", perdeuContexto);
      cena.destruir();
      cenaRef.current = null;
    };
  }, [aoFalhar]);

  /*
   * Girar o carro com o dedo.
   *
   * Os `pointer*` cobrem mouse e toque de uma vez, que é o que importa num
   * painel que roda no Mac enquanto se desenha e num vidro de head unit depois.
   * O `setPointerCapture` mantém o arrasto vivo mesmo quando o dedo sai do
   * canvas — sem ele, girar rápido solta o carro no meio do caminho.
   */
  const aoPegar = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    arrastando.current = { id: e.pointerId, x: e.clientX, andou: 0 };
  };

  const aoMover = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const a = arrastando.current;
    if (!a || a.id !== e.pointerId) return;
    const dx = e.clientX - a.x;
    a.x = e.clientX;
    a.andou += Math.abs(dx);
    // 260 px de arrasto dão meia volta: rápido o bastante para dar a volta sem
    // repicar o dedo, devagar o bastante para parar num ângulo escolhido.
    arrastoRef.current += (dx / 260) * Math.PI;
  };

  const aoSoltar = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const a = arrastando.current;
    arrastando.current = null;
    if (a && a.andou > LIMIAR_DE_ARRASTO) {
      // Foi arrasto, não toque: o quadro não deve abrir em tela cheia porque
      // alguém girou o carro.
      e.stopPropagation();
      naoAbrir.current = true;
    }
  };

  const aoClicar = (e: ReactMouseEvent<HTMLCanvasElement>) => {
    if (naoAbrir.current) {
      naoAbrir.current = false;
      e.stopPropagation();
    }
  };

  if (!vivo) return null;

  return (
    <canvas
      className="carro3d"
      ref={canvasRef}
      onPointerDown={aoPegar}
      onPointerMove={aoMover}
      onPointerUp={aoSoltar}
      onPointerCancel={aoSoltar}
      onClickCapture={aoClicar}
      aria-hidden
    />
  );
}
