export interface Fix {
  lat: number;
  lon: number;
  heading: number;
  speedKmh: number;
}

export interface MapaState {
  apiKey: string;
  mapId: string | null;
  fix: Fix | null;
}
