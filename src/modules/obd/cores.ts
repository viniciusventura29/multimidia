/**
 * O ciano do RPM.
 *
 * Fica fora do `core/telemetria` porque não é limiar nem alerta — é só a cor do
 * mostrador de rotação, e nenhum outro canto da tela precisa concordar com ela.
 *
 * Aponta para um token de CSS pelo mesmo motivo que as cores de lá: o #4dd8e0
 * original foi afinado para brilhar sobre quase-preto e rende 1,4:1 sobre o card
 * claro — a barra de rotação sumia no trilho. O valor de cada tema mora no
 * `App.css` (`--tom-ciano`).
 */
export const CIANO = "var(--tom-ciano)";
