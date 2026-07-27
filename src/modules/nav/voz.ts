/**
 * Fala as instruções.
 *
 * Quem decide *o que* e *quando* falar é o Rust — ele tem a rota e a posição, e
 * guarda o que já disse. Aqui só se pronuncia.
 *
 * A checagem de repetição existe porque o mesmo envelope pode chegar mais de uma
 * vez: o `useModuleStates` repinta a partir do snapshot ao montar, e sem isso o
 * painel repetiria a última frase toda vez que a janela recarregasse.
 */
let ultimaFalada: string | null = null;

export function falar(frase: string | null): void {
  if (!frase || frase === ultimaFalada) return;
  ultimaFalada = frase;

  const sintese = window.speechSynthesis;
  if (!sintese) return;

  // Uma instrução nova cancela a anterior. Com o carro andando, ouvir a manobra
  // passada terminar antes da próxima é pior que perder o fim da frase.
  sintese.cancel();

  const voz = new SpeechSynthesisUtterance(frase);
  voz.lang = "pt-BR";
  voz.rate = 1.05;
  sintese.speak(voz);
}

/** Esquece o que foi dito, para o próximo trajeto começar limpo. */
export function calarSe(): void {
  ultimaFalada = null;
  window.speechSynthesis?.cancel();
}
