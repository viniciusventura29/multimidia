/**
 * Cores e limiares da telemetria do carro, num lugar só.
 *
 * Os mostradores (temperatura, velocidade) e o header (bateria, gasolina) leem
 * daqui, para "gasolina verde/laranja/vermelho" significar a mesma coisa em
 * qualquer canto da tela.
 */

/*
 * O PIGMENTO MORA NO CSS, os limiares moram aqui.
 *
 * Estas eram cinco constantes de hex. Viraram ponteiros para tokens porque o
 * painel tem dois temas, e "verde = está bom" precisa de dois verdes diferentes
 * para dizer a mesma coisa: o #3ddc97 que rende 11:1 sobre o card escuro rende
 * 1,4:1 sobre o card claro, ou seja, desaparece. Ver `--tom-*` no `App.css`.
 *
 * Continuam sendo `string` e continuam servindo para comparar (`cor === VERDE`
 * em `modules/obd/index.tsx` compara duas vezes a MESMA constante, não dois
 * hex), e continuam viajando do mesmo jeito: como valor de `--tom` num `style`
 * inline, de onde os ícones herdam por `currentColor`.
 */
export const AZUL = "var(--tom-azul)";
export const VERDE = "var(--tom-verde)";
export const AMARELO = "var(--tom-amarelo)";
export const LARANJA = "var(--tom-laranja)";
export const VERMELHO = "var(--tom-vermelho)";

/** Fundo de escala dos mostradores do motor. É escala de display, não limiar. */
export const RPM_MAX = 6500;
export const TEMP_MAX = 120;

/**
 * Acima disto o carro está andando.
 *
 * Serve para duas coisas: km/l só existe em movimento (abaixo disso o Rust manda
 * `null` e a tela mostra L/h), e ajustar um stepper com o carro rodando merece mais
 * tempo de dedo.
 */
export const ANDANDO_KMH = 5;

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
