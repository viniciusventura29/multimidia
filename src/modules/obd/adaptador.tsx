import { Bluetooth, Check, ChevronLeft, Loader2 } from "lucide-react";

import { dispatchAction } from "../../core/actions";
import { useModuleEnvelope } from "../../core/moduleStore";
import type {
  AchadoBt,
  AcaoAdaptador,
  EstadoAdaptador,
  StateEnvelope,
} from "../../core/types";

const acaoAdaptador = (acao: AcaoAdaptador): void => dispatchAction("adaptador", acao);

/** O sinal em três degraus. Número de RSSI não diz nada a ninguém. */
function sinal(rssi: number | null): string {
  if (rssi === null) return "";
  if (rssi >= -60) return "perto";
  if (rssi >= -80) return "longe";
  return "muito longe";
}

/**
 * A linha de um aparelho.
 *
 * Sem placa, sem cápsula, sem moldura: o vão entre as linhas já divide. O que
 * destaca o candidato é o peso do nome, não uma borda em volta dele.
 */
function Linha({
  achado,
  escolhido,
  pareando,
  aoEscolher,
}: {
  achado: AchadoBt;
  escolhido: boolean;
  pareando: boolean;
  aoEscolher: () => void;
}) {
  const marcas = [
    achado.tipo === "ble" ? "BLE" : null,
    achado.pareado ? "pareado" : null,
    sinal(achado.rssi) || null,
  ].filter(Boolean);

  return (
    <button
      type="button"
      className={`bt__linha${achado.pareceObd ? " bt__linha--candidato" : ""}`}
      onClick={aoEscolher}
      disabled={pareando}
    >
      <span className="bt__nome">{achado.nome || "(sem nome)"}</span>
      <span className="bt__meta">
        {achado.mac}
        {marcas.length > 0 && ` · ${marcas.join(" · ")}`}
      </span>
      {escolhido && <Check size="1.1em" className="bt__marca" aria-label="em uso" />}
      {pareando && <Loader2 size="1.1em" className="bt__girando" aria-label="pareando" />}
    </button>
  );
}

/**
 * Escolher o adaptador OBD do carro.
 *
 * Existe porque a tela de Bluetooth da central não dá conta: ela lista o
 * adaptador e não o pareia, e um adaptador BLE nem aparece nela. Daqui o app fala
 * com o rádio direto — busca dos dois jeitos, pareia respondendo o PIN de fábrica
 * e grava o MAC. Depois desta vez, o carro conecta sozinho a cada ignição.
 */
export function Adaptador({ aoVoltar }: { aoVoltar: () => void }) {
  const envelope = useModuleEnvelope("adaptador") as
    | StateEnvelope<EstadoAdaptador>
    | undefined;
  const estado = envelope?.data ?? null;

  const fase = estado?.fase ?? { fase: "ocioso" as const };
  const pareandoMac = fase.fase === "pareando" ? fase.mac : null;
  const salvo = estado?.salvo ?? null;
  const radio = estado?.radio ?? { existe: true, ligado: true, motivo: null };

  // O aviso do rádio vem antes de qualquer lista: sem ele, uma tela vazia parece
  // "não achou nada" quando na verdade o Bluetooth está desligado.
  const aviso = !radio.existe
    ? (radio.motivo ?? "esta central não tem Bluetooth")
    : !radio.ligado
      ? "o Bluetooth está desligado"
      : fase.fase === "falhou"
        ? fase.motivo
        : null;

  return (
    <div className="bt" onClick={(e) => e.stopPropagation()}>
      <header className="bt__topo">
        <button type="button" className="bt__voltar" onClick={aoVoltar}>
          <ChevronLeft size="1.1em" /> carro
        </button>
        <h2 className="bt__titulo">Adaptador OBD</h2>
        <button
          type="button"
          className="bt__buscar"
          onClick={() =>
            acaoAdaptador(estado?.buscando ? { acao: "parar" } : { acao: "buscar" })
          }
          disabled={!radio.ligado}
        >
          {estado?.buscando ? (
            <>
              <Loader2 size="1em" className="bt__girando" /> buscando
            </>
          ) : (
            <>
              <Bluetooth size="1em" /> buscar
            </>
          )}
        </button>
      </header>

      {aviso && <p className="bt__aviso">{aviso}</p>}

      {salvo ? (
        <p className="bt__salvo">
          usando <strong>{salvo.nome || salvo.mac}</strong>
          {/* Sem `Segurar`: esquecer não apaga número em que o motorista confia —
              apaga uma escolha que se refaz em dois toques, e sem ela o painel
              volta a adivinhar pelo nome em vez de ficar cego. */}
          <button
            type="button"
            className="bt__esquecer"
            onClick={() => acaoAdaptador({ acao: "esquecer" })}
          >
            esquecer
          </button>
        </p>
      ) : (
        <p className="bt__salvo bt__salvo--vazio">
          nenhum adaptador escolhido — o painel está adivinhando pelo nome
        </p>
      )}

      <div className="bt__lista">
        {(estado?.encontrados ?? []).map((a) => (
          <Linha
            key={a.mac}
            achado={a}
            escolhido={salvo?.mac === a.mac}
            pareando={pareandoMac === a.mac}
            aoEscolher={() => acaoAdaptador({ acao: "escolher", mac: a.mac })}
          />
        ))}

        {(estado?.encontrados.length ?? 0) === 0 && (
          <p className="bt__vazio">
            {estado?.buscando
              ? "procurando…"
              : "toque em buscar com o adaptador plugado na tomada OBD"}
          </p>
        )}
      </div>

      {/* A dica que evita a ligação para o suporte que não existe. */}
      <p className="bt__nota">
        O adaptador só responde com a chave na ignição. Se ele não aparecer aqui nem
        depois de meio minuto, é sinal de que não está energizado.
      </p>
    </div>
  );
}
