/** Um componente por tipo de cartão. */

import { useEffect, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";

import { Barras, Linha, MINIMO_PONTOS } from "./graficos";
import {
  corDoTom,
  ehArquivoLocal,
  PREFIXO_ARQUIVO,
  TOM_PADRAO,
  type Cartao,
} from "./tipos";

/**
 * O tipo do arquivo, deduzido da extensão.
 *
 * Um `Blob` sem tipo funciona para PNG e JPEG, porque o navegador fareja o
 * cabeçalho desses formatos — mas **não** para SVG, que sem `image/svg+xml`
 * declarado simplesmente não desenha. Como o nome do arquivo já traz a
 * extensão, não custa nada acertar.
 */
function tipoDaImagem(nome: string): string {
  const extensao = nome.slice(nome.lastIndexOf(".") + 1).toLowerCase();
  switch (extensao) {
    case "svg":
      return "image/svg+xml";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "webp":
      return "image/webp";
    default:
      return "image/png";
  }
}

/**
 * As imagens que o Rust baixou ou gerou vivem no diretório de dados do app, e
 * não numa URL que o WebView alcance. Um comando devolve os bytes e aqui eles
 * viram um object URL.
 *
 * Um comando, e não o protocolo de asset do Tauri, porque assim funciona igual
 * no Mac e no Android sem mexer em capability nenhuma.
 */
function useImagemLocal(url: string): string | null {
  const [pronta, setPronta] = useState<string | null>(null);

  useEffect(() => {
    if (!ehArquivoLocal(url)) {
      setPronta(url);
      return;
    }

    let vivo = true;
    let objeto: string | null = null;

    const nome = url.slice(PREFIXO_ARQUIVO.length);

    invoke<number[]>("imagem_ia", { nome })
      .then((bytes) => {
        // O tile monta duas vezes (grid + expandido) e o quadro pode trocar no
        // meio do carregamento: sem esta guarda, o `revoke` do cleanup correria
        // atrás de um URL que já foi substituído.
        if (!vivo) return;
        objeto = URL.createObjectURL(
          new Blob([new Uint8Array(bytes)], { type: tipoDaImagem(nome) }),
        );
        setPronta(objeto);
      })
      .catch(() => {
        // Arquivo podado pela limpeza, ou nome inválido. Some em silêncio: um
        // erro escrito no lugar da imagem é pior que nenhuma imagem.
        if (vivo) setPronta(null);
      });

    return () => {
      vivo = false;
      if (objeto) URL.revokeObjectURL(objeto);
    };
  }, [url]);

  return pronta;
}

function CartaoImagem({ url, legenda }: { url: string; legenda: string | null }) {
  const fonte = useImagemLocal(url);
  if (!fonte) return null;

  return (
    <figure className="ia-cartao ia-cartao--imagem">
      <img className="ia-cartao__img" src={fonte} alt={legenda ?? ""} />
      {legenda && <figcaption className="ia-cartao__legenda">{legenda}</figcaption>}
    </figure>
  );
}

export function CartaoView({ cartao }: { cartao: Cartao }) {
  switch (cartao.tipo) {
    case "texto":
      return (
        <article
          className="ia-cartao ia-cartao--texto"
          style={{ "--tom": corDoTom[cartao.tom] } as CSSProperties}
        >
          {cartao.titulo && <h3 className="ia-cartao__titulo">{cartao.titulo}</h3>}
          <p className="ia-cartao__corpo">{cartao.corpo}</p>
        </article>
      );

    case "metrica":
      return (
        <article
          className="ia-cartao ia-cartao--metrica"
          style={{ "--tom": corDoTom[cartao.tom] } as CSSProperties}
        >
          <p className="ia-cartao__numero">
            {cartao.valor}
            {cartao.unidade && (
              <span className="ia-cartao__unidade">{cartao.unidade}</span>
            )}
          </p>
          <p className="ia-cartao__rotulo">{cartao.rotulo}</p>
        </article>
      );

    case "grafico": {
      // Um ponto só não desenha série nenhuma. Em vez de mostrar um gráfico
      // vazio, o cartão some — o modelo tem outros cinco para dizer a mesma
      // coisa.
      if (cartao.pontos.length < MINIMO_PONTOS) return null;

      const Desenho = cartao.grafico === "barras" ? Barras : Linha;
      return (
        <article className="ia-cartao ia-cartao--grafico">
          <h3 className="ia-cartao__titulo">
            {cartao.titulo}
            {cartao.unidade && (
              <span className="ia-cartao__unidade"> {cartao.unidade}</span>
            )}
          </h3>
          <Desenho pontos={cartao.pontos} cor={TOM_PADRAO} />
        </article>
      );
    }

    case "imagem":
      return <CartaoImagem url={cartao.url} legenda={cartao.legenda} />;

    case "lista":
      return (
        <article className="ia-cartao ia-cartao--lista">
          {cartao.titulo && <h3 className="ia-cartao__titulo">{cartao.titulo}</h3>}
          <ul className="ia-cartao__itens">
            {cartao.itens.map((item, i) => (
              <li key={`${item}-${i}`}>{item}</li>
            ))}
          </ul>
        </article>
      );

    default:
      // Cartão de um tipo que esta versão da tela não conhece — pode acontecer
      // se o Rust for atualizado sozinho. Ignorar é melhor que quebrar a coluna
      // inteira.
      return null;
  }
}
