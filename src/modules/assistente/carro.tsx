import { lazy, Suspense, useCallback, useState } from "react";

import { Carrinho } from "./carrinho";

/**
 * O carro do herói — em três dimensões, com o desenho como rede de segurança.
 *
 * O 3D chega por chunk separado porque ele carrega o `three` junto: pendurá-lo
 * no bundle inicial atrasaria o primeiro paint do painel inteiro por causa de um
 * enfeite. Enquanto o chunk não chega — e para sempre, se ele não puder rodar —,
 * quem está na tela é o desenho em SVG de sempre.
 *
 * **O SVG não é código morto, é o plano B declarado.** Ele volta quando o
 * aparelho não tem WebGL sobrando, quando o contexto cai (ver `carro3d/`), e
 * enquanto o chunk carrega. Apagar um dos dois deixaria o painel sem carro em
 * exatamente o aparelho em que ele mais precisa funcionar: o barato.
 */

const Carro3D = lazy(() =>
  import("./carro3d").then((m) => ({ default: m.Carro3D })),
);

export function CarroDoHeroi({ coberto = false }: { coberto?: boolean }) {
  const [semTresD, setSemTresD] = useState(false);
  // Estável: o `useEffect` que monta a cena depende dela, e uma função nova a
  // cada render remontaria a cena a cada leitura do OBD.
  const desistir = useCallback(() => setSemTresD(true), []);

  if (semTresD) return <Carrinho />;

  return (
    <div className="heroi-carro">
      <Suspense fallback={<Carrinho />}>
        <Carro3D coberto={coberto} aoFalhar={desistir} />
      </Suspense>
    </div>
  );
}
