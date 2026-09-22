import { useCallback, useEffect, useState } from "react";
// TEMPORÁRIO — `invoke` cronometrado; ver `core/perf.ts`.
import { invoke } from "../core/perf";

import { anotar } from "../core/diario";

/**
 * Quanto esperar depois do boot antes da primeira pergunta.
 *
 * Não é folga de estilo. Na ignição a head unit está pareando o ELM327,
 * fixando GPS, subindo o Web Playback SDK e compilando os shaders do carro 3D —
 * e, o que mais pesa, **a rede em geral ainda não existe**: o tethering do
 * celular associa alguns segundos depois. Perguntar aos 0 s é quase garantir
 * uma falha, e a falha custa quinze minutos de espera até a próxima tentativa.
 */
const ATRASO_INICIAL_MS = 60_000;

/**
 * Entre duas perguntas que deram certo.
 *
 * Uma viagem típica é de 20-40 min, então na prática é uma pergunta por
 * ignição. As seis horas são para o carro que fica ligado a tarde inteira: ele
 * fica sabendo no mesmo dia sem martelar o GitHub.
 */
const DESCANSO_MS = 6 * 60 * 60 * 1000;

/**
 * Depois de uma pergunta que não deu.
 *
 * Cadência própria, e não o mesmo descanso: "sem rede na ignição, com rede três
 * minutos depois" é o caso NORMAL do carro, não a exceção. Com um intervalo só
 * eu teria que escolher entre martelar o GitHub o dia inteiro ou perder a
 * janela em que a rede finalmente subiu.
 */
const RETENTAR_MS = 15 * 60 * 1000;

/** O que o Rust devolve. A URL fica lá — ver `baixar`. */
export interface Atualizacao {
  versionCode: number;
}

/**
 * "Tem versão nova?", e o toque que leva ao download.
 *
 * SILÊNCIO ABSOLUTO NA TELA QUANDO FALHA. O `catch` não mexe no estado — nem
 * para limpar. Carro sem rede não vê nada: nem pílula, nem chip apagando, nem
 * `console.error` (que num aparelho sem SIM pareceria defeito onde não há).
 * Não limpar também evita o chip piscar: quem já sabe que existe a 43 continua
 * sabendo depois de entrar num túnel.
 *
 * Silêncio na tela, não no diário. Uma checagem que falha PARA SEMPRE — o
 * manifesto mudou de formato, o release sumiu, a URL quebrou — é indistinguível
 * de "não há versão nova": o carro simplesmente nunca avisa, e o dono fica para
 * trás sem nada aparecer errado. O motorista não precisa saber disso; eu
 * preciso. Por isso o erro vai para o diário de bordo, que é lido daqui.
 *
 * O `StrictMode` monta duas vezes em desenvolvimento, mas o primeiro cleanup
 * derruba o timer bem antes dos 60 s — nenhuma pergunta duplicada sai daqui.
 */
export function useAtualizacao(): {
  nova: Atualizacao | null;
  baixar: () => void;
} {
  const [nova, setNova] = useState<Atualizacao | null>(null);

  useEffect(() => {
    let vivo = true;
    let timer: ReturnType<typeof setTimeout>;

    const perguntar = async () => {
      // O padrão é o intervalo curto: só uma resposta de verdade compra as seis
      // horas de descanso.
      let proximo = RETENTAR_MS;
      try {
        const resposta = await invoke<Atualizacao | null>("checar_atualizacao");
        if (!vivo) return;
        setNova(resposta);
        proximo = DESCANSO_MS;
      } catch (err) {
        console.debug("[eclipse] não deu para checar atualização", err);
        // `aviso` e não `erro`: sem rede é o caso NORMAL do carro, e um erro a
        // cada ignição transformaria o diário em ruído. Como aviso ele sobe do
        // mesmo jeito, e o que interessa é o padrão — falhar uma vez é a
        // garagem sem Wi-Fi; falhar sempre é a checagem quebrada.
        anotar("aviso", "atualizacao", "não deu para checar se há versão nova", {
          motivo: String(err).slice(0, 300),
        });
      }
      if (vivo) timer = setTimeout(() => void perguntar(), proximo);
    };

    timer = setTimeout(() => void perguntar(), ATRASO_INICIAL_MS);
    return () => {
      vivo = false;
      clearTimeout(timer);
    };
  }, []);

  // Quem abre o navegador é o Rust, com a URL que ele mesmo peneirou — a tela
  // pede o efeito de um toque, não o inventa (mesmo princípio do
  // `dispatch_action`).
  const baixar = useCallback(() => {
    void invoke("baixar_atualizacao").catch((err) => {
      console.debug("[eclipse] não deu para abrir o navegador", err);
      // Este é pior que o de cima: o dono TOCOU no aviso, quis atualizar, e
      // nada aconteceu. Na tela continua não havendo o que dizer, mas isto não
      // pode passar despercebido por aqui.
      anotar("erro", "atualizacao", "o toque em atualizar não abriu o navegador", {
        motivo: String(err).slice(0, 300),
      });
    });
  }, []);

  return { nova, baixar };
}

/**
 * A versão que está rodando, para a barra mostrar.
 *
 * Pergunta uma vez e pronto: o número só muda reinstalando o APK, e reinstalar
 * reinicia o WebView. Não há o que revalidar.
 *
 * Vem do Rust em vez de um `define` do Vite de propósito. O `versionCode` é
 * cravado pela CI no build do Android, DEPOIS do bundle do front; um valor
 * assado no JavaScript seria o do momento do `vite build`, que não é o mesmo
 * número — e um número errado aqui é pior que nenhum, porque é exatamente o
 * número que se olha para confiar que atualizou.
 */
export function useVersao(): string | null {
  const [versao, setVersao] = useState<string | null>(null);

  useEffect(() => {
    let vivo = true;
    void invoke<string>("versao_rodando")
      .then((v) => {
        if (vivo) setVersao(v);
      })
      .catch((err) => {
        // Sem barulho na tela: o rodapé simplesmente não aparece. Mas vai para
        // o diário, porque um carro que não sabe dizer sua versão é justamente
        // o que quebra o diagnóstico que este número existe para dar.
        anotar("aviso", "atualizacao", "não deu para saber a versão que está rodando", {
          motivo: String(err).slice(0, 300),
        });
      });
    return () => {
      vivo = false;
    };
  }, []);

  return versao;
}
