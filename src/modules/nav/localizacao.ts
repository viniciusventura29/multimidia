import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

import { anotar } from "../../core/diario";
import { metros, rumoEntre } from "./geo";

/** Abaixo disto o GPS é considerado parado — o mesmo 1,4 m/s (~5 km/h) que o
 *  `FiltroDeParada` usa no Rust, para os dois lados discordarem o mínimo. */
const VELOCIDADE_MINIMA_MS = 1.4;

/** Piso do deslocamento que autoriza deduzir rumo do movimento.
 *
 *  Menos que isto pode ser jitter, e rumo tirado de jitter é uma seta girando
 *  sozinha — pior que uma seta parada apontando para o lado errado. */
const DESLOCAMENTO_MINIMO_M = 10;

/**
 * Liga a geolocalização de verdade do navegador ao módulo `nav`.
 *
 * O Rust não tem como chamar `navigator.geolocation` sozinho — só o navegador
 * fala com o sistema operacional para isso. Então a posição entra pelo caminho
 * inverso dos outros sensores: aqui ela é lida e empurrada para o Rust, que só
 * escuta (ver `PushedLocation` em `eclipse-gps`).
 *
 * No Mac o resultado é um ponto parado — o notebook não anda — mas é a
 * posição real, e é o que o Vinicius pediu no lugar do trajeto simulado.
 */
export function useLocalizacaoReal(): void {
  // Guarda o último rumo válido: `coords.heading` vem nulo quando o aparelho
  // está parado (é o caso normal aqui), e sem isso o mapa perderia a direção
  // toda vez que o GPS relatasse velocidade zero.
  const ultimoRumo = useRef(0);
  // A última posição usada para deduzir rumo. Não é a última leitura: é a
  // última que ficou longe o bastante da anterior para a dedução valer.
  const ultimoPonto = useRef<{ lat: number; lon: number } | null>(null);
  // Só para o console dizer, uma vez, que o GPS chegou a fixar. Numa head unit
  // onde o WebView pode nunca entregar posição, "fixou?" é a primeira pergunta
  // da depuração, e a resposta tem que caber num logcat sem inundá-lo.
  const jaLogouFix = useRef(false);

  useEffect(() => {
    if (!("geolocation" in navigator)) {
      void invoke("push_location_error", { permissaoNegada: false }).catch(() => {});
      return;
    }

    const id = navigator.geolocation.watchPosition(
      (posicao) => {
        const { latitude, longitude, heading, speed, accuracy } = posicao.coords;

        if (!jaLogouFix.current) {
          jaLogouFix.current = true;
          console.info("[eclipse] GPS fixou", { latitude, longitude, accuracy });
          // Também no diário: "fixou?" é a primeira pergunta da depuração, e o
          // console da head unit não é lido por ninguém. A coordenada é
          // arredondada na saída pelo próprio diário.
          anotar("info", "nav", "GPS fixou", { latitude, longitude, accuracy });
        }

        const aqui = { lat: latitude, lon: longitude };

        // Só aceita rumo novo quando há movimento de verdade. Parado, o GPS
        // devolve `heading` não-nulo porém ruidoso (gira sozinho a cada
        // leitura) — era isso que fazia o carro "sambar" no mapa. Abaixo de
        // ~5 km/h congela o último rumo bom.
        const emMovimento =
          speed !== null && !Number.isNaN(speed) && speed > VELOCIDADE_MINIMA_MS;

        if (emMovimento && heading !== null && !Number.isNaN(heading)) {
          ultimoRumo.current = heading;
          ultimoPonto.current = aqui;
        } else if (
          // Nem todo provedor sabe dizer para onde se está indo: geolocalização
          // por Wi-Fi e vários GPS de head unit entregam `heading` nulo mesmo
          // andando. Sem isto o rumo ficaria em 0 para sempre e a seta apontaria
          // para o norte a viagem inteira — que é exatamente o que acontecia.
          // Dois pontos afastados dizem a direção que o sensor não disse.
          ultimoPonto.current &&
          metros(ultimoPonto.current, aqui) > Math.max(DESLOCAMENTO_MINIMO_M, accuracy)
        ) {
          ultimoRumo.current = rumoEntre(ultimoPonto.current, aqui);
          ultimoPonto.current = aqui;
        } else {
          ultimoPonto.current ??= aqui;
        }

        void invoke("push_location", {
          lat: latitude,
          lon: longitude,
          heading: ultimoRumo.current,
          // m/s -> km/h. `speed` vem nulo em GPS por Wi-Fi parado; tratar
          // como zero é o mesmo "sem sinal de movimento" que o simulador
          // aplicava ao carro parado no semáforo.
          speedKmh: (speed ?? 0) * 3.6,
          // Quem segura o carro parado é o Rust (FiltroDeParada), e a zona
          // morta dele é dimensionada por esta incerteza — Wi-Fi espalha
          // dezenas de metros, GPS bom espalha unidades.
          accuracyM: accuracy,
        }).catch((err) => console.error("[eclipse] falha ao repassar posição", err));
      },
      (erro) => {
        // Antes este erro era engolido em silêncio, e a falha ficava invisível:
        // na central o mapa parava em São Paulo sem uma linha no console. O
        // código diz de que falha se trata — 1 = permissão negada, 2 = posição
        // indisponível (o caso provável quando a ROM não expõe o GPS ao Android),
        // 3 = timeout — e é ele que separa "sem permissão" de "o Android nem vê
        // o GPS" na hora de depurar na head unit.
        console.warn("[eclipse] geolocalização falhou", erro.code, erro.message);
        // `aviso` e não `erro`: perder o sinal num túnel é normal. O que denuncia
        // o problema de verdade é o código repetido para sempre — 1 é permissão
        // negada, 2 é a ROM não expondo o GPS ao Android, 3 é nunca fixar.
        anotar("aviso", "nav", "geolocalização falhou", {
          codigo: erro.code,
          motivo: erro.message,
          significado:
            erro.code === 1
              ? "permissão negada"
              : erro.code === 2
                ? "posição indisponível (a ROM pode não expor o GPS)"
                : "timeout: nenhum fix dentro do teto",
        });
        // Código 1 = PERMISSION_DENIED na spec do Geolocation API — comparar
        // pelo número em vez de uma constante evita depender de qual versão
        // do lib.dom.d.ts está instalada.
        void invoke("push_location_error", {
          permissaoNegada: erro.code === 1,
        }).catch(() => {});
      },
      {
        enableHighAccuracy: true,
        maximumAge: 5_000,
        // Sem `timeout`, `watchPosition` pode esperar para sempre por uma
        // posição que não vem — nem sucesso, nem erro — e era isso que deixava a
        // central parada no ponto inicial sem nenhum sinal de que algo falhou.
        // Com teto, a ausência de fix vira um erro código 3 (TIMEOUT) visível.
        // Vinte segundos é folga para um GPS frio de verdade, curto o bastante
        // para a depuração não ter que esperar um minuto por resposta.
        timeout: 20_000,
      },
    );

    return () => navigator.geolocation.clearWatch(id);
  }, []);
}
