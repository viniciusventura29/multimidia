/**
 * Geometria de mapa, do lado da tela.
 *
 * O raciocínio pesado sobre posição mora no Rust (`eclipse-gps`), que é onde a
 * rota e o GPS se encontram e onde dá para testar. Aqui ficam só as duas contas
 * que a própria tela precisa fazer: para animar o carro entre duas leituras, e
 * para descobrir o rumo quando o navegador não relata nenhum.
 */

/** Raio da Terra, em metros. */
const RAIO_TERRA_M = 6_371_000;

const RAD = Math.PI / 180;

/**
 * Distância aproximada em metros entre dois pontos.
 *
 * Equirretangular, não haversine: a precisão sobra para distinguir jitter de
 * GPS de movimento real, que é para o que ela serve nos dois usos daqui.
 */
export function metros(
  a: { lat: number; lon: number },
  b: { lat: number; lon: number },
): number {
  const dLat = (b.lat - a.lat) * RAD;
  const dLon = (b.lon - a.lon) * RAD * Math.cos(((a.lat + b.lat) / 2) * RAD);
  return RAIO_TERRA_M * Math.hypot(dLat, dLon);
}

/**
 * Rumo de `a` para `b`, em graus — 0 = norte, crescendo no sentido horário.
 *
 * É a mesma conta do `rumo()` do `eclipse-gps`; existe em dobro porque este
 * lado precisa dela antes de a posição chegar ao Rust, para preencher o rumo
 * que o navegador não soube informar.
 */
export function rumoEntre(
  a: { lat: number; lon: number },
  b: { lat: number; lon: number },
): number {
  const lat1 = a.lat * RAD;
  const lat2 = b.lat * RAD;
  const dLon = (b.lon - a.lon) * RAD;

  const y = Math.sin(dLon) * Math.cos(lat2);
  const x =
    Math.cos(lat1) * Math.sin(lat2) -
    Math.sin(lat1) * Math.cos(lat2) * Math.cos(dLon);

  return ((Math.atan2(y, x) / RAD) % 360 + 360) % 360;
}
