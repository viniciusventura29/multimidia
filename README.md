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
| Map ID vetorial | `ECLIPSE_MAPS_MAP_ID` | `maps_map_id.txt` |
| Chave da API da Anthropic | `ECLIPSE_ANTHROPIC_API_KEY` | `anthropic_api_key.txt` |
| Chave do OpenRouter (só imagem) | `ECLIPSE_OPENROUTER_API_KEY` | `openrouter_api_key.txt` |

No macOS o diretório é `~/Library/Application Support/com.eclipseos.app`.

O Spotify exige um app registrado em [developer.spotify.com](https://developer.spotify.com),
com `http://127.0.0.1:8888/callback` cadastrado como Redirect URI, e conta Premium.
**Configure um teto de cota no Google Cloud antes da primeira chamada ao Maps.**

O Map ID precisa ser criado em *Google Maps Platform → Map Management*, tipo
JavaScript, renderização **Vector**. Sem ele o mapa é raster, e mapa raster
ignora inclinação e rotação: fica sempre chapado, olhando para o norte. É esse
Map ID que separa "um mapa na tela" de "modo navegação".

`ECLIPSE_MUSIC_DEMO=1` troca o Spotify por faixas de mentira, para trabalhar no
layout sem Client ID.

## Como está organizado

```
crates/
  eclipse-core       contrato de módulo, barramento, supervisor, perfis
  eclipse-sim        o carro imaginário, lido pelo OBD e pelo GPS
  eclipse-obd        cadência de varredura dos PIDs
  eclipse-gps        posição, rumo e o traçado percorrido
  eclipse-music      cofre de tokens e a ponte com o Spotify
  eclipse-messaging  caixa de entrada e a fonte de mensagens
  eclipse-mcp        o catálogo de ferramentas, no formato do MCP
  eclipse-ia         o laço do agente e o quadro que ele pinta
src-tauri            fiação: eventos, comandos, registro dos módulos
src                  React: grid, widgets, perfis
```

O Rust é dono de todo o estado; a tela é uma projeção. Nenhuma regra de negócio
no React.

**Módulo e tile são coisas diferentes.** Os cinco mostradores leem todos do módulo
`obd`, porque é um ELM327 e um barramento — por isso caem juntos quando o
adaptador solta, sem contaminar música e navegação.

**Um simulador só, vários sensores.** No carro real o OBD e o GPS observam o
mesmo movimento. Se cada simulador inventasse o seu, o painel mostraria o motor
acelerando com o mapa parado — e pareceria funcionar, contando duas histórias.
Por isso o `eclipse-sim` é função pura do tempo: dois sensores amostrando em
ritmos diferentes (OBD a 300 ms, GPS a 1 s) ficam coerentes de graça.

**Falhar é um estado, não uma exceção.** `ModuleState::Degraded` carrega o último
valor bom: o tile fica esmaecido mostrando o número velho em vez de sumir. Cada
módulo roda na própria task; erro *ou pânico* viram degradação com backoff, e os
vizinhos não percebem.

## O assistente

A coluna da esquerda é uma IA **proativa**: ninguém digita nem fala com ela, e é
de propósito — quem está na frente dela está dirigindo. Ela é acionada por
acontecimentos e escreve sozinha.

| Gatilho | Quando | Modelo |
|---|---|---|
| ignição | o painel subiu | Haiku 4.5 |
| rota definida | um destino novo foi traçado | Opus 4.8 |
| alerta do carro | temperatura, combustível ou tensão cruzaram o limiar | Opus 4.8 |
| periódico | viagem longa em curso, a cada 20 min | Haiku 4.5 |
| chegada | chegamos | Opus 4.8 |

**Tudo que ela sabe fazer é uma ferramenta MCP.** Carro, mapa, música,
mensagens, relógio, foto de lugar, geração de imagem e a própria escrita no
quadro entram todas pelo mesmo catálogo (`eclipse-mcp`), com o mesmo formato do
`tools/list` e `tools/call`. Um `Registro` respondendo por `protocolo::atender`
já **é** um servidor MCP; falta só um transporte para um cliente externo se
conectar.

**Os dados do carro não passam pelo conector de MCP da Anthropic**, e isso não é
escolha: aquele conector alcança servidores remotos por HTTP, porque quem conecta
é a Anthropic. Uma head unit atrás do 4G do celular não tem endereço público.
Então o carro é ferramenta local, executada aqui, e o conector fica para os
servidores remotos que você quiser plugar em `mcp_servers.json`.

**A única saída é `pintar_quadro`.** Texto solto na resposta do modelo é
ignorado; a tela desenha cartões tipados (texto, métrica, gráfico, imagem,
lista). É o que evita duas maneiras de dizer a mesma coisa.

**Sem novidade, a coluna vira o carro.** As rodas giram na velocidade que o OBD
está relatando, a carroceria mergulha quando o carro freia de verdade e a luz de
freio acende junto. Parado, ele fica em marcha lenta soltando fumaça. Essa parte
funciona sem credencial nenhuma.

Ajustes ficam em `assistente.json` no diretório de dados — teto de chamadas por
dia (padrão 40), teto de imagens (4), o modelo de imagem do OpenRouter, e a lista
de servidores MCP remotos. O gasto é contado em `assistente_uso.json`, que
sobrevive ao desligamento: numa head unit o app morre junto com o carro, e teto
diário que vivesse só na memória não seria teto nenhum.

`ECLIPSE_IA_DEMO=1` pinta cartões de mentira sem tocar na API, e
`ECLIPSE_IA_GATILHO=rota` (ou `ignicao`, `periodico`, `chegada`, `alerta`) força
um gatilho na subida — dá para trabalhar no layout sem sair dirigindo.

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
- GPS real via `LocationManager` — hoje a posição vem de um traçado simulado
  pela Av. Paulista.
- OBD real via ELM327 (comprar a variante **Bluetooth Clássico/SPP**, não BLE).
- Câmera lateral via USB/UVC.
- APK, launcher, viagens, rádio.
