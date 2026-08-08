import { useEffect, useRef, useState } from "react";

import { useMensagensNovas, type MensagemNova } from "../modules/messaging";

/** Quanto tempo cada notificação fica na tela antes de sumir sozinha. */
const DURACAO_MS = 5000;
/** Quantas cabem ao mesmo tempo na base — a faixa se divide em até três. */
const VISIVEIS_MAX = 3;

const chave = (m: MensagemNova) => `${m.conversation} ${m.at} ${m.body}`;

/**
 * A foto de quem mandou. Cai numa inicial quando não há foto ou ela falha ao
 * carregar — o que é comum, já que nem toda notificação do Android traz o ícone.
 */
function Avatar({ nome, src }: { nome: string; src: string | null }) {
  const [erro, setErro] = useState(false);
  const inicial = nome.trim().charAt(0).toUpperCase() || "?";

  if (src && !erro) {
    return (
      <img className="notif__avatar" src={src} alt="" onError={() => setErro(true)} />
    );
  }
  return (
    <span className="notif__avatar notif__avatar--inicial" aria-hidden>
      {inicial}
    </span>
  );
}

/**
 * As notificações de mensagem nova, subindo da base.
 *
 * O WhatsApp não é mais um quadro no painel: dirigindo, ninguém lê uma lista de
 * conversas. Cada mensagem que chega vira uma pílula que sobe, mostra a foto, o
 * nome e o texto, e some sozinha em alguns segundos. É só leitura — tocar para
 * responder fica para depois.
 *
 * Uma conversa que manda de novo **substitui** a própria pílula (não empilha) e
 * reinicia o relógio; conversas diferentes ao mesmo tempo dividem a faixa em até
 * três. O padrão visual e de acessibilidade segue o `BemVindo`.
 */
export function Notificacoes() {
  const mensagens = useMensagensNovas();
  const [ativos, setAtivos] = useState<MensagemNova[]>([]);

  // O que já processamos por conversa. A primeira leitura entra aqui SEM virar
  // notificação: mensagens que já existiam antes de montar (ou o snapshot do
  // boot) não devem estourar um monte de pílulas de uma vez.
  const vistos = useRef(new Map<string, string>());
  const primeira = useRef(true);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  useEffect(() => {
    for (const m of mensagens) {
      const k = chave(m);
      if (vistos.current.get(m.conversation) === k) continue;
      vistos.current.set(m.conversation, k);
      if (primeira.current) continue;

      setAtivos((prev) => {
        const sem = prev.filter((a) => a.conversation !== m.conversation);
        const lista = [...sem, m];
        return lista.length > VISIVEIS_MAX ? lista.slice(lista.length - VISIVEIS_MAX) : lista;
      });

      const anterior = timers.current.get(m.conversation);
      if (anterior) clearTimeout(anterior);
      const t = setTimeout(() => {
        setAtivos((prev) => prev.filter((a) => a.conversation !== m.conversation));
        timers.current.delete(m.conversation);
      }, DURACAO_MS);
      timers.current.set(m.conversation, t);
    }
    primeira.current = false;
  }, [mensagens]);

  // Ninguém fica com um timer pendurado quando o componente sai (troca de perfil).
  useEffect(() => {
    const mapa = timers.current;
    return () => mapa.forEach((t) => clearTimeout(t));
  }, []);

  if (ativos.length === 0) return null;

  return (
    <div className="notificacoes" role="status" aria-live="polite">
      {ativos.map((n) => {
        const grupo = n.sender !== n.conversation;
        return (
          <article key={n.conversation} className="notif">
            <Avatar nome={n.conversation} src={n.avatar} />
            <div className="notif__corpo">
              <span className="notif__titulo">{n.conversation}</span>
              <span className="notif__linha">
                {grupo ? `${n.sender}: ${n.body}` : n.body}
              </span>
            </div>
          </article>
        );
      })}
    </div>
  );
}
