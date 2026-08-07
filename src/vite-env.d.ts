/// <reference types="vite/client" />

/** `?url` do Vite: importa o caminho final do arquivo em vez do conteúdo. */
declare module "*?url" {
  const url: string;
  export default url;
}
