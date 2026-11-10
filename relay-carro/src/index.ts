import { DurableObject } from "cloudflare:workers";

// O carro não tem endereço público: está atrás do hotspot do iPhone ou do Wi-Fi
// de casa, e a central não aceita conexão de fora. Então é ele que liga para
// cá, e este relay atende o MCP em nome dele.
//
// Rotas:
//   POST /mcp/<chave>   MCP (Streamable HTTP, sem sessão) para Claude Code,
//                       claude.ai e o Claude do iPhone. A chave vai no caminho
//                       porque o conector do claude.ai não manda header próprio.
//   GET  /carro         WebSocket do carro. Chave no header `x-carro-chave`, e
//                       `x-carro-app` diz quem é ("maos", depois "eclipse").
//   PUT  /apk/<chave>   O corpo é um APK; vai em pedaços pelo WebSocket do Mãos,
//                       que instala. Nada fica guardado aqui.
//   PUT  /link/<chave>  Guarda o APK do PRÓPRIO Mãos por 2 h e devolve um link
//                       curto (/m/<código>) para baixar pelo navegador da
//                       central. É a primeira instalação: ainda não há Mãos
//                       para receber pelo WebSocket, e o APK leva a chave
//                       dentro, então não pode ir para um release público.
//   GET  /m/<código>    O download desse link.

export interface Env {
  CARRO: DurableObjectNamespace<Carro>;
  CHAVE: string;
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const partes = url.pathname.split("/").filter(Boolean);
    const rota = partes[0];

    if (!env.CHAVE) return new Response("relay sem CHAVE configurada", { status: 500 });

    // Um carro só, então um Durable Object só: é ele que segura os WebSockets
    // e casa cada pergunta com a resposta.
    const carro = env.CARRO.get(env.CARRO.idFromName("carro"));

    // O link curto não leva a chave: quem o tem é o próprio código.
    if (rota === "m" && req.method === "GET") return carro.fetch(req);

    const chave = rota === "carro" ? req.headers.get("x-carro-chave") ?? "" : partes[1] ?? "";
    if (!["mcp", "carro", "apk", "link"].includes(rota)) return new Response("não", { status: 404 });
    if (!iguais(chave, env.CHAVE)) return new Response("chave errada", { status: 401 });
    return carro.fetch(req);
  },
} satisfies ExportedHandler<Env>;

function iguais(a: string, b: string): boolean {
  const ca = new TextEncoder().encode(a);
  const cb = new TextEncoder().encode(b);
  if (ca.byteLength !== cb.byteLength) return false;
  return crypto.subtle.timingSafeEqual(ca, cb);
}

// ---------------------------------------------------------------------------

type Conteudo = { type: "text"; text: string } | { type: "image"; data: string; mimeType: string };

interface FerramentaDoCarro {
  name: string;
  description: string;
  inputSchema: unknown;
  /** Quanto esperar pela resposta. Instalar APK demora; tocar, não. */
  timeout?: number;
}

/** O que o carro manda pelo WebSocket. */
interface MensagemDoCarro {
  tipo?: string;
  id?: string;
  ferramentas?: FerramentaDoCarro[];
  info?: { maos?: string } & Record<string, unknown>;
  conteudo?: Conteudo[];
  erro?: boolean;
  recebidos?: number;
}

/** Uma requisição JSON-RPC, como chega do cliente MCP. */
interface Rpc {
  id?: string | number | null;
  method?: string;
  params?: { protocolVersion?: string; name?: string; arguments?: unknown };
}

interface Pendente {
  resolver: (r: { conteudo: Conteudo[]; erro: boolean }) => void;
  timer: ReturnType<typeof setTimeout>;
}

const TIMEOUT_PADRAO = 60_000;
/** Sem ping há este tempo, o socket está morto (o Mãos pinga a cada 20 s). */
const SOCKET_MUDO_MS = 70_000;
// Quanto do APK pode estar "no ar" sem o carro confirmar que gravou. Sem isso
// o Durable Object empurra 60 MB para o socket de uma vez e estoura a memória.
const JANELA_APK = 4 * 1024 * 1024;
const PEDACO_APK = 256 * 1024;

const ESTADO: FerramentaDoCarro = {
  name: "carro_estado",
  description:
    "Diz se o carro está conectado agora (Mãos e Eclipse), desde quando, e quando foi visto pela última vez. Use primeiro quando uma ferramenta responder que o carro está desligado.",
  inputSchema: { type: "object", properties: {} },
};

export class Carro extends DurableObject<Env> {
  private pendentes = new Map<string, Pendente>();
  /** APK em instalação. Se for o próprio Mãos, o Android o mata no meio e a
   * resposta nunca vem: quem avisa que deu certo é ele reconectando. */
  private apkEmCurso: string | null = null;
  private acks = new Map<string, { recebidos: number; acordar?: () => void }>();

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // O carro manda "ping" a cada 20 s para saber se a conexão ainda vive.
    // Responder aqui não acorda o objeto hibernado.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }

  async fetch(req: Request): Promise<Response> {
    const rota = new URL(req.url).pathname.split("/").filter(Boolean)[0];
    if (rota === "carro") return this.aceitarCarro(req);
    if (rota === "apk") return this.repassarApk(req);
    if (rota === "link") return this.guardarLink(req);
    if (rota === "m") return this.baixarLink(new URL(req.url).pathname.split("/")[2] ?? "");
    return this.mcp(req);
  }

  // --- o lado do carro ------------------------------------------------------

  private aceitarCarro(req: Request): Response {
    if (req.headers.get("upgrade")?.toLowerCase() !== "websocket") {
      return new Response("esperava WebSocket", { status: 426 });
    }
    const app = (req.headers.get("x-carro-app") ?? "maos").replace(/[^a-z0-9_-]/gi, "");
    // Reconexão depois de queda: o socket velho às vezes ainda não morreu
    // do lado de cá. Fica só o novo.
    for (const velho of this.ctx.getWebSockets(app)) velho.close(1000, "substituído");

    const [cliente, servidor] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(servidor, [app]);
    servidor.serializeAttachment({ app, desde: Date.now() });
    this.ctx.storage.kv.put(`visto:${app}`, Date.now());
    return new Response(null, { status: 101, webSocket: cliente });
  }

  async webSocketMessage(ws: WebSocket, msg: string | ArrayBuffer): Promise<void> {
    if (typeof msg !== "string") return;
    let m: MensagemDoCarro;
    try {
      m = JSON.parse(msg);
    } catch {
      return;
    }
    const { app } = ws.deserializeAttachment() as { app: string };
    this.ctx.storage.kv.put(`visto:${app}`, Date.now());

    if (m.tipo === "ola" && Array.isArray(m.ferramentas)) {
      this.ctx.storage.kv.put(`ferramentas:${app}`, m.ferramentas);
      this.ctx.storage.kv.put(`info:${app}`, m.info ?? {});
      const apk = app === "maos" ? this.apkEmCurso : null;
      const p = apk ? this.pendentes.get(apk) : undefined;
      if (apk && p) {
        clearTimeout(p.timer);
        this.pendentes.delete(apk);
        p.resolver({
          conteudo: [
            { type: "text", text: `O Eclipse Mãos reiniciou e reconectou (versão ${m.info?.maos ?? "?"}): o APK era ele mesmo e foi instalado.` },
          ],
          erro: false,
        });
      }
    } else if (m.tipo === "resposta" && typeof m.id === "string") {
      const p = this.pendentes.get(m.id);
      if (!p) return;
      clearTimeout(p.timer);
      this.pendentes.delete(m.id);
      p.resolver({ conteudo: Array.isArray(m.conteudo) ? m.conteudo : [], erro: !!m.erro });
    } else if (m.tipo === "apk_ack" && typeof m.id === "string") {
      const a = this.acks.get(m.id);
      if (!a) return;
      a.recebidos = m.recebidos ?? a.recebidos;
      a.acordar?.();
    }
  }

  async webSocketClose(ws: WebSocket, code: number, reason: string): Promise<void> {
    const { app } = ws.deserializeAttachment() as { app: string };
    this.ctx.storage.kv.put(`visto:${app}`, Date.now());
    try {
      ws.close(code, reason);
    } catch {
      // já fechado
    }
  }

  /**
   * O socket vivo do app, se houver.
   *
   * "Vivo" é ter respondido ping há pouco. A central desligada pela chave não
   * fecha a conexão — some sem FIN —, e o socket fica aberto deste lado. Visto
   * em 10/11/2026: `carro_estado` dizia "conectado há 19 min" com a central
   * desligada, e mandar por ele estourava a exceção 1101 do Worker. O Mãos
   * manda "ping" a cada 20 s; sem resposta automática há 70 s, está morto.
   */
  private socket(app: string): WebSocket | undefined {
    for (const ws of this.ctx.getWebSockets(app)) {
      const ultimoPing = this.ctx.getWebSocketAutoResponseTimestamp(ws)?.getTime();
      const { desde } = ws.deserializeAttachment() as { desde: number };
      const ultimoSinal = Math.max(ultimoPing ?? 0, desde);
      if (Date.now() - ultimoSinal < SOCKET_MUDO_MS) return ws;
      try {
        ws.close(1001, "sem ping");
      } catch {
        // já fechado
      }
    }
    return undefined;
  }

  /** `send` num socket que caiu lança; aqui isso vira "não entregou". */
  private enviar(ws: WebSocket, msg: string | ArrayBuffer | Uint8Array): boolean {
    try {
      ws.send(msg);
      return true;
    } catch {
      return false;
    }
  }

  private chamar(
    app: string,
    ferramenta: string,
    args: unknown,
    timeout: number,
  ): Promise<{ conteudo: Conteudo[]; erro: boolean }> {
    const ws = this.socket(app);
    if (!ws) return Promise.resolve(this.offline(app));
    const id = crypto.randomUUID();
    return new Promise((resolver) => {
      const timer = setTimeout(() => {
        this.pendentes.delete(id);
        resolver({
          conteudo: [{ type: "text", text: `O carro não respondeu em ${timeout / 1000} s.` }],
          erro: true,
        });
      }, timeout);
      this.pendentes.set(id, { resolver, timer });
      if (!this.enviar(ws, JSON.stringify({ tipo: "chamada", id, ferramenta, args: args ?? {} }))) {
        clearTimeout(timer);
        this.pendentes.delete(id);
        resolver(this.offline(app));
      }
    });
  }

  private offline(app: string): { conteudo: Conteudo[]; erro: boolean } {
    const visto = this.ctx.storage.kv.get<number>(`visto:${app}`);
    const quando = visto ? `Visto pela última vez há ${tempo(Date.now() - visto)}.` : "Nunca conectou.";
    return {
      conteudo: [
        {
          type: "text",
          text: `O ${app === "maos" ? "Eclipse Mãos" : app} não está conectado: a central está desligada, sem internet, ou o serviço de acessibilidade foi desligado. ${quando}`,
        },
      ],
      erro: true,
    };
  }

  // --- APK ------------------------------------------------------------------

  private async repassarApk(req: Request): Promise<Response> {
    if (req.method !== "PUT" || !req.body) return new Response("PUT com o APK no corpo", { status: 400 });
    const ws = this.socket("maos");
    if (!ws) return new Response(texto(this.offline("maos").conteudo), { status: 503 });

    const id = crypto.randomUUID();
    const tamanho = Number(req.headers.get("content-length") ?? 0);
    const controle = { recebidos: 0, acordar: undefined as undefined | (() => void) };
    this.acks.set(id, controle);

    const resposta = new Promise<{ conteudo: Conteudo[]; erro: boolean }>((resolver) => {
      const timer = setTimeout(() => {
        this.pendentes.delete(id);
        resolver({ conteudo: [{ type: "text", text: "A instalação não terminou em 5 min." }], erro: true });
      }, 5 * 60_000);
      this.pendentes.set(id, { resolver, timer });
    });

    this.apkEmCurso = id;
    if (!this.enviar(ws, JSON.stringify({ tipo: "apk_inicio", id, tamanho }))) {
      this.acks.delete(id);
      this.apkEmCurso = null;
      return new Response(texto(this.offline("maos").conteudo), { status: 503 });
    }
    let enviados = 0;
    const buffer = new Uint8Array(PEDACO_APK);
    let usado = 0;
    const despachar = async () => {
      if (usado === 0) return;
      ws.send(buffer.slice(0, usado));
      enviados += usado;
      usado = 0;
      while (enviados - controle.recebidos > JANELA_APK) {
        await new Promise<void>((acordar) => {
          controle.acordar = acordar;
          setTimeout(acordar, 30_000);
        });
        if (!this.socket("maos")) throw new Error("o carro caiu no meio do envio");
      }
    };
    try {
      const leitor = req.body.getReader();
      for (;;) {
        const { done, value } = await leitor.read();
        if (done) break;
        let off = 0;
        while (off < value.byteLength) {
          const n = Math.min(PEDACO_APK - usado, value.byteLength - off);
          buffer.set(value.subarray(off, off + n), usado);
          usado += n;
          off += n;
          if (usado === PEDACO_APK) await despachar();
        }
      }
      await despachar();
      ws.send(JSON.stringify({ tipo: "apk_fim", id, tamanho: enviados }));
    } catch (e) {
      this.acks.delete(id);
      this.apkEmCurso = null;
      return new Response(`falhou enviando: ${(e as Error).message}`, { status: 502 });
    }

    const r = await resposta;
    this.acks.delete(id);
    this.apkEmCurso = null;
    return new Response(texto(r.conteudo) + "\n", { status: r.erro ? 500 : 200 });
  }

  // --- link de instalação --------------------------------------------------

  private async guardarLink(req: Request): Promise<Response> {
    const apk = new Uint8Array(await req.arrayBuffer());
    // O armazenamento do Durable Object guarda até 2 MiB por valor; o Mãos
    // tem ~1,4 MB. Não serve para o Eclipse (60 MB), e não precisa: esse vai
    // pelo /apk, com o Mãos já instalado.
    if (apk.byteLength === 0 || apk.byteLength > 2_000_000) {
      return new Response("o link é só para o APK do Mãos (até 2 MB)", { status: 413 });
    }
    // Sem 0/o/1/l/i: vai ser digitado num teclado de tela de carro.
    const letras = "abcdefghjkmnpqrstuvwxyz23456789";
    const sorteio = crypto.getRandomValues(new Uint8Array(6));
    const codigo = [...sorteio].map((b) => letras[b % letras.length]).join("");
    const ate = Date.now() + 2 * 3600_000;
    this.ctx.storage.kv.put("link", { codigo, ate, apk });
    const url = new URL(req.url);
    return new Response(`${url.origin}/m/${codigo}\n`);
  }

  private baixarLink(codigo: string): Response {
    const link = this.ctx.storage.kv.get<{ codigo: string; ate: number; apk: Uint8Array }>("link");
    if (!link || !iguais(codigo, link.codigo)) return new Response("link inválido", { status: 404 });
    if (Date.now() > link.ate) {
      this.ctx.storage.kv.delete("link");
      return new Response("link vencido; gere outro", { status: 410 });
    }
    return new Response(link.apk, {
      headers: {
        "content-type": "application/vnd.android.package-archive",
        "content-disposition": 'attachment; filename="eclipse-maos.apk"',
      },
    });
  }

  // --- MCP ------------------------------------------------------------------

  private async mcp(req: Request): Promise<Response> {
    if (req.method === "GET") {
      // Sem stream de servidor: nada a empurrar para o cliente.
      return new Response(null, { status: 405, headers: { allow: "POST" } });
    }
    if (req.method === "DELETE") return new Response(null, { status: 204 });
    if (req.method !== "POST") return new Response(null, { status: 405 });

    let corpo: Rpc | Rpc[];
    try {
      corpo = await req.json();
    } catch {
      return json(rpcErro(null, -32700, "JSON inválido"), 400);
    }
    const lote = Array.isArray(corpo);
    const msgs: Rpc[] = Array.isArray(corpo) ? corpo : [corpo];
    const respostas = (await Promise.all(msgs.map((m) => this.rpc(m)))).filter((r) => r !== null);
    if (respostas.length === 0) return new Response(null, { status: 202 });
    return json(lote ? respostas : respostas[0]);
  }

  private async rpc(m: Rpc): Promise<unknown | null> {
    // Notificação (sem id) não leva resposta.
    if (m?.id === undefined || m?.id === null) return null;
    const id = m.id;
    switch (m.method) {
      case "initialize":
        return {
          jsonrpc: "2.0",
          id,
          result: {
            protocolVersion: m.params?.protocolVersion ?? "2025-06-18",
            capabilities: { tools: { listChanged: false } },
            serverInfo: { name: "carro", version: "1.0.0" },
            instructions:
              "Este servidor é a central multimídia do carro do dono (Eclipse OS, Android 10, tela de carro). " +
              "Use `print` para ver a tela e `tela` para a lista de elementos com posições; `tocar`, `arrastar`, " +
              "`digitar` e `tecla` para mexer. Coordenadas são pixels da tela real. Depois de agir, confira com " +
              "`print` ou `esperar_tela` em vez de supor que deu certo.",
          },
        };
      case "ping":
        return { jsonrpc: "2.0", id, result: {} };
      case "tools/list":
        // `timeout` é combinado entre o carro e o relay; não é do MCP.
        return { jsonrpc: "2.0", id, result: { tools: this.listar().map((f) => ({ name: f.name, description: f.description, inputSchema: f.inputSchema })) } };
      case "tools/call":
        return { jsonrpc: "2.0", id, result: await this.chamarFerramenta(m.params?.name ?? "", m.params?.arguments) };
      default:
        return rpcErro(id, -32601, `método desconhecido: ${m.method}`);
    }
  }

  /** As ferramentas que cada app do carro anunciou da última vez que conectou. */
  private ferramentasPorApp(): Map<string, FerramentaDoCarro[]> {
    const mapa = new Map<string, FerramentaDoCarro[]>();
    for (const [k, v] of this.ctx.storage.kv.list<FerramentaDoCarro[]>({ prefix: "ferramentas:" })) {
      mapa.set(k.slice("ferramentas:".length), v);
    }
    return mapa;
  }

  private listar(): FerramentaDoCarro[] {
    // Mesmo com o carro desligado a lista sai inteira (a última conhecida): o
    // Claude Code lê a lista uma vez ao abrir, e uma lista vazia ali esconderia
    // as ferramentas até reiniciar a sessão.
    return [ESTADO, ...[...this.ferramentasPorApp().values()].flat()];
  }

  private async chamarFerramenta(nome: string, args: unknown) {
    if (nome === ESTADO.name) return { content: [{ type: "text", text: this.estado() }] };
    for (const [app, ferramentas] of this.ferramentasPorApp()) {
      const f = ferramentas.find((x) => x.name === nome);
      if (!f) continue;
      const r = await this.chamar(app, nome, args, f.timeout ?? TIMEOUT_PADRAO);
      return { content: r.conteudo, isError: r.erro };
    }
    return { content: [{ type: "text", text: `Ferramenta desconhecida: ${nome}` }], isError: true };
  }

  private estado(): string {
    const apps = new Set<string>(["maos", ...this.ferramentasPorApp().keys()]);
    const linhas = [...apps].map((app) => {
      const ws = this.socket(app);
      const visto = this.ctx.storage.kv.get<number>(`visto:${app}`);
      const info = this.ctx.storage.kv.get<Record<string, unknown>>(`info:${app}`);
      if (ws) {
        const { desde } = ws.deserializeAttachment() as { desde: number };
        return `${app}: conectado há ${tempo(Date.now() - desde)}${info ? ` — ${JSON.stringify(info)}` : ""}`;
      }
      return `${app}: desconectado${visto ? ` (visto há ${tempo(Date.now() - visto)})` : " (nunca conectou)"}`;
    });
    return linhas.join("\n");
  }
}

function texto(conteudo: Conteudo[]): string {
  return conteudo.map((c) => (c.type === "text" ? c.text : "")).join("\n");
}

function json(corpo: unknown, status = 200): Response {
  return new Response(JSON.stringify(corpo), { status, headers: { "content-type": "application/json" } });
}

function rpcErro(id: unknown, code: number, message: string) {
  return { jsonrpc: "2.0", id, error: { code, message } };
}

function tempo(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 90) return `${s} s`;
  const min = Math.round(s / 60);
  if (min < 90) return `${min} min`;
  const h = Math.round(min / 60);
  if (h < 36) return `${h} h`;
  return `${Math.round(h / 24)} dias`;
}
