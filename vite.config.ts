import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // O worker do MapLibre é ESM e importa outro módulo. Empacotá-lo como ES
  // (e não como IIFE, que é o padrão) mantém o arquivo coerente com o
  // `type: "module"` que a própria biblioteca usa ao criar o Worker.
  worker: {
    format: "es",
  },

  build: {
    // O alvo real é o Android System WebView da head unit (Chromium). 105 é o
    // baseline que o próprio Tauri usa para WebViews Chromium — conferir a
    // versão no aparelho (chrome://version) antes de subir isto.
    target: "chrome105",
    sourcemap: false,
    rollupOptions: {
      output: {
        // React muda em ritmo próprio; num chunk separado, o cache do WebView
        // o segura entre versões do app. Por função, e não pela forma de
        // objeto, porque o react-dom/client do React 19 escapava dela.
        manualChunks(id: string) {
          if (/node_modules\/(react|react-dom|scheduler)\//.test(id)) {
            return "react";
          }
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
