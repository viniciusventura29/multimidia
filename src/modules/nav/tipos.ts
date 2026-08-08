export interface Fix {
  lat: number;
  lon: number;
  heading: number;
  speedKmh: number;
  /** Raio de incerteza relatado pelo provedor, em metros. */
  accuracyM: number;
}

export interface Passo {
  instrucao: string;
  detalhe: string | null;
  distanciaM: number;
  manobra: string | null;
}

export interface Rota {
  destino: string;
  pontos: [number, number][];
  passos: Passo[];
  distanciaTotalM: number;
  duracaoTotalS: number;
}

export interface Progresso {
  distanciaRestanteM: number;
  chegadaEmS: number;
  passoAtual: number;
  proximaInstrucao: string;
  proximoDetalhe: string | null;
  proximaManobra: string | null;
  distanciaParaManobraM: number;
  desvioM: number;
  foraDaRota: boolean;
  recalcular: boolean;
  chegou: boolean;
}

/**
 * A que família o tempo pertence — é ela que escolhe o ícone.
 *
 * Quem traduz o código WMO em família é o Rust (`eclipse-clima`), para a tela
 * não precisar decorar vinte e oito números só para saber se desenha uma nuvem
 * ou um raio.
 */
export type FamiliaClima =
  | "limpo"
  | "nuvem"
  | "nevoa"
  | "chuva"
  | "neve"
  | "tempestade";

export interface Clima {
  tempC: number;
  familia: FamiliaClima;
  /** "chuva fraca", "céu limpo" — já em português, vindo do Rust. */
  rotulo: string;
  /** O código WMO cru, para depuração. */
  codigo: number;
}

export interface MapaState {
  /** A chave do Google, para sugestão de endereço e busca de postos. `null` é
   *  estado de trabalho: o mapa vem do OpenStreetMap e não pede chave. */
  apiKey: string | null;
  fix: Fix | null;
  rota: Rota | null;
  progresso: Progresso | null;
  fala: string | null;
  /** O sol já se pôs onde o carro está? Decide o tema do mapa. */
  noite: boolean;
  /** Tem rota sendo calculada agora — quem calcula é o Rust. */
  buscando: boolean;
  /** Por que a última busca de rota não deu certo, se não deu. */
  erro: string | null;
  /** Que tempo faz onde o carro está. `null` até a primeira resposta — e
   *  quando o serviço de clima falha, fica o último valor conhecido. */
  clima: Clima | null;
}
