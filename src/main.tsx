import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ligarDiario } from "./core/diario";
import { startModuleBus } from "./core/moduleStore";

// O diário primeiro: erro que aconteça montando o React tem que ser gravado
// também, e é justamente esse que hoje vira uma tela branca sem explicação.
ligarDiario();

// A ponte do carro (ver `dev/ponte.ts`). Só com o Vite servindo: no build de
// produção `import.meta.hot` vira `undefined` e este bloco some do bundle.
if (import.meta.hot) {
  const hot = import.meta.hot;
  void import("./dev/ponte").then((m) => m.ligarPonte(hot));
}

// O barramento de estado liga antes do React montar e vive a vida do WebView.
startModuleBus();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
