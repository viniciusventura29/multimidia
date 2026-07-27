import { useState, type FormEvent } from "react";

import type { Perfis } from "../core/useProfiles";
import type { Profile } from "../core/types";

/** Poucas cores em vez de um seletor livre: é mais rápido no toque, dentro do carro. */
const CORES = ["#3ddc97", "#4da3ff", "#f5a524", "#e5484d", "#a06bff", "#e8edf5"];

function CartaoPerfil({
  profile,
  ativo,
  aoEscolher,
  aoRemover,
}: {
  profile: Profile;
  ativo: boolean;
  aoEscolher: () => void;
  aoRemover?: () => void;
}) {
  const [confirmando, setConfirmando] = useState(false);

  return (
    <div className={`perfil${ativo ? " perfil--ativo" : ""}`}>
      <button className="perfil__escolher" onClick={aoEscolher}>
        <span
          className="perfil__ponto"
          style={{ background: profile.color }}
          aria-hidden
        />
        <span className="perfil__nome">{profile.name}</span>
      </button>

      {aoRemover &&
        (confirmando ? (
          <button
            className="perfil__remover perfil__remover--confirma"
            onClick={aoRemover}
          >
            confirmar
          </button>
        ) : (
          // Dois toques de propósito: apagar perfil sem querer, com o carro
          // andando, seria irrecuperável.
          <button
            className="perfil__remover"
            onClick={() => setConfirmando(true)}
            aria-label={`Remover ${profile.name}`}
          >
            remover
          </button>
        ))}
    </div>
  );
}

interface Props extends Perfis {
  titulo: string;
  /** Sem isto, a tela é bloqueante: não há para onde voltar sem escolher. */
  aoFechar?: () => void;
}

export function ProfilePicker({
  profiles,
  active,
  titulo,
  criar,
  selecionar,
  remover,
  aoFechar,
}: Props) {
  const [nome, setNome] = useState("");
  const [cor, setCor] = useState(CORES[0]);

  const enviar = async (event: FormEvent) => {
    event.preventDefault();
    const limpo = nome.trim();
    if (!limpo) return;

    await criar(limpo, cor);
    setNome("");
    aoFechar?.();
  };

  const escolher = async (id: string) => {
    await selecionar(id);
    aoFechar?.();
  };

  return (
    <div className="perfis">
      <div className="perfis__painel">
        <header className="perfis__head">
          <h1 className="perfis__titulo">{titulo}</h1>
          {aoFechar && (
            <button className="overlay__fechar" onClick={aoFechar}>
              fechar
            </button>
          )}
        </header>

        <div className="perfis__lista">
          {profiles.map((profile) => (
            <CartaoPerfil
              key={profile.id}
              profile={profile}
              ativo={profile.id === active?.id}
              aoEscolher={() => void escolher(profile.id)}
              aoRemover={
                profiles.length > 1 ? () => void remover(profile.id) : undefined
              }
            />
          ))}
        </div>

        <form className="novo" onSubmit={enviar}>
          <input
            className="novo__nome"
            value={nome}
            onChange={(e) => setNome(e.target.value)}
            placeholder="novo perfil"
            maxLength={24}
          />

          <div className="novo__cores">
            {CORES.map((opcao) => (
              <button
                key={opcao}
                type="button"
                className={`novo__cor${opcao === cor ? " novo__cor--ativa" : ""}`}
                style={{ background: opcao }}
                onClick={() => setCor(opcao)}
                aria-label={`Cor ${opcao}`}
              />
            ))}
          </div>

          <button className="novo__criar" type="submit" disabled={!nome.trim()}>
            criar
          </button>
        </form>
      </div>
    </div>
  );
}

export function ProfileChip({
  profile,
  aoClicar,
}: {
  profile: Profile;
  aoClicar: () => void;
}) {
  return (
    <button className="chip" onClick={aoClicar}>
      <span
        className="chip__ponto"
        style={{ background: profile.color }}
        aria-hidden
      />
      {profile.name}
    </button>
  );
}
