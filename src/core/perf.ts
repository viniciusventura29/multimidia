/**
 * Medição de performance no lado da tela — **TEMPORÁRIA**.
 *
 * # Como arrancar isto depois
 *
 * 1. apague este arquivo;
 * 2. troque `from "./perf"` / `from "../core/perf"` de volta por
 *    `from "@tauri-apps/api/core"` nos arquivos que importam `invoke`;
 * 3. apague as chamadas de `contarQuadro` e `medirMapa` em `nav/mapa.tsx`.
 *
 * # Por que existe
 *
 * O dono diz que o mapa "está travado e demora para renderizar", e hoje não há
 * **um único número medido** no front: nem quadro, nem tempo de tile, nem
 * latência de chamada ao Rust. Sem linha de base, qualquer conserto vira
 * opinião — "melhorou" e "não melhorou" ficam indistinguíveis.
 *
 * Nada aqui aparece na tela. Tudo vai para o diário de bordo pelo caminho que
 * já existe, e sobe no lote dos 30 s.
 */

import { invoke as invokeCru } from "@tauri-apps/api/core";

import { anotar } from "./diario";

/** Acumula e manda o resumo de tempos de uma grandeza para o Rust. */
function registrar(nome: string, ms: number) {
  void invokeCru("medir_do_painel", { nome, ms }).catch(() => {});
}

/**
 * `invoke` cronometrado. Substitui o do Tauri onde importa.
 *
 * Existem oito lugares no front chamando `invoke` direto, sem nada em comum
 * entre eles — então nenhum deles era mensurável. Um embrulho só cobre todos,
 * e some junto com este arquivo.
 *
 * **Não** mede `medir_do_painel` nem `anotar_do_painel`: medir o medidor daria
 * recursão infinita, cada medição gerando outra.
 */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (cmd === "medir_do_painel" || cmd === "anotar_do_painel") {
    return invokeCru<T>(cmd, args);
  }
  const comecou = performance.now();
  try {
    return await invokeCru<T>(cmd, args);
  } finally {
    registrar(`ipc.${cmd}`, performance.now() - comecou);
  }
}

/**
 * Conta quadros e reporta o INTERVALO entre eles.
 *
 * Intervalo entre quadros, e não "FPS médio": a média esconde exatamente o que
 * o dono está sentindo. Trinta quadros de 10 ms e um de 400 ms dão uma média
 * boa e um engasgo bem visível — e é o `pior` do resumo que vai denunciar
 * isso.
 */
let ultimoQuadro: number | null = null;

export function contarQuadro(agora: number) {
  if (ultimoQuadro !== null) {
    const dt = agora - ultimoQuadro;
    // Acima de 2 s é o laço tendo sido parado de propósito (tela coberta, aba
    // escondida) e não um engasgo — contar isso poluiria o pior caso com uma
    // pausa que ninguém viu.
    if (dt < 2000) registrar("tela.quadro", dt);
  }
  ultimoQuadro = agora;
}

/** O laço parou; o próximo quadro não deve ser comparado com este. */
export function quadroPausado() {
  ultimoQuadro = null;
}

/**
 * Mede um trecho do ciclo de vida do mapa (carregar estilo, chegar ao `idle`).
 *
 * Vai como linha direta e não como amostra: acontece uma vez ou outra, e o
 * valor individual é a informação — "o mapa levou 4,2 s para acabar de
 * desenhar" é a frase que responde a queixa.
 */
export function medirMapa(oque: string, ms: number) {
  anotar("info", "perf", `mapa: ${oque}`, { ms: Math.round(ms) });
}
