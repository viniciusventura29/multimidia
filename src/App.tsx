import { useState } from "react";

import { useProfiles, useTema } from "./core/useProfiles";
import { ProfileChip, ProfilePicker } from "./profiles/ProfilePicker";
import { Dashboard } from "./shell/Dashboard";
import "./App.css";

export default function App() {
  const perfis = useProfiles();
  const [trocando, setTrocando] = useState(false);

  useTema(perfis.active);

  if (perfis.carregando) {
    return <div className="boot">Eclipse OS</div>;
  }

  // Sem perfil ativo não há painel: é preciso saber de quem são as contas antes
  // de mostrar qualquer coisa.
  if (!perfis.active) {
    return <ProfilePicker {...perfis} titulo="quem está dirigindo?" />;
  }

  return (
    <>
      <Dashboard />

      <ProfileChip profile={perfis.active} aoClicar={() => setTrocando(true)} />

      {trocando && (
        <ProfilePicker
          {...perfis}
          titulo="trocar de perfil"
          aoFechar={() => setTrocando(false)}
        />
      )}
    </>
  );
}
