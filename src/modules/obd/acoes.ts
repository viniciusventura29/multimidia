import { dispatchAction } from "../../core/actions";
import type { AcaoObd } from "../../core/types";

/**
 * Manda uma ação para o módulo `obd`, com o contrato tipado.
 *
 * Os outros módulos passam objeto solto para o `dispatchAction`. Aqui vale envelopar:
 * são ações que mudam litros e autonomia — número em que o motorista confia — e o Rust
 * descarta payload que ele não entende. Errar o nome de uma chave falharia calado.
 */
export const acaoObd = (acao: AcaoObd): void => dispatchAction("obd", acao);
