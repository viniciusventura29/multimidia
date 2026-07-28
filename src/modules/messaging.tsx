import { useState, type FormEvent, type MouseEvent } from "react";
import { MessageSquare } from "lucide-react";

import { dispatchAction } from "../core/actions";
import { defineTile, type AnyTileSpec, type TileView } from "../core/types";

const MESSAGING = "messaging";

interface Message {
  autor: "eles" | "eu";
  sender: string;
  body: string;
  at: string;
}

interface Conversation {
  name: string;
  messages: Message[];
  unread: number;
  canReply: boolean;
}

interface InboxState {
  conversations: Conversation[];
}

function Resposta({ conversa }: { conversa: Conversation }) {
  const [texto, setTexto] = useState("");

  const enviar = (event: FormEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const limpo = texto.trim();
    if (!limpo) return;

    dispatchAction(MESSAGING, {
      acao: "responder",
      conversa: conversa.name,
      texto: limpo,
    });
    setTexto("");
  };

  if (!conversa.canReply) {
    // A notificação foi dispensada e o RemoteInput foi junto. Dizer isso é
    // melhor que oferecer um campo que vai falhar.
    return <p className="msg__sem-resposta">essa conversa não aceita mais resposta</p>;
  }

  return (
    <form className="msg__responder" onSubmit={enviar} onClick={(e) => e.stopPropagation()}>
      <input
        className="msg__campo"
        value={texto}
        onChange={(e) => setTexto(e.target.value)}
        placeholder="responder…"
        maxLength={200}
      />
      <button className="msg__enviar" type="submit" disabled={!texto.trim()}>
        enviar
      </button>
    </form>
  );
}

function Caixa({ data, grande }: TileView<InboxState> & { grande?: boolean }) {
  const conversas = data?.conversations ?? [];

  if (conversas.length === 0) {
    // Não é erro: a caixa nasce vazia todo boot porque só se enxerga o que virar
    // notificação daqui pra frente.
    return <p className="msg__vazio">sem mensagens novas</p>;
  }

  if (!grande) {
    return (
      <ul className="msg__lista">
        {conversas.slice(0, 3).map((conversa) => (
          <li key={conversa.name} className="msg__item">
            <span className="msg__nome">{conversa.name}</span>
            <span className="msg__previa">
              {conversa.messages[conversa.messages.length - 1]?.body}
            </span>
            {conversa.unread > 0 && <span className="msg__badge">{conversa.unread}</span>}
          </li>
        ))}
      </ul>
    );
  }

  return (
    <div className="msg__thread">
      {conversas.map((conversa) => (
        <section key={conversa.name} className="msg__conversa">
          <header className="msg__cabecalho">
            <span className="msg__nome">{conversa.name}</span>
            {conversa.unread > 0 && (
              <button
                className="msg__lida"
                onClick={(e: MouseEvent) => {
                  e.stopPropagation();
                  dispatchAction(MESSAGING, { acao: "lida", conversa: conversa.name });
                }}
              >
                marcar lida
              </button>
            )}
          </header>

          {conversa.messages.map((mensagem, i) => (
            <p key={i} className={`msg__balao msg__balao--${mensagem.autor}`}>
              {mensagem.body}
            </p>
          ))}

          <Resposta conversa={conversa} />
        </section>
      ))}
    </div>
  );
}

export const messagingTile: AnyTileSpec = defineTile<InboxState>({
  id: "mensagens",
  module: MESSAGING,
  title: "WhatsApp",
  area: "whatsapp",
  icon: <MessageSquare size="1em" />,
  Compact: (view) => <Caixa {...view} />,
  Expanded: (view) => <Caixa {...view} grande />,
});
