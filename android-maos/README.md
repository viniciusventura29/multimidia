# Eclipse Mãos

Um app pequeno, separado do Eclipse, que deixa o Claude **ver e tocar na tela da
central** de qualquer lugar: tira print (com o mapa), lê os elementos de
qualquer app, toca, arrasta, dá pinça, digita, aperta voltar/home, abre apps,
lê o que está tocando e instala APK sozinho.

A central não tem ADB (a FYT trancou). O que sobra, sem root, é o **serviço de
acessibilidade**: é a mesma porta que o TalkBack usa, e é a única que deixa um
app mexer em outros apps. O Mãos usa essa porta sem falar nada e sem mudar o
toque de quem dirige.

```
Claude (Code, claude.ai, iPhone) ──MCP/HTTP──▶ relay-carro (Cloudflare) ◀──WebSocket── Eclipse Mãos
```

O carro não tem endereço público, então é o Mãos que liga para fora; o relay
atende o MCP em nome dele. Ver `relay-carro/`.

## Os dados do Eclipse e a trava de velocidade

O Mãos também repassa as ferramentas do próprio Eclipse:
- `carro_telemetria`, `carro_localizacao`, `carro_musica` e `carro_mensagens`;
- `contexto_agora`;
- `eclipse_modulos`, com o estado de cada módulo e o motivo quando está degradado.

Ele as lê em `127.0.0.1:47823`, onde o Eclipse serve o catálogo (`src-tauri/src/mcp_local.rs`). O Eclipse não fala direto com o relay porque o APK dele é público, e a chave do relay não pode ir junto. Com o Eclipse fechado, essas ferramentas continuam listadas e respondem pedindo para abrir o app.

**Com o carro a mais de 5 km/h (velocidade do OBD), nada que mexa na tela funciona.** Ficam travados:
- tocar, arrastar, pinça e digitar;
- teclas, abrir app, abrir configurações e instalar APK.

Ler continua liberado, e pausar ou pular música também. Sem leitura de velocidade (Eclipse fechado, adaptador fora), não trava.

## Configuração (uma vez por Mac)

`~/.eclipse-carro.properties`, fora do git (o repositório é público):

```properties
relay=https://relay-carro.eclipsegt.workers.dev
chave=<a mesma do secret CHAVE do relay>
```

E o MCP no Claude Code, em escopo de usuário (vale para todo workspace):

```sh
claude mcp add --transport http --scope user carro "https://relay-carro.eclipsegt.workers.dev/mcp/<chave>"
```

## Compilar

```sh
cd android-maos
export JAVA_HOME=/opt/homebrew/opt/openjdk@17
./gradlew assembleDebug
```

Sai em `app/build/outputs/apk/debug/app-debug.apk` (~1,4 MB). O endereço e a
chave entram no APK pelo build; um `android-maos/maos.properties` sobrepõe o
arquivo da home (é assim que o emulador aponta para o `wrangler dev`, com
`relay=http://10.0.2.2:8787`).

## Instalar na central

**Primeira vez**, pelo navegador da central:

```sh
node scripts/carro-apk.mjs --link
```

Devolve um link curto (vale 2 h). Depois de instalar, abra o Eclipse Mãos e
ligue o que ele pedir:

1. **Acessibilidade** → Eclipse Mãos → ligar. Sem isto não há nada.
2. **Acesso a notificações** → Eclipse Mãos. É como o Claude sabe o que está tocando.
3. **Instalar apps** → permitir. Para o Claude instalar versões novas sozinho.

O print pede permissão a cada vez que a central liga; o próprio Mãos aceita.

4. **Whitelist da central:** em Configurações do carro → Fábrica (senha 8888), ligar o Eclipse Mãos e o eclipse-os nos itens 44 "Backend whitelist" e 31 "Sleep whitelist". Ligar também o Spotify no 44.
   - Sem isso, a central dá force-stop no Mãos toda vez que um app termina de instalar.
   - No Android 10, force-stop desliga o serviço de acessibilidade, e ele não volta sozinho.
   - Se o Mãos sumir, a ferramenta `eventos` conta o que aconteceu com ele.

**Daí em diante**, qualquer APK (inclusive o próprio Mãos) vai pelo relay:

```sh
node scripts/carro-apk.mjs caminho/do/app.apk
```

O Eclipse só instala por cima do que está no carro se tiver a **mesma
assinatura** (release com release; ver a memória do canal de atualização).

## Desligar

Acessibilidade → Eclipse Mãos → desligar. Ou desinstalar. O carro volta a ser
como era.

## Testar no Mac sem o carro

Emulador Android 10 (o Android real da central, que diz 16 na tela "Sobre"):

```sh
A=/opt/homebrew/share/android-commandlinetools
$A/cmdline-tools/latest/bin/sdkmanager --sdk_root=$A emulator "system-images;android-29;google_apis;arm64-v8a"
$A/cmdline-tools/latest/bin/avdmanager create avd -n central_android10 -k "system-images;android-29;google_apis;arm64-v8a"
# config.ini: hw.lcd.width = 1024, hw.lcd.height = 600, hw.lcd.density = 160,
# disk.dataPartition.size = 1G (o padrão de 6 GB não cabe no disco)
$A/emulator/emulator -avd central_android10 -no-window -no-audio -gpu swiftshader_indirect &
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell settings put secure enabled_accessibility_services com.eclipseos.maos/com.eclipseos.maos.Acessibilidade
adb shell settings put secure accessibility_enabled 1
adb shell cmd notification allow_listener com.eclipseos.maos/com.eclipseos.maos.Notificacoes
```

No emulador o adb liga as permissões; na central é pela tela de configurações.
