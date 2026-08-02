/** Espelho do `StateEnvelope` do `eclipse-core`. */

export type Status = "loading" | "ready" | "degraded";

export interface StateEnvelope<T = unknown> {
  module: string;
  /** Contador global e monotônico, usado para descartar eventos fora de ordem. */
  seq: number;
  status: Status;
  /** O último valor bom. Sobrevive à degradação — por isso não é limpo aqui. */
  data: T | null;
  /** Só vem preenchido quando `status` é `"degraded"`. */
  reason: string | null;
}

/* ------------------------------------------------------------------ */
/* Payloads dos módulos                                                */
/* ------------------------------------------------------------------ */

/**
 * Leituras do OBD.
 *
 * Todo campo é anulável porque o barramento do Eclipse é lento: os PIDs lentos
 * só chegam a cada ~2,7 s, então nos primeiros segundos a temperatura está
 * genuinamente vazia enquanto o RPM já anda. Não é erro, é o carro.
 */
export interface ObdReadings {
  rpm: number | null;
  speedKmh: number | null;
  coolantC: number | null;
  fuelPct: number | null;
  voltage: number | null;
  /** Massa de ar admitida, em g/s. A entrada da conta de consumo. */
  mafGs: number | null;
  cargaPct: number | null;
  mapKpa: number | null;
  iatC: number | null;
  /** Vazão que o próprio carro calcula. Raro antes de 2010. */
  vazaoLh: number | null;
  consumo: Consumo;
  tanque: Tanque;
  viagem: Viagem;
  capacidades: Capacidades;
  /** Como o adaptador descreve o barramento negociado. Só para o diagnóstico. */
  protocolo: string | null;
}

/**
 * Consumo, calculado no Rust.
 *
 * `medido` é `false` quando a vazão foi modelada a partir de carga ou pressão do
 * coletor em vez de medida por um sensor de massa de ar. A tela marca esses números
 * com `~`: dizer "11,4 km/l" com a mesma cara nos dois casos seria mentir sobre a
 * precisão.
 */
export interface Consumo {
  /** `null` com o carro parado: km/l não existe a 0 km/h — aí só existe `litrosHora`. */
  instantaneoKmL: number | null;
  litrosHora: number | null;
  metodo: "direto" | "maf" | "coletor" | "carga" | "indisponivel";
  medido: boolean;
}

export interface Tanque {
  capacidadeL: number;
  litros: number | null;
  /** Derivada dos litros estimados — combina com a barra, ao contrário de `fuelPct`. */
  pct: number | null;
  /** Quanto cabe até encher. Vem pronto para a tela não fazer conta. */
  faltaParaEncherL: number | null;
  autonomiaKm: number | null;
  mediaTanqueKmL: number | null;
  calibracao: number;
  /** O nível vem do PID do carro **e** a estimativa já convergiu com ele. */
  medido: boolean;
}

export interface Viagem {
  distanciaKm: number;
  duracaoS: number;
  litros: number;
  mediaKmL: number | null;
}

/** O que este carro respondeu na máscara de PIDs suportados. */
export interface Capacidades {
  /** Em hex de dois dígitos: `["04","05","0C","0D"]`. */
  pids: string[];
  descoberto: boolean;
}

/**
 * O que a tela pode pedir ao módulo `obd`. Espelho do enum de ações do Rust.
 *
 * Tipado porque são cinco ações que mudam número em que o motorista confia: errar o
 * nome de uma chave falharia em silêncio do outro lado.
 */
export type AcaoObd =
  | { acao: "enchi" }
  | { acao: "abasteci"; litros: number }
  | { acao: "nivel"; litros: number }
  | { acao: "tanque"; capacidadeL: number }
  | { acao: "calibrar"; fator: number }
  | { acao: "zerarViagem" };

export interface NowPlaying {
  track: string;
  artist: string;
  isPlaying: boolean;
  albumArt: string | null;
  progressMs: number | null;
  durationMs: number | null;
}

/** Uma faixa achada na busca. `uri` é o que se manda para tocar. */
export interface Faixa {
  uri: string;
  track: string;
  artist: string;
  albumArt: string | null;
}

export interface Playlist {
  uri: string;
  nome: string;
  albumArt: string | null;
}

export interface Album {
  uri: string;
  nome: string;
  artist: string;
  albumArt: string | null;
}

export interface Busca {
  faixas: Faixa[];
  albuns: Album[];
}

/** Uma playlist ou álbum aberto, com as faixas de dentro. */
export interface Contexto {
  uri: string;
  nome: string;
  subtitulo: string;
  albumArt: string | null;
  faixas: Faixa[];
}

/**
 * O que está errado, tipado pelo Rust. Antes a tela adivinhava por regex no
 * texto do erro e dois casos não casavam — o painel dizia "sem sinal" e não
 * oferecia saída nenhuma.
 */
export type TipoProblema =
  | "precisaLogin"
  | "precisaPremium"
  | "semDispositivo"
  | "rede";

export interface Problema {
  tipo: TipoProblema;
  detalhe: string;
}

/** O estado do módulo de música. */
export interface MusicState {
  nowPlaying: NowPlaying | null;
  busca: Busca;
  playlists: Playlist[];
  contexto: Contexto | null;
  problema: Problema | null;
}

/* ------------------------------------------------------------------ */
/* Perfis                                                              */
/* ------------------------------------------------------------------ */

export type Units = "metric" | "imperial";

export interface Preferences {
  units: Units;
}

export interface Profile {
  id: string;
  name: string;
  /** Cor de destaque, em hex. Vira a `--accent` do tema inteiro. */
  color: string;
  preferences: Preferences;
}

/* ------------------------------------------------------------------ */
/* Contrato de tile                                                    */
/* ------------------------------------------------------------------ */

/**
 * O que um tile recebe: o estado do módulo do qual ele lê — e SÓ dele.
 *
 * Cada tile lê apenas do próprio módulo, e é essa separação que faz o OBD cair
 * sem levar música e navegação junto — e que faz um tick de RPM re-renderizar
 * só os mostradores. Quem precisa espiar o módulo do vizinho (o carrinho do
 * assistente, que reage à telemetria de verdade) assina direto no
 * `moduleStore` com um seletor, sem desfazer o isolamento dos demais.
 */
export interface TileView<T> {
  data: T | null;
  status: Status;
  reason: string | null;
}

/**
 * Um quadro na tela.
 *
 * Tile não é o mesmo que módulo: os cinco mostradores do painel leem todos do
 * módulo `obd`, porque é uma conexão só com o carro. Quando o adaptador cai, os
 * cinco escurecem juntos — e o Spotify nem fica sabendo.
 */
export interface TileSpec<T> {
  id: string;
  /** De qual módulo este tile lê. */
  module: string;
  title: string;
  /** Nome da área no `grid-template-areas`. */
  area: string;
  /** Ícone no cabeçalho do tile — dá um marcador visual de relance. */
  icon?: React.ReactNode;
  /**
   * Tile que não lê de módulo nenhum (relógio, por ex.): nasce pronto em vez de
   * "carregando", já que nunca vai chegar um evento para ele.
   */
  estatico?: boolean;
  Compact: React.ComponentType<TileView<T>>;
  Expanded?: React.ComponentType<TileView<T>>;
}

// A lista de tiles é heterogênea, então o registro apaga o tipo do payload.
// `defineTile` mantém a checagem dentro de cada tile.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type AnyTileSpec = TileSpec<any>;

export function defineTile<T>(spec: TileSpec<T>): AnyTileSpec {
  return spec;
}
