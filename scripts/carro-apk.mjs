#!/usr/bin/env node
// Instala um APK do Mac na central do carro, de qualquer lugar.
//
//   node scripts/carro-apk.mjs caminho/do/app.apk
//   node scripts/carro-apk.mjs --link [caminho/do/maos.apk]
//
// O segundo é a PRIMEIRA instalação do Eclipse Mãos: devolve um link curto
// (vale 2 h) para digitar no navegador da central. O padrão é o APK que o
// Gradle acabou de gerar em android-maos/.
//
// O carro não alcança o Mac, então o arquivo vai pelo relay (relay-carro/):
// o relay repassa em pedaços pelo WebSocket que o Eclipse Mãos abriu, e o Mãos
// instala, tocando em "Instalar" sozinho. Nada fica guardado no caminho.
//
// O endereço e a chave vêm de ~/.eclipse-carro.properties (fora do git, o
// repositório é público), o mesmo arquivo que o build do Mãos lê:
//   relay=https://relay-carro.<conta>.workers.dev
//   chave=<a chave>
import { readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const link = process.argv[2] === "--link";
const apk = process.argv[link ? 3 : 2] ?? (link ? "android-maos/app/build/outputs/apk/debug/app-debug.apk" : undefined);
if (!apk) {
  console.error("uso: node scripts/carro-apk.mjs <arquivo.apk>\n     node scripts/carro-apk.mjs --link [maos.apk]");
  process.exit(2);
}

const arquivo = join(homedir(), ".eclipse-carro.properties");
let conteudo = "";
try {
  conteudo = readFileSync(arquivo, "utf8");
} catch {
  if (!process.env.CARRO_RELAY) {
    console.error(`falta ${arquivo} (relay=… e chave=…)`);
    process.exit(2);
  }
}
const config = Object.fromEntries(
  conteudo
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#") && l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")).trim(), l.slice(l.indexOf("=") + 1).trim()]),
);
// O build do Mãos pode apontar para outro relay (o emulador fala com o
// wrangler dev por 10.0.2.2); este script fala do Mac.
const relay = (process.env.CARRO_RELAY ?? config.relay_mac ?? config.relay).replace(/\/$/, "");
const chave = process.env.CARRO_CHAVE ?? config.chave;

const tamanho = statSync(apk).size;
if (link) {
  const r = await fetch(`${relay}/link/${encodeURIComponent(chave)}`, { method: "PUT", body: readFileSync(apk) });
  const texto = (await r.text()).trim();
  console.log(r.ok ? `Digite no navegador da central (vale 2 h):\n\n  ${texto}\n` : texto);
  process.exit(r.ok ? 0 : 1);
}
console.log(`enviando ${(tamanho / 1024 / 1024).toFixed(1)} MB para o carro…`);
const inicio = Date.now();
const r = await fetch(`${relay}/apk/${encodeURIComponent(chave)}`, {
  method: "PUT",
  headers: { "content-type": "application/vnd.android.package-archive", "content-length": String(tamanho) },
  body: readFileSync(apk),
});
const texto = (await r.text()).trim();
console.log(`${texto} (${((Date.now() - inicio) / 1000).toFixed(0)} s)`);
process.exit(r.ok ? 0 : 1);
