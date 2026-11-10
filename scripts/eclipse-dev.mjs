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
  // O IP do Mac muda quando ele troca de rede (hotspot → Wi-Fi de casa). Se a
  // página só soubesse o IP de quando foi compilada, cada troca custaria um APK
  // novo. Então ela tenta o último IP que funcionou (guardado no localStorage,
  // que é do app e sobrevive a reinício) e, enquanto não acha, deixa digitar.
  return `<!doctype html>
<html>
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no, viewport-fit=cover" />
<title>Eclipse DEV</title>
<style>
  html, body { margin: 0; height: 100%; background: #000; color: #8a8f98; }
  body { display: grid; place-items: center; font: 22px system-ui, sans-serif; text-align: center; }
  small { display: block; margin-top: 10px; font-size: 15px; opacity: .6; }
  form { margin-top: 28px; display: flex; gap: 12px; justify-content: center; }
  input, button { font: inherit; padding: 12px 16px; border-radius: 10px; border: 1px solid #333; }
  input { background: #111; color: #ddd; width: 300px; }
  button { background: #1db954; color: #000; border: 0; }
</style>
</head>
<body>
<div>
  <div id="msg">procurando o Mac…</div>
  <form id="form">
    <input id="ip" inputmode="decimal" autocomplete="off" placeholder="IP do Mac" />
    <button>conectar</button>
  </form>
</div>
<script>
  const CHAVE = "eclipse-dev-mac";
  const PADRAO = ${JSON.stringify(urlDoMac)};
  let mac = localStorage.getItem(CHAVE) || PADRAO;
  let tentativa = null;
  // Uma tentativa num IP velho pode terminar DEPOIS de outra já ter carregado
  // a tela; sem esta trava ela reescreveria o documento e reiniciaria o painel.
  let carregou = false;
  const campo = document.getElementById("ip");
  campo.value = new URL(mac).hostname;

  document.getElementById("form").addEventListener("submit", (e) => {
    e.preventDefault();
    const ip = campo.value.trim();
    if (!ip) return;
    mac = "http://" + ip + ":${PORTA}";
    clearTimeout(tentativa);
    tentar();
  });

  async function tentar() {
    if (carregou) return;
    const base = mac + "/";
    document.getElementById("msg").innerHTML = "procurando o Mac em " + base;
    try {
      const resposta = await fetch(base, { cache: "no-store" });
      if (!resposta.ok) throw new Error("HTTP " + resposta.status);
      const html = (await resposta.text()).replace(/<head([^>]*)>/i, '<head$1><base href="' + base + '">');
      if (carregou) return;
      carregou = true;
      localStorage.setItem(CHAVE, mac);
      document.open();
      document.write(html);
      document.close();
    } catch (e) {
      if (carregou) return;
      document.getElementById("msg").innerHTML =
        "esperando o Mac em " + base +
        "<small>" + e.message + " — o \\"eclipse-dev tela\\" está rodando? Mesma rede? Se o IP mudou, digite abaixo.</small>";
      tentativa = setTimeout(tentar, 3000);
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
