import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ligarDiario } from "./core/diario";
import { startModuleBus } from "./core/moduleStore";

// O diário primeiro: erro que aconteça montando o React tem que ser gravado
// também, e é justamente esse que hoje vira uma tela branca sem explicação.
ligarDiario();

// O barramento de estado liga antes do React montar e vive a vida do WebView.
startModuleBus();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
