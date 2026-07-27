export interface Fix {
  lat: number;
  lon: number;
  heading: number;
  speedKmh: number;
}

export interface Passo {
  instrucao: string;
  distanciaM: number;
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
  proximaInstrucao: string;
  distanciaParaManobraM: number;
  desvioM: number;
  foraDaRota: boolean;
  chegou: boolean;
}

export interface MapaState {
  apiKey: string;
  mapId: string | null;
  fix: Fix | null;
  rota: Rota | null;
  progresso: Progresso | null;
}
