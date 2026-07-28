/**
 * Cores e limiares da telemetria do carro, num lugar só.
 *
 * Os mostradores (temperatura, velocidade) e o header (bateria, gasolina) leem
 * daqui, para "gasolina verde/laranja/vermelho" significar a mesma coisa em
 * qualquer canto da tela.
 */

export const AZUL = "#4da3ff";
export const VERDE = "#3ddc97";
export const AMARELO = "#f5c542";
export const LARANJA = "#f5a524";
export const VERMELHO = "#e5484d";

/** Faixa plausível de tensão de um carro: 11,8 V (fraca) → 14,4 V (carregando). */
export const V_VAZIO = 11.8;
export const V_CHEIO = 14.4;

/** Tensão não é carga de fato — não há PID de SoC, só voltage. É uma leitura. */
export const voltagemPct = (v: number | null) =>
  v === null ? null : ((v - V_VAZIO) / (V_CHEIO - V_VAZIO)) * 100;

export const corTemp = (v: number | null) =>
  v === null ? VERDE : v > 110 ? VERMELHO : v > 100 ? LARANJA : VERDE;

export const corFuel = (v: number | null) =>
  v === null ? VERDE : v < 15 ? VERMELHO : v < 40 ? LARANJA : VERDE;

export const corBateria = (v: number | null) =>
  v === null ? AMARELO : v < V_VAZIO ? VERMELHO : v < 12.4 ? LARANJA : AMARELO;
