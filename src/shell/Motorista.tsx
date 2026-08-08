import type { Profile } from "../core/types";

interface Props {
  profile: Profile;
  /** Clicar abre a troca de perfil — herda o papel do antigo chip da topbar. */
  aoTrocar: () => void;
}

/**
 * Quem está dirigindo, no alto do painel.
 *
 * Não é decoração: o perfil ativo manda no Spotify que toca, no WhatsApp que
 * chega e na cor de acento da tela inteira. Antes ele vivia como um botãozinho
 * no canto direito da barra de sistema, do tamanho de um chip de status — o que
 * é pouco para a coisa que decide de quem são todas as contas do painel.
 *
 * Aqui ele vira o cabeçalho da coluna da esquerda, com a saudação por cima do
 * nome. É o mesmo lugar onde a referência põe o nome do carro, e pela mesma
 * razão: título grande à esquerda ancora a leitura da tela.
 */
export function Motorista({ profile, aoTrocar }: Props) {
  return (
    <button className="motorista" onClick={aoTrocar}>
      <span className="motorista__saudacao">{saudacao()}</span>
      <span className="motorista__nome">
        <span
          className="motorista__ponto"
          style={{ background: profile.color }}
          aria-hidden
        />
        {profile.name}
      </span>
    </button>
  );
}

/**
 * "Bom dia" e afins.
 *
 * Calculado na hora de desenhar, sem relógio próprio: nenhum componente do
 * painel fica mais de alguns minutos sem re-renderizar, e errar a saudação por
 * um instante na virada do meio-dia não é problema que valha um `setInterval`.
 */
function saudacao(): string {
  const h = new Date().getHours();
  if (h < 5) return "boa madrugada";
  if (h < 12) return "bom dia";
  if (h < 18) return "boa tarde";
  return "boa noite";
}
