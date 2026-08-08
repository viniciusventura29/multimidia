// Lint do frontend.
//
// O CI já cobrava `tsc` e `vite build`, mas nenhum dos dois olha para regra de
// React: um efeito com dependência a mais compila e constrói igual. Foi assim
// que o autocomplete de destino passou a disparar uma requisição cobrada por
// segundo — a posição do GPS estava na lista de dependências, e nada reclamou.
//
// Mesma disciplina do `clippy -D warnings` do lado Rust: o conjunto
// recomendado, e só. Nada de `strict` nem de estilo — aí brigaria com decisões
// deliberadas e comentadas deste código, e o gate deixaria de ser de graça.
//
// O `--max-warnings 0` do script `lint` é o `-D warnings`, e não é detalhe: o
// `exhaustive-deps` — a regra que motivou tudo isto — é **warning** no
// conjunto recomendado. Sem ele, o `eslint` sairia com zero e o CI passaria
// por cima da dependência faltando.

import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    // Só o frontend. `src-tauri/gen` é código gerado do Android, `dist` é saída
    // de build e `target` é do cargo — que despeja JS de verdade lá dentro
    // (`__global-api-script.js` do Tauri). Lintar qualquer um deles é reclamar
    // de coisa que ninguém escreveu.
    ignores: ["dist/", "src-tauri/", "target/", "node_modules/"],
  },

  js.configs.recommended,
  tseslint.configs.recommended,
  // `configs.flat.*`, e não `configs["recommended-latest"]`: o segundo ainda é
  // o formato eslintrc antigo (com `plugins` como lista de strings) e o ESLint
  // 10 recusa na cara.
  reactHooks.configs.flat["recommended-latest"],

  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parserOptions: { ecmaVersion: 2022, sourceType: "module" },
      // A UI roda num WebView: `window`, `document`, `fetch`, `crypto` e os
      // temporizadores existem. Declarados à mão em vez de puxar o pacote
      // `globals` inteiro — é a lista curta do que este painel realmente usa.
      globals: {
        cancelAnimationFrame: "readonly",
        clearInterval: "readonly",
        clearTimeout: "readonly",
        console: "readonly",
        crypto: "readonly",
        document: "readonly",
        fetch: "readonly",
        navigator: "readonly",
        performance: "readonly",
        requestAnimationFrame: "readonly",
        setInterval: "readonly",
        setTimeout: "readonly",
        window: "readonly",
        Blob: "readonly",
        SpeechSynthesisUtterance: "readonly",
        URL: "readonly",
      },
    },
    linterOptions: {
      // O ponto principal deste arquivo.
      //
      // Existiam seis `eslint-disable-next-line react-hooks/exhaustive-deps`
      // no `mapa.tsx` sem eslint nenhum no repositório: comentário decorativo,
      // que ninguém lia e ninguém podia conferir. Com isto, um `disable` que
      // deixou de ser necessário vira erro — a supressão passa a ser uma
      // decisão viva, e não um resto de uma decisão antiga.
      reportUnusedDisableDirectives: "error",
    },
    rules: {
      // Argumento não usado com `_` na frente é assinatura de interface sendo
      // cumprida, não descuido.
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],

      // As três abaixo vêm do conjunto do React Compiler, que o
      // `eslint-plugin-react-hooks` 7 passou a embutir no "recommended". Elas
      // são o `clippy::pedantic` deste lado: assumem um código que não escreve
      // laço de `requestAnimationFrame` na mão nem store externa própria — e
      // este escreve os dois, de propósito e comentado. Ligadas, seriam 11
      // supressões espalhadas; é a briga que a nota do `clippy` no ci.yml diz
      // para não comprar.
      //
      // Não é "não vale nada": as três apontam para melhorias reais, listadas
      // aqui para não virarem dívida silenciosa. Valem uma revisita se o
      // projeto adotar o React Compiler.

      // 7 ocorrências. A maior parte é o padrão de assinar uma store externa e
      // semear o valor atual na hora (`spotifyPlayer.ts`, `useProfiles.ts`) —
      // o jeito certo é `useSyncExternalStore`, que o `moduleStore.ts` já usa.
      "react-hooks/set-state-in-effect": "off",

      // 3 ocorrências, todas do padrão "ref com o valor mais recente" escrito
      // durante o render (`sugestoes.ts`, `mapa.tsx`) e de um ref lido para
      // decidir className (`music.tsx`).
      "react-hooks/refs": "off",

      // 1 ocorrência: o `performance.now()` do laço de animação do mapa. É
      // chamado de dentro do `requestAnimationFrame`, não do render — a regra
      // não tem como provar isso e marca o falso positivo.
      "react-hooks/purity": "off",
    },
  },
);
