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

## O carro

Tamanho do tanque, cilindrada e o fator de calibração do consumo moram em
`veiculo.json`, e o estado vivo do tanque em `tanque.json`, os dois no diretório de
dados do app. **Não** são preferência de perfil: trocar de motorista não muda o
tanque do carro. Os padrões são os do Eclipse GT 2000 (61 L, 3.0 V6), e tudo se
ajusta com o dedo no rodapé da tela do carro — não há tela de Ajustes, e não há
teclado do sistema em lugar nenhum.

Consumo não é um dado do barramento: sai do fluxo de ar do motor, por uma cascata de
fontes que depende do que o carro responde (vazão `015E` → MAF `0110` →
coletor `010B`+`010F` → carga `0104`). Todo número derivado dessa conta chega na tela
com `~` na frente. **A primeira ignição com o adaptador é o que decide qual fonte
vale**: o log traz as capacidades do carro (`adb logcat -s EclipseObdBt`), e o fator de
calibração se afere comparando um tanque inteiro com a bomba.

No desktop o carro é simulado, com a mesma lentidão do barramento. `ECLIPSE_SIM_SEM=maf,nivel`
faz o carro de mentira esconder PIDs — é assim que se vê no Mac como o painel se
comporta quando o Eclipse não entrega o sensor de massa de ar ou o nível do tanque.

## Como está organizado

```
crates/
  eclipse-core       contrato de módulo, barramento, supervisor, perfis
  eclipse-sim        o carro imaginário, lido pelo OBD e pelo GPS
  eclipse-obd        varredura dos PIDs, consumo, tanque e autonomia
  eclipse-gps        posição, rumo e o traçado percorrido
  eclipse-music      cofre de tokens e a ponte com o Spotify
  eclipse-messaging  caixa de entrada e a fonte de mensagens
src-tauri            fiação: eventos, comandos, registro dos módulos
src                  React: grid, widgets, perfis
```

O Rust é dono de todo o estado; a tela é uma projeção. Nenhuma regra de negócio
no React.

**Módulo e tile são coisas diferentes.** Todos os mostradores do carro leem do módulo
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
  10.400 baud, não CAN. O simulador respeita essa lentidão de propósito, e é por isso
  que ler a fonte de ar do consumo custa velocidade e RPM: cada PID a mais no ciclo
  atrasa todos os outros (~1,2 s nos rápidos em vez de ~0,9 s).
- **Consumo é conta, não leitura.** Nenhum carro de 2000 informa km/l. Sai de massa de
  ar ÷ proporção ar/combustível, e erra por baixo em aceleração forte (a injeção
  enriquece a mistura e nenhum PID garantido conta isso). O fator de calibração é
  quem paga essa conta.
- **A câmera de ré não passa pelo app.** O chaveamento é feito em hardware pelo
  MCU da head unit, antes do Android.

## O que falta

- Plugin Kotlin com `NotificationListenerService` — hoje as mensagens vêm de um
  mock. Depende da head unit comprada.
- GPS real via `LocationManager` — hoje a posição vem de um traçado simulado
  pela Av. Paulista.
- Aferir a telemetria no carro: qual fonte de consumo o Eclipse permite, e o fator de
  calibração de um tanque inteiro contra a bomba.
- Câmera lateral via USB/UVC.
- APK, launcher, viagens, rádio.
