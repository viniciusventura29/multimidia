#!/usr/bin/env node
// Mexer na tela do Eclipse DEV a partir do Mac — o par de `ponte-do-carro.ts`.
//
//   node scripts/carro.mjs print [arquivo.png]   # print da tela (padrão em .context/)
//   node scripts/carro.mjs texto                 # o texto visível na tela
//   node scripts/carro.mjs click "<seletor css>" # toca no elemento
//   node scripts/carro.mjs toque X Y             # toca na coordenada
//   node scripts/carro.mjs eval "<js>"           # roda JS na tela e devolve o valor
//   node scripts/carro.mjs log [n]               # as últimas n linhas do console
//
// Precisa do `node scripts/eclipse-dev.mjs tela` rodando e do app aberto.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const RAIZ = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const arquivoUrl = path.join(RAIZ, ".ponte", "url.txt");
const BASE =
  process.env.PONTE_URL ??
  (existsSync(arquivoUrl) ? readFileSync(arquivoUrl, "utf8").trim() : "http://localhost:1420");

async function mandar(corpo) {
  const resposta = await fetch(`${BASE}/__carro`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(corpo),
  });
  const json = await resposta.json();
  if (!json.ok) {
    console.error(json.erro);
    process.exit(1);
  }
  return json.valor;
}

const [comando, ...args] = process.argv.slice(2);
switch (comando) {
  case "print": {
    const dataUrl = await mandar({ tipo: "print" });
    const destino =
      args[0] ?? path.join(RAIZ, ".context", `carro-${new Date().toISOString().slice(11, 19).replaceAll(":", "")}.png`);
    writeFileSync(destino, Buffer.from(dataUrl.split(",")[1], "base64"));
    console.log(destino);
    break;
  }
  case "texto":
    console.log(await mandar({ tipo: "texto" }));
    break;
  case "click":
    console.log(await mandar({ tipo: "click", arg: args[0] }));
    break;
  case "toque":
    console.log(await mandar({ tipo: "toque", x: Number(args[0]), y: Number(args[1]) }));
    break;
  case "eval":
    console.log(await mandar({ tipo: "eval", arg: args.join(" ") }));
    break;
  case "log": {
    const log = path.join(RAIZ, ".context", "carro.log");
    const linhas = existsSync(log) ? readFileSync(log, "utf8").trimEnd().split("\n") : [];
    console.log(linhas.slice(-Number(args[0] ?? 50)).join("\n"));
    break;
  }
  default:
    console.log("uso: node scripts/carro.mjs print | texto | click <sel> | toque X Y | eval <js> | log [n]");
    process.exit(1);
}
