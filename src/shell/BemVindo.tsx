import { useEffect, useState } from "react";
import { Hand } from "lucide-react";

import type { Profile } from "../core/types";

/**
 * A saudação ao entrar.
 *
 * Aparece por alguns segundos e some. Não rastreia perfil anterior: o `App`
 * monta este componente com `key={active.id}`, então trocar de perfil o
 * remonta e o `useEffect` dispara de novo, com o nome novo. Um só efeito,
 * sem estado a limpar entre trocas.
 */
export function BemVindo({ profile }: { profile: Profile }) {
  const [visivel, setVisivel] = useState(true);

  useEffect(() => {
    const t = setTimeout(() => setVisivel(false), 3500);
    return () => clearTimeout(t);
  }, []);

  if (!visivel) return null;

  return (
    <div className="bemvindo" role="status">
      <Hand className="bemvindo__icone" size="1em" />
      <span className="bemvindo__texto">
        Olá, <strong>{profile.name}</strong>
      </span>
    </div>
  );
}
