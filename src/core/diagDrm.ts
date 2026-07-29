/**
 * Diagnóstico: a WebView tem DRM (Widevine/EME)?
 *
 * O Web Playback SDK do Spotify — o único caminho para o Eclipse tocar o áudio
 * ele mesmo, sem o app oficial do Spotify no aparelho — exige `encrypted-media`.
 * A doc do Spotify garante suporte a navegadores mobile, mas o Eclipse roda numa
 * **WebView**, não no Chrome, e é isso que precisa ser provado no aparelho.
 *
 * Se `requestMediaKeySystemAccess` falhar aqui, o caminho está fechado e não
 * vale escrever mais nada em cima dele — por isso este teste vem primeiro.
 */
export async function diagnosticarDrm(): Promise<void> {
  const log = (msg: string) => console.log(`[eclipse-drm] ${msg}`);

  if (!("requestMediaKeySystemAccess" in navigator)) {
    log("EME AUSENTE: navigator.requestMediaKeySystemAccess não existe");
    return;
  }
  log("EME presente (requestMediaKeySystemAccess existe)");

  // Widevine é o DRM que o Spotify usa no Android/Chrome.
  const config: MediaKeySystemConfiguration[] = [
    {
      initDataTypes: ["cenc"],
      audioCapabilities: [{ contentType: 'audio/mp4;codecs="mp4a.40.2"' }],
    },
  ];

  for (const sistema of ["com.widevine.alpha", "org.w3.clearkey"]) {
    try {
      const acesso = await navigator.requestMediaKeySystemAccess(sistema, config);
      log(`OK ${sistema} — keySystem=${acesso.keySystem}`);
      try {
        await acesso.createMediaKeys();
        log(`OK ${sistema} — createMediaKeys funcionou`);
      } catch (err) {
        log(`FALHOU ${sistema} — createMediaKeys: ${err}`);
      }
    } catch (err) {
      log(`FALHOU ${sistema} — ${err}`);
    }
  }
}
