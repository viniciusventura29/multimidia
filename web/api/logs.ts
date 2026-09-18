import { get, list, put } from "@vercel/blob";

/**
 * A central de logs do Eclipse.
 *
 * O painel roda numa head unit dentro de um carro: sem console, sem `adb` e sem
 * ninguém olhando. Este endpoint é onde o que deu errado lá chega para ser lido
 * daqui. Duas rotas e nada mais — `POST` grava um lote, `GET` devolve o que
 * houver.
 *
 * Um lote por blob, e não um arquivo que cresce: o Blob não tem append, e
 * ler-alterar-gravar um arquivo único perderia lotes que chegassem juntos. O
 * nome carrega a data e a sessão, que é o que permite filtrar sem abrir tudo.
 *
 * Privado de propósito. Esses logs carregam a localização do carro (arredondada
 * na origem, mas ainda assim) e o MAC do adaptador — nada aqui deve ser
 * alcançável por quem souber a URL.
 */

/** O teto de um lote. Acima disso é engano ou abuso, não depuração. */
const MAX_LINHAS = 500;

interface Linha {
  ts: string;
  nivel: "debug" | "info" | "aviso" | "erro";
  onde: string;
  msg: string;
  dados?: Record<string, unknown>;
}

interface Lote {
  sessao: string;
  versao: string;
  linhas: Linha[];
}

/**
 * O Blob só funciona com a store conectada ao projeto, que é o que injeta o
 * `BLOB_READ_WRITE_TOKEN`. Sem ele o SDK estoura um erro genérico e a resposta
 * vira um 500 mudo — que é exatamente o tipo de falha que esta central existe
 * para não deixar acontecer. Melhor dizer o que falta.
 */
function semStore(): Response | null {
  if (process.env.BLOB_READ_WRITE_TOKEN) return null;
  return Response.json(
    { erro: "a store do Blob não está conectada ao projeto (falta BLOB_READ_WRITE_TOKEN)" },
    { status: 503 },
  );
}

function autorizado(req: Request): boolean {
  const esperada = process.env.ECLIPSE_LOGS_CHAVE;
  // Sem chave configurada o endpoint fica FECHADO, não aberto. O padrão inseguro
  // seria descobrir o buraco depois de o carro já estar publicando.
  if (!esperada) return false;
  return req.headers.get("x-eclipse-chave") === esperada;
}

export async function POST(req: Request): Promise<Response> {
  if (!autorizado(req)) return new Response("não", { status: 401 });
  const semBlob = semStore();
  if (semBlob) return semBlob;

  let lote: Lote;
  try {
    lote = (await req.json()) as Lote;
  } catch {
    return new Response("json inválido", { status: 400 });
  }

  if (!Array.isArray(lote.linhas) || lote.linhas.length === 0) {
    return new Response("lote vazio", { status: 400 });
  }
  if (lote.linhas.length > MAX_LINHAS) {
    return new Response("lote grande demais", { status: 413 });
  }

  const dia = new Date().toISOString().slice(0, 10);
  const sessao = (lote.sessao ?? "sem-sessao").slice(0, 40).replace(/[^\w-]/g, "");
  const nome = `logs/${dia}/${sessao}/${Date.now()}.json`;

  await put(nome, JSON.stringify({ ...lote, recebidoEm: new Date().toISOString() }), {
    access: "private",
    contentType: "application/json",
    // Sem sufixo aleatório: o nome já é único pelo timestamp, e um nome estável
    // é o que deixa listar por dia e por sessão fazer sentido.
    addRandomSuffix: false,
  });

  return Response.json({ ok: true, guardadas: lote.linhas.length });
}

export async function GET(req: Request): Promise<Response> {
  if (!autorizado(req)) return new Response("não", { status: 401 });
  const semBlob = semStore();
  if (semBlob) return semBlob;

  const url = new URL(req.url);
  const dia = url.searchParams.get("dia") ?? new Date().toISOString().slice(0, 10);
  const sessao = url.searchParams.get("sessao");
  const nivel = url.searchParams.get("nivel");
  const limite = Math.min(Number(url.searchParams.get("limite") ?? 200), 1000);

  const prefixo = sessao ? `logs/${dia}/${sessao}/` : `logs/${dia}/`;
  const { blobs } = await list({ prefix: prefixo, limit: 1000 });

  // Do mais novo para o mais velho: a pergunta é sempre "o que acabou de
  // acontecer", e ninguém lê log de trás para frente.
  const ordenados = blobs.sort((a, b) => b.pathname.localeCompare(a.pathname));

  const linhas: (Linha & { sessao: string; versao: string })[] = [];
  for (const blob of ordenados) {
    if (linhas.length >= limite) break;

    // `get` do SDK, e não `fetch(blob.url)`: a store é privada, e a URL de um
    // blob privado não é buscável sem autenticação. Um `fetch` cru aqui não dá
    // erro — devolve 401 e a lista volta vazia, que foi exatamente como isto
    // falhou da primeira vez.
    const conteudo = await get(blob.pathname, { access: "private" });
    if (!conteudo || conteudo.statusCode !== 200) continue;

    const lote = JSON.parse(await new Response(conteudo.stream).text()) as Lote;
    for (const linha of lote.linhas) {
      if (nivel && linha.nivel !== nivel) continue;
      linhas.push({ ...linha, sessao: lote.sessao, versao: lote.versao });
    }
  }

  const recorte = linhas.slice(0, limite);
  return Response.json({
    dia,
    lotes: ordenados.length,
    // Duas contagens porque elas divergem, e a diferença é informação: se
    // `encontradas` for maior que `total`, o limite cortou e há mais para ver.
    encontradas: linhas.length,
    total: recorte.length,
    linhas: recorte,
  });
}
