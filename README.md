# Eclipse OS

Computador de bordo para o Mitsubishi Eclipse GT 2000. Rust no núcleo, Tauri como
casca, React na tela. Roda no Mac hoje; o alvo é uma head unit Android 2-DIN.

## Rodar

```sh
nvm use          # Node 24 — há um Node 18 em /usr/local/bin que atrapalha
npm install
npm run tauri dev
```

Sem nenhuma credencial configurada o painel abre normalmente: navegação e Spotify
aparecem degradados dizendo o que falta, e o resto funciona.

## Credenciais

Cada uma pode vir de variável de ambiente (para desenvolver) ou de um arquivo no
diretório de dados do app (para a head unit, onde não há shell antes do launcher):

| O quê | Variável | Arquivo |
|---|---|---|
| Client ID do Spotify | `ECLIPSE_SPOTIFY_CLIENT_ID` | `spotify_client_id.txt` |
| Chave da Maps JavaScript API | `ECLIPSE_MAPS_API_KEY` | `maps_api_key.txt` |

No macOS o diretório é `~/Library/Application Support/com.eclipseos.app`.

O Spotify exige um app registrado em [developer.spotify.com](https://developer.spotify.com),
com `http://127.0.0.1:8888/callback` cadastrado como Redirect URI, e conta Premium.
**Configure um teto de cota no Google Cloud antes da primeira chamada ao Maps.**

`ECLIPSE_MUSIC_DEMO=1` troca o Spotify por faixas de mentira, para trabalhar no
layout sem Client ID.

## Como está organizado

```
crates/
  eclipse-core       contrato de módulo, barramento, supervisor, perfis
  eclipse-obd        cadência de varredura e o trajeto simulado
  eclipse-music      cofre de tokens e a ponte com o Spotify
  eclipse-messaging  caixa de entrada e a fonte de mensagens
src-tauri            fiação: eventos, comandos, registro dos módulos
src                  React: grid, widgets, perfis
```

O Rust é dono de todo o estado; a tela é uma projeção. Nenhuma regra de negócio
no React.

**Módulo e tile são coisas diferentes.** Os cinco mostradores leem todos do módulo
`obd`, porque é um ELM327 e um barramento — por isso caem juntos quando o
adaptador solta, sem contaminar música e navegação.

**Falhar é um estado, não uma exceção.** `ModuleState::Degraded` carrega o último
valor bom: o tile fica esmaecido mostrando o número velho em vez de sumir. Cada
módulo roda na própria task; erro *ou pânico* viram degradação com backoff, e os
vizinhos não percebem.

## Limites que o desenho respeita

Não são pendências — são o que a plataforma permite:

- **Navegação turn-by-turn embutida não existe.** O Maps SDK dá mapa, não
  navegação; o Navigation SDK, que dá, é enterprise. Guiar de verdade é abrir o
  app do Google Maps por cima.
- **Perfil só troca conta no Spotify.** WhatsApp é uma conta por aparelho e o
  mapa embutido não fica logado numa conta Google.
- **A sessão do Spotify dura no máximo 6 meses.** O refresh token expira contado
  do login original, e renovar o access token não estende. Reconectar é estado
  previsto do painel, com um toque.
- **O carro entrega 1 a 3 leituras por segundo.** O Eclipse 2000 é ISO 9141-2 a
  10.400 baud, não CAN. O simulador respeita essa lentidão de propósito.
- **A câmera de ré não passa pelo app.** O chaveamento é feito em hardware pelo
  MCU da head unit, antes do Android.

## O que falta

- Plugin Kotlin com `NotificationListenerService` — hoje as mensagens vêm de um
  mock. Depende da head unit comprada.
- OBD real via ELM327 (comprar a variante **Bluetooth Clássico/SPP**, não BLE).
- Câmera lateral via USB/UVC.
- APK, launcher, viagens, rádio.
