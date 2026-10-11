#!/usr/bin/env node
// O "Eclipse DEV": programar na central sem ADB.
//
// A central (FYT/UIS7862, Android 10) não tem modo programador — o firmware o
// desligou, e todas as portas conhecidas estão trancadas. Então a ponte mora no
// app: um segundo app, `com.eclipseos.app.dev`, cujo Rust roda no carro de
// verdade (OBD, Bluetooth, GPS, mídia) mas cuja TELA vem do Vite deste Mac.
//
//   node scripts/eclipse-dev.mjs tela   # sobe o Vite na rede (deixe rodando)
//   node scripts/eclipse-dev.mjs apk    # compila o APK dev (só quando o Rust muda)
//
// Mudou React/CSS → aparece no carro sozinho (HMR). Mudou Rust → `apk` de novo
// e, no navegador da central, abrir o link que ele imprime e tocar em instalar.
// Console e comandos: ver `scripts/carro.mjs`.
//
// COMO A TELA CHEGA LÁ. O APK não leva o `dist/`: leva só a página que este
// script escreve em `.ponte/www/index.html`. Ela busca o `index.html` que o Vite
// serve, põe um `<base href>` apontando para o Mac e o escreve no lugar de si
// mesma (`document.write`). A origem continua sendo a do app (`tauri.localhost`)
// — por isso o IPC com o Rust funciona sem liberar origem remota nenhuma nas
// capabilities — e todo caminho `/src/...` resolve para o Mac por causa do
// `<base>`. O CORS do Vite já aceita `*.localhost` por padrão.

import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const RAIZ = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PONTE = path.join(RAIZ, ".ponte");
const PORTA = 1420;

/** O IP do Mac na rede (o hotspot do iPhone). `ECLIPSE_MAC_IP` sobrepõe. */
function ipDoMac() {
  if (process.env.ECLIPSE_MAC_IP) return process.env.ECLIPSE_MAC_IP;
  try {
    return execFileSync("ipconfig", ["getifaddr", "en0"], { encoding: "utf8" }).trim();
  } catch {
    console.error("sem IP no Wi-Fi (en0). Conecte o Mac na mesma rede da central.");
    process.exit(1);
  }
}

function pagina(urlDoMac) {
  return `<!doctype html>
<html>
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no, viewport-fit=cover" />
<title>Eclipse DEV</title>
<style>
  html, body { margin: 0; height: 100%; background: #000; color: #8a8f98; }
  body { display: grid; place-items: center; font: 22px system-ui, sans-serif; text-align: center; }
  small { display: block; margin-top: 12px; font-size: 15px; opacity: .6; }
</style>
</head>
<body>
<div id="msg">procurando o Mac em ${urlDoMac}</div>
<script>
  const MAC = ${JSON.stringify(urlDoMac + "/")};
  async function tentar() {
    try {
      const resposta = await fetch(MAC, { cache: "no-store" });
      if (!resposta.ok) throw new Error("HTTP " + resposta.status);
      const html = (await resposta.text()).replace(/<head([^>]*)>/i, '<head$1><base href="' + MAC + '">');
      document.open();
      document.write(html);
      document.close();
    } catch (e) {
      document.getElementById("msg").innerHTML =
        "esperando o Mac em " + MAC + "<small>" + e.message + " — o \\"eclipse-dev tela\\" está rodando?</small>";
      setTimeout(tentar, 2000);
    }
  }
  tentar();
</script>
</body>
</html>
`;
}

function tela() {
  const ip = ipDoMac();
  const url = `http://${ip}:${PORTA}`;
  mkdirSync(PONTE, { recursive: true });
  writeFileSync(path.join(PONTE, "url.txt"), url);
  console.log(`\n  tela do carro servida em ${url}`);
  console.log(`  APK dev para instalar:  ${url}/__carro/apk`);
  console.log(`  console do carro:       .context/carro.log\n`);
  const vite = spawn("npx", ["vite"], {
    cwd: RAIZ,
    stdio: "inherit",
    env: { ...process.env, TAURI_DEV_HOST: ip },
  });
  vite.on("exit", (codigo) => process.exit(codigo ?? 0));
}

/** As chaves entram por ARQUIVO ao lado do Cargo.toml — ver `src-tauri/build.rs`. */
function copiarChaves() {
  const origem = path.join(homedir(), "Library/Application Support/com.eclipseos.app");
  for (const nome of ["maps_api_key", "spotify_client_id", "anthropic_api_key", "openrouter_api_key"]) {
    const de = path.join(origem, `${nome}.txt`);
    const para = path.join(RAIZ, "src-tauri", `${nome}.txt`);
    if (!existsSync(para) && existsSync(de)) copyFileSync(de, para);
  }
}

function ambienteAndroid() {
  const env = { ...process.env };
  env.JAVA_HOME ??= "/opt/homebrew/opt/openjdk@17";
  env.ANDROID_HOME ??= "/opt/homebrew/share/android-commandlinetools";
  if (!env.NDK_HOME) {
    const ndks = existsSync(path.join(env.ANDROID_HOME, "ndk"))
      ? readdirSync(path.join(env.ANDROID_HOME, "ndk")).sort()
      : [];
    if (ndks.length) env.NDK_HOME = path.join(env.ANDROID_HOME, "ndk", ndks.at(-1));
  }
  env.ANDROID_NDK_HOME ??= env.NDK_HOME;
  env.PATH = `${path.join(homedir(), ".cargo/bin")}:${env.PATH}`;
  return env;
}

function apk() {
  const ip = ipDoMac();
  const url = `http://${ip}:${PORTA}`;
  const www = path.join(PONTE, "www");
  mkdirSync(www, { recursive: true });
  writeFileSync(path.join(www, "index.html"), pagina(url));
  copiarChaves();

  // `null` apaga a chave no merge do `--config`: sem `npm run build`, porque o
  // APK dev não leva o painel, só a página acima.
  const config = JSON.stringify({ build: { frontendDist: "../.ponte/www", beforeBuildCommand: null } });
  execFileSync(
    "npx",
    ["tauri", "android", "build", "--apk", "--debug", "--target", "aarch64", "--config", config],
    { cwd: RAIZ, stdio: "inherit", env: ambienteAndroid() },
  );

  const gerado = path.join(
    RAIZ,
    "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk",
  );
  copyFileSync(gerado, path.join(PONTE, "eclipse-dev.apk"));
  console.log(`\n  APK pronto. Com o "tela" rodando, abra no navegador da central:`);
  console.log(`  ${url}/__carro/apk\n`);
}

const comando = process.argv[2];
if (comando === "tela") tela();
else if (comando === "apk") apk();
else {
  console.log("uso: node scripts/eclipse-dev.mjs tela | apk");
  process.exit(1);
}
