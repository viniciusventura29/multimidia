# De onde veio este binário

`spotify-app-remote-release-0.8.0.aar`

- **Origem:** https://github.com/spotify/android-sdk/releases (repositório oficial
  da Spotify), tag `v0.8.0-appremote_v2.1.0-auth`.
- **SHA-256:** `b5a6dd880eaf01f63a871cba9ef7af77c341f8a94ffc8fdf2e9021f9a9d4c198`
- **Tamanho:** 132.749 bytes

## Por que um binário no repositório

Porque não há alternativa. A Spotify **não publica** o App Remote em repositório
de dependências nenhum — nem Maven Central, nem Google, nem um Maven próprio.
Conferido: `com.spotify.android:auth` está no Maven Central, o App Remote não
está em lugar algum. O único jeito de obtê-lo é este arquivo solto.

## Por que ele é necessário

O Eclipse passou três versões tentando mandar o som para o app do Spotify da
central pela Web API, e falhou por um motivo que não tem contorno: para a Web
API comandar um aparelho, ele precisa estar anunciado como dispositivo do
Spotify Connect — e o app do Spotify só se anuncia depois de ser ABERTO.

Tentativas que não resolveram, e por quê:

1. Casar o nome do aparelho com o do dispositivo. O Android chama a central de
   "K706" em todos os campos que conhece; o Spotify se anuncia como
   "HT-9960CA". Não há relação entre os dois.
2. Acordar o app ligando no `MediaBrowserService` dele. Eu acreditei que
   funcionava porque o dispositivo apareceu ~51 s depois de um bind no diário
   de 24/09 — mas o diário de 25/09 mostra o bind acontecendo e NENHUM
   dispositivo novo aparecendo. Era correlação, e o dono provavelmente tinha
   aberto o Spotify na mão naquele minuto.

O App Remote não precisa de nada disso: ele fala com o app do Spotify **deste
aparelho**, inicia o processo dele sozinho, e manda tocar direto. Não há lista
de dispositivos, não há nome para casar, não há risco de o som sair no
computador de casa.

## O que precisa estar cadastrado (e já está)

No painel de desenvolvedor do Spotify, no mesmo app do deep link:

- pacote: `com.eclipseos.app`
- SHA1: `2A:18:98:48:FE:2F:E6:94:C6:79:32:CD:99:F9:E2:0C:39:85:0E:08`

Sem isso o SDK recusa a conexão.

## Como atualizar

Baixar a versão nova do mesmo lugar, conferir que o `sha256` acima mudou de
propósito, e trocar o número no `build.gradle.kts`.
