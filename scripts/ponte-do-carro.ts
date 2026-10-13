// A ponte do carro, lado do Mac: um plugin do Vite que só existe no `serve`.
//
// A central não tem ADB — a FYT desligou o modo programador no firmware, e
// todas as portas (senha `adbon`, número de compilação, EngineerMode) estão
// trancadas. O "Eclipse DEV" (`scripts/eclipse-dev.mjs`) puxa a tela deste
// Vite pela rede, e este plugin é o que faz dessa conexão um console:
//
// - o que o WebView do carro escreve no console chega em `.context/carro.log`;
// - `POST /__carro` manda um comando para a tela (rodar JS, clicar, ler o
//   texto, tirar print) e devolve a resposta — `scripts/carro.mjs` é o cliente;
// - `GET /__carro/apk` entrega o APK dev, para instalar pelo navegador do carro.
//
// Tudo passa pelo WebSocket do HMR, que já está aberto de qualquer jeito: nada
// de porta nova nem servidor a mais.

import { createReadStream, existsSync, mkdirSync, statSync, appendFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import path from "node:path";
import type { Plugin } from "vite";

/** Teto de uma resposta. O print de uma tela inteira leva uns 2 s no carro. */
const TETO_MS = 15_000;

export function ponteDoCarro(raiz: string): Plugin {
  const contexto = path.join(raiz, ".context");
  const arquivoLog = path.join(contexto, "carro.log");
  const apk = path.join(raiz, ".ponte", "eclipse-dev.apk");

  return {
    name: "ponte-do-carro",
    apply: "serve",
    configureServer(server) {
      mkdirSync(contexto, { recursive: true });

      server.ws.on("carro:log", (dados: { nivel: string; texto: string }) => {
        const hora = new Date().toTimeString().slice(0, 8);
        appendFileSync(arquivoLog, `${hora} ${dados.nivel.padEnd(5)} ${dados.texto}\n`);
      });

      const pendentes = new Map<string, (resposta: unknown) => void>();
      server.ws.on("carro:resp", (dados: { id: string }) => {
        pendentes.get(dados.id)?.(dados);
        pendentes.delete(dados.id);
      });

      server.middlewares.use("/__carro/apk", (_req, res) => {
        if (!existsSync(apk)) {
          res.statusCode = 404;
          res.end("sem APK — rode `node scripts/eclipse-dev.mjs apk` no Mac");
          return;
        }
        res.setHeader("content-type", "application/vnd.android.package-archive");
        res.setHeader("content-length", statSync(apk).size);
        res.setHeader("content-disposition", 'attachment; filename="eclipse-dev.apk"');
        createReadStream(apk).pipe(res);
      });

      server.middlewares.use("/__carro", (req, res, next) => {
        if (req.method !== "POST") return next();
        let corpo = "";
        req.on("data", (pedaco) => (corpo += pedaco));
        req.on("end", () => {
          const id = randomUUID();
          const responder = (status: number, valor: unknown) => {
            res.statusCode = status;
            res.setHeader("content-type", "application/json");
            res.end(JSON.stringify(valor));
          };
          const teto = setTimeout(() => {
            pendentes.delete(id);
            responder(504, { ok: false, erro: "a tela não respondeu — o Eclipse DEV está aberto?" });
          }, TETO_MS);
          pendentes.set(id, (resposta) => {
            clearTimeout(teto);
            responder(200, resposta);
          });
          try {
            server.ws.send("carro:cmd", { id, ...JSON.parse(corpo) });
          } catch (e) {
            clearTimeout(teto);
            pendentes.delete(id);
            responder(400, { ok: false, erro: String(e) });
          }
        });
      });
    },
  };
}
