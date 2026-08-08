import {
  Cloud,
  CloudFog,
  CloudLightning,
  CloudRain,
  CloudSnow,
  Moon,
  Sun,
} from "lucide-react";

import type { FamiliaClima } from "../modules/nav/tipos";

/**
 * O ícone do tempo.
 *
 * O trabalho pesado — vinte e oito códigos WMO virando seis famílias e uma
 * frase em português — mora no `eclipse-clima`, do lado do Rust. Aqui só se
 * escolhe o desenho, que é a única parte que precisa saber que existe um
 * lucide.
 *
 * A noite só troca o ícone de céu limpo: nublado é nublado às três da tarde e
 * às três da manhã, mas "sol" à meia-noite seria absurdo na cara do motorista.
 */
export function ClimaIcon({
  familia,
  noite,
  className,
}: {
  familia: FamiliaClima;
  noite: boolean;
  className?: string;
}) {
  const props = { size: "1em" as const, className, "aria-hidden": true };

  switch (familia) {
    case "limpo":
      return noite ? <Moon {...props} /> : <Sun {...props} />;
    case "nevoa":
      return <CloudFog {...props} />;
    case "chuva":
      return <CloudRain {...props} />;
    case "neve":
      return <CloudSnow {...props} />;
    case "tempestade":
      return <CloudLightning {...props} />;
    case "nuvem":
      return <Cloud {...props} />;
  }
}
