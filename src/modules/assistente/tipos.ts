/**
 * Espelho de `crates/eclipse-ia/src/cartao.rs`.
 *
 * Mexer num lado sem mexer no outro quebra a tela em silêncio: o Rust manda um
 * cartão que o `switch` daqui não conhece e ele simplesmente não desenha.
 */

import { AMARELO, LARANJA, VERDE, VERMELHO } from "../../core/telemetria";

export type Tom = "neutro" | "bom" | "atencao" | "alerta";
export type TipoGrafico = "barras" | "linha";

export interface Ponto {
  rotulo: string;
  valor: number;
}

export type Cartao =
  | { tipo: "texto"; titulo: string | null; corpo: string; tom: Tom }
  | {
      tipo: "metrica";
      rotulo: string;
      valor: string;
      unidade: string | null;
      tom: Tom;
    }
  | {
      tipo: "grafico";
      titulo: string;
      grafico: TipoGrafico;
      unidade: string | null;
      pontos: Ponto[];
    }
  | { tipo: "imagem"; url: string; legenda: string | null }
  | { tipo: "lista"; titulo: string | null; itens: string[] };

/** O que o módulo `assistente` publica. */
export interface AssistenteState {
  cartoes: Cartao[];
  /** ISO 8601. `null` enquanto nada foi pintado. */
  geradoEm: string | null;
  pensando: boolean;
  gatilho: string | null;
}

/**
 * As cores vêm de `core/telemetria.ts`, as mesmas dos mostradores do OBD: um
 * cartão em laranja e a barra de gasolina em laranja querem dizer a mesma coisa.
 *
 * `neutro` foge da tabela e usa a cor do perfil ativo — é o tom da maioria dos
 * cartões, e assim o assistente fica na cor de quem está dirigindo em vez de
 * pintar de "estado da telemetria" o que não é telemetria.
 */
export const corDoTom: Record<Tom, string> = {
  neutro: "var(--accent)",
  bom: VERDE,
  atencao: LARANJA,
  alerta: VERMELHO,
};

/** Cartão sem tom (imagem, lista) usa este. */
export const TOM_PADRAO = AMARELO;

/**
 * Depois de quanto tempo um quadro deixa de valer.
 *
 * Não é só estética: um comentário sobre o trânsito de meia hora atrás é pior
 * que nenhum. Passado o prazo, a coluna troca o quadro pela animação.
 */
export const VALIDADE_MS = 25 * 60 * 1000;

export function envelheceu(geradoEm: string | null, agora: number): boolean {
  if (!geradoEm) return true;
  const quando = Date.parse(geradoEm);
  return Number.isNaN(quando) || agora - quando > VALIDADE_MS;
}

/**
 * Imagem que o Rust baixou ou gerou vem como `arquivo:<nome>`; o resto é URL
 * pública (capa de álbum, foto na web) que o `<img>` busca direto.
 *
 * A distinção existe porque a URL da foto do Places carrega a chave do Maps na
 * query — ela nunca sai do Rust.
 */
export const PREFIXO_ARQUIVO = "arquivo:";

export function ehArquivoLocal(url: string): boolean {
  return url.startsWith(PREFIXO_ARQUIVO);
}
