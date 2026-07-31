import { ANDANDO_KMH } from "../../core/telemetria";
import type { ObdReadings } from "../../core/types";
import { Segurar } from "../../shell/Segurar";
import { acaoObd } from "./acoes";
import { Passo } from "./passo";

/** Com o carro andando, exige mais dedo parado para confirmar. */
const HOLD_PARADO_MS = 800;
const HOLD_ANDANDO_MS = 1500;

/**
 * O rodapé de ajustes da tela do carro.
 *
 * Fica embaixo, mais escuro e com tipografia menor que a faixa de leitura. Sem uma
 * tela de Ajustes separada, é essa separação vertical que impede o motorista de
 * cutucar um stepper enquanto procura a autonomia.
 *
 * **Não trava com o carro em movimento**, e isso é decisão, não esquecimento:
 * `speedKmh` pode vir `null` — e é justamente no carro que não responde o PID que os
 * ajustes mais importam — e o tanque se enche com o motor desligado, quando o ELM327
 * está dormindo. O que muda em movimento é o tempo de segurar, e um aviso.
 */
export function Ajustes({ data }: { data: ObdReadings | null }) {
  const t = data?.tanque ?? null;
  const andando = (data?.speedKmh ?? 0) > ANDANDO_KMH;
  const hold = andando ? HOLD_ANDANDO_MS : HOLD_PARADO_MS;

  const capacidade = t?.capacidadeL ?? 61;
  const falta = t?.faltaParaEncherL ?? 0;
  const calibracao = Math.round((t?.calibracao ?? 1) * 100);

  return (
    // O overlay fecha ao clicar fora do painel; o painel já para a propagação, mas
    // parar aqui também deixa explícito que o rodapé nunca fecha a tela.
    <footer className="carro__rodape" onClick={(e) => e.stopPropagation()}>
      {andando && (
        <p className="carro__aviso">carro andando — melhor ajustar parado</p>
      )}

      <div className="carro__grupo">
        {/* `key` re-semeia o rascunho quando o Rust confirma um valor novo: é o
            idioma React para "esqueça o que eu digitei, o servidor falou". */}
        <Passo
          key={`tanque-${capacidade}`}
          rotulo="Tamanho do tanque"
          inicial={capacidade}
          unit="L"
          passos={[1, 5]}
          min={20}
          max={120}
          aoConfirmar={(capacidadeL) => acaoObd({ acao: "tanque", capacidadeL })}
        />
      </div>

      <div className="carro__grupo">
        <Passo
          key={`abastecer-${falta}`}
          rotulo="Abastecer"
          nota={`cabem ${falta.toFixed(1)}L`}
          inicial={falta}
          unit="L"
          decimais={1}
          passos={[0.5, 5]}
          min={0}
          max={capacidade}
          render={(litros, intocado) => (
            <Segurar
              // Sem mexer no stepper, o gesto natural é "enchi": o rascunho já vem
              // com o que falta para encher, então os dois caminhos concordam.
              rotulo={intocado ? "enchi o tanque" : `abasteci ${litros.toFixed(1)} L`}
              ms={hold}
              aoConfirmar={() =>
                acaoObd(intocado ? { acao: "enchi" } : { acao: "abasteci", litros })
              }
            />
          )}
        />
      </div>

      <div className="carro__grupo">
        {/* Em porcentagem porque é como se raciocina — "está 5% otimista". O fio
            carrega o fator; a conversão é formatação, não regra. */}
        <Passo
          key={`calibracao-${calibracao}`}
          rotulo="Calibração do consumo"
          inicial={calibracao}
          unit="%"
          passos={[1, 5]}
          min={80}
          max={120}
          aoConfirmar={(pct) => acaoObd({ acao: "calibrar", fator: pct / 100 })}
        />
      </div>

      <div className="carro__grupo carro__grupo--fim">
        <Segurar
          rotulo="zerar viagem"
          ms={hold}
          aoConfirmar={() => acaoObd({ acao: "zerarViagem" })}
        />
      </div>
    </footer>
  );
}
