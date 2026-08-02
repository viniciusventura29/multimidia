import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { startModuleBus } from "./core/moduleStore";

// O barramento de estado liga antes do React montar e vive a vida do WebView.
startModuleBus();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
