import { useState } from "react";

import { useModuleStates } from "./core/useModuleStates";
import { useProfiles, useTema } from "./core/useProfiles";
import { useLocalizacaoReal } from "./modules/nav";
import { ProfilePicker } from "./profiles/ProfilePicker";
import { BemVindo } from "./shell/BemVindo";
import { Dashboard } from "./shell/Dashboard";
import { Header } from "./shell/Header";
import "./App.css";

export default function App() {
  const perfis = useProfiles();
  const states = useModuleStates();
  const [trocando, setTrocando] = useState(false);

  useTema(perfis.active);
  // Mora aqui, e não dentro do tile do mapa, porque o tile monta duas vezes
  // (grid + tela expandida) — abrir dois `watchPosition` ao mesmo tempo seria
  // desperdício. Aqui só existe uma instância do App.
  useLocalizacaoReal();

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
      <div className="app">
        <Header
          states={states}
          profile={perfis.active}
          aoTrocar={() => setTrocando(true)}
        />
        <Dashboard states={states} />
      </div>

      <BemVindo key={perfis.active.id} profile={perfis.active} />

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
