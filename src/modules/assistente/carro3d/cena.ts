/**
 * O Eclipse em três dimensões.
 *
 * ## Por que não um modelo pronto
 *
 * O caminho óbvio seria baixar um GLTF de carro e carregar. Ele foi descartado
 * por três motivos, e vale registrar para ninguém tentar de novo sem saber:
 * um modelo decente pesa de 2 a 15 MB num APK que já tem 66; a licença de
 * redistribuição raramente permite; e nenhum deles é um Eclipse — seria "um
 * cupê genérico" no lugar do carro que o painel inteiro fala sobre. A geometria
 * daqui sai da silhueta que o projeto já tinha (ver `blueprint.ts`), custa zero
 * byte de download e continua sendo o carro certo.
 *
 * ## Por que estilizado e não fotorrealista
 *
 * Fotorrealismo pede material PBR com mapa de ambiente de verdade, texturas e,
 * de preferência, sombra projetada — tudo isso numa Mali-G52 que já está
 * segurando o mapa. E, mesmo se coubesse, um carro branco de estúdio brigaria
 * com um painel escuro cuja única cor é o acento de quem dirige. Aqui a
 * carroceria é escura e quem a desenha é uma luz de contorno na cor do perfil:
 * lê como foto de produto, mantém a identidade e cabe no orçamento.
 *
 * ## O orçamento
 *
 * Menos de 3 mil triângulos, sem sombra projetada (uma mancha no chão faz o
 * serviço), sem antialias, sem textura de arquivo — o mapa de ambiente é um
 * degradê de 64x32 gerado em memória. O laço roda a 30 fps, não 60: numa roda
 * girando ninguém vê a diferença, e é metade do custo.
 */

import {
  AmbientLight,
  BackSide,
  BoxGeometry,
  CanvasTexture,
  CircleGeometry,
  Color,
  CylinderGeometry,
  DirectionalLight,
  DoubleSide,
  EquirectangularReflectionMapping,
  Group,
  Mesh,
  MeshBasicMaterial,
  MeshPhysicalMaterial,
  MeshStandardMaterial,
  PerspectiveCamera,
  PMREMGenerator,
  RingGeometry,
  Scene,
  Vector3,
  WebGLRenderer,
} from "three";

import { CARRO } from "./blueprint";
import { construirCarroceria, construirEstufa } from "./carroceria";

/** O que a cena precisa saber do carro de verdade a cada quadro. */
export interface EstadoDaCena {
  velocidade: number;
  rpm: number | null;
  aceleracao: number;
  curva: number;
  parado: boolean;
  /** Cor do perfil ativo, em hex. É ela que contorna o carro. */
  acento: string;
  /** Quem pediu menos movimento não recebe o giro de vitrine. */
  menosMovimento: boolean;
}

const limitar = (v: number, min: number, max: number) =>
  Math.max(min, Math.min(max, v));

/* ------------------------------------------------------------------ */
/* O ambiente                                                          */
/* ------------------------------------------------------------------ */

/**
 * O céu falso que a lataria reflete.
 *
 * Um `MeshStandardMaterial` metálico sem mapa de ambiente reflete o vazio e
 * fica chapado — é o erro que faz carro 3D parecer brinquedo de plástico. Um
 * HDRI de verdade custa megabytes; este é um degradê de 64x32 pixels desenhado
 * em memória: escuro embaixo (o asfalto), claro em cima (o céu) e uma faixa na
 * cor do perfil na altura do horizonte, que é o que aparece escorrendo pela
 * lateral do carro.
 */
function ambiente(renderer: WebGLRenderer) {
  const L = 256;
  const A = 128;
  const cv = document.createElement("canvas");
  cv.width = L;
  cv.height = A;
  const ctx = cv.getContext("2d")!;

  // Céu, horizonte e chão do estúdio.
  const g = ctx.createLinearGradient(0, 0, 0, A);
  g.addColorStop(0, "#c9d2de");
  g.addColorStop(0.34, "#7f8896");
  g.addColorStop(0.49, "#4a5058");
  g.addColorStop(0.51, "#2a2e34");
  g.addColorStop(0.78, "#1a1d21");
  g.addColorStop(1, "#0d0f12");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, L, A);

  /*
   * As duas softboxes.
   *
   * São elas que desenham o carro. O brilho comprido e reto que escorre pela
   * lateral de um carro em foto de estúdio não é "luz": é o REFLEXO de uma caixa
   * de luz retangular. Sem uma forma clara no ambiente para a lataria espelhar,
   * a pintura fica com um brilho redondo e sem graça, por mais luz direcional
   * que se jogue nela.
   */
  const caixa = (x: number, y: number, w: number, h: number, forca: number) => {
    const r = ctx.createRadialGradient(x + w / 2, y + h / 2, 0, x + w / 2, y + h / 2, w / 2);
    r.addColorStop(0, `rgba(255,255,255,${forca})`);
    r.addColorStop(1, "rgba(255,255,255,0)");
    ctx.fillStyle = r;
    ctx.save();
    ctx.translate(x + w / 2, y + h / 2);
    ctx.scale(1, h / w);
    ctx.translate(-(x + w / 2), -(y + h / 2));
    ctx.fillRect(x - w, y - w, w * 3, w * 3);
    ctx.restore();
  };
  caixa(L * 0.06, A * 0.1, L * 0.42, A * 0.22, 0.95);
  caixa(L * 0.58, A * 0.14, L * 0.3, A * 0.16, 0.6);

  const textura = new CanvasTexture(cv);
  textura.mapping = EquirectangularReflectionMapping;

  const pmrem = new PMREMGenerator(renderer);
  const alvo = pmrem.fromEquirectangular(textura);
  pmrem.dispose();
  textura.dispose();

  return alvo.texture;
}

/**
 * A máscara que apaga o piso nas bordas.
 *
 * Um disco de chão opaco encheria o fundo de cinza e taparia o painel — o quadro
 * do herói é nu, e o fundo dele é a tela. Some das beiradas para dentro, o piso
 * existe só onde serve: embaixo do carro, dando o brilho de estúdio e o apoio
 * para a roda tocar.
 */
function texturaDoPiso(): CanvasTexture {
  const cv = document.createElement("canvas");
  cv.width = cv.height = 128;
  const ctx = cv.getContext("2d")!;

  const g = ctx.createRadialGradient(64, 64, 0, 64, 64, 64);
  g.addColorStop(0, "#ffffff");
  g.addColorStop(0.42, "#c8c8c8");
  g.addColorStop(0.78, "#2a2a2a");
  g.addColorStop(1, "#000000");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, 128, 128);

  return new CanvasTexture(cv);
}

/** A mancha de sombra no chão. Custa um quad; sombra projetada custa um passe. */
function texturaDaSombra(): CanvasTexture {
  const cv = document.createElement("canvas");
  cv.width = cv.height = 128;
  const ctx = cv.getContext("2d")!;

  const g = ctx.createRadialGradient(64, 64, 0, 64, 64, 64);
  g.addColorStop(0, "rgba(0,0,0,0.78)");
  g.addColorStop(0.55, "rgba(0,0,0,0.34)");
  g.addColorStop(1, "rgba(0,0,0,0)");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, 128, 128);

  return new CanvasTexture(cv);
}

/* ------------------------------------------------------------------ */
/* A montagem                                                          */
/* ------------------------------------------------------------------ */

export interface Cena {
  redimensionar(largura: number, altura: number): void;
  atualizar(estado: EstadoDaCena, dt: number): void;
  desenhar(): void;
  destruir(): void;
}

export function montarCena(canvas: HTMLCanvasElement, acentoInicial: string): Cena {
  const renderer = new WebGLRenderer({
    canvas,
    // Fundo transparente: o herói é um quadro nu, e a lavagem do `body` precisa
    // continuar aparecendo por trás do carro.
    alpha: true,
    // Sem antialias por decisão de custo. O que tira a serrilha aqui é o
    // `devicePixelRatio` da head unit, e o carro é escuro sobre fundo escuro —
    // é o caso em que serrilha menos aparece.
    antialias: false,
    powerPreference: "low-power",
  });
  renderer.setClearAlpha(0);

  const cena = new Scene();
  cena.environment = ambiente(renderer);
  cena.environmentIntensity = 1.15;

  /*
   * Câmera de lente longa e de baixo.
   *
   * Os 24° de campo são o que separa "foto de carro" de "foto de celular": lente
   * longa achata a perspectiva e deixa as proporções honestas, enquanto um campo
   * largo curvaria o carro e engordaria o que estivesse mais perto. E a câmera
   * fica na altura do capô, não na de quem está em pé — carro visto de cima
   * parece miniatura.
   */
  const camera = new PerspectiveCamera(24, 1, 0.5, 40);
  const alvoDaCamera = new Vector3(0, 0.52, 0);
  /** De onde se olha. O comprimento não importa — quem o define é `enquadrar`. */
  const direcaoDaCamera = new Vector3(8.4, 2.5, 5.6).normalize();

  /**
   * O quanto a cena precisa caber, em metros: o carro de ponta a ponta com o
   * anel do chão, e a altura do teto com uma folga em cima.
   */
  /*
   * A caixa muda com a ALTURA da câmera, não só com o carro.
   *
   * Visto de cima, o comprimento do carro projeta em altura na tela: a esta
   * inclinação, os 4,4 m de comprimento valem mais de um metro vertical, que
   * somam com a altura do teto. Dimensionar pela altura real do carro cortava o
   * para-choque fora do quadro.
   */
  const CENA_LARGA = 6.0;
  const CENA_ALTA = 3.0;

  /**
   * Aproxima ou afasta a câmera para a cena caber na caixa que ela recebeu.
   *
   * Sem isto a distância seria fixa, e aí o carro só ficaria do tamanho certo
   * numa proporção de tela: no herói do painel — que é bem mais largo do que
   * alto — a altura mandava, e o carro virava uma miniatura no meio de dois
   * vazios laterais. Com o ajuste, a câmera recua numa caixa alta e chega perto
   * numa caixa larga, e o carro ocupa o quadro nos dois casos.
   */
  const enquadrar = (aspecto: number) => {
    const tanV = Math.tan(((camera.fov * Math.PI) / 180) / 2);
    const porAltura = CENA_ALTA / 2 / tanV;
    const porLargura = CENA_LARGA / 2 / (tanV * aspecto);
    const distancia = Math.max(porAltura, porLargura) * 1.06;

    camera.position.copy(direcaoDaCamera).multiplyScalar(distancia).add(alvoDaCamera);
    camera.lookAt(alvoDaCamera);
  };

  /* --- luzes --- */

  /*
   * Três pontos, como num estúdio: principal alta à frente, preenchimento fraco
   * do lado oposto para a sombra não fechar em preto, e contorno atrás para
   * separar o carro do fundo. O grosso da luz, porém, vem do ambiente — numa
   * pintura envernizada, luz direcional faz o brilho pontual e o AMBIENTE faz a
   * superfície. Por isso a ambiente aqui é baixa: ela só levanta o piso.
   */
  cena.add(new AmbientLight(0xffffff, 0.18));

  const principal = new DirectionalLight(0xffffff, 2.1);
  principal.position.set(4.5, 6.2, 5.2);
  cena.add(principal);

  const preenchimento = new DirectionalLight(0xcfd8e6, 0.5);
  preenchimento.position.set(-5.5, 2.2, 4);
  cena.add(preenchimento);

  const contorno = new DirectionalLight(new Color(acentoInicial), 0.75);
  contorno.position.set(-5, 2.4, -5.5);
  cena.add(contorno);

  /* --- materiais --- */

  /*
   * Pintura automotiva de verdade: base clara e verniz por cima.
   *
   * A versão anterior era quase preta e metálica, e errava duas vezes. Escura,
   * ela some no painel escuro e a forma vira silhueta; e `metalness` alto é
   * *metal*, não pintura — carro pintado é dielétrico com uma camada de verniz,
   * e é o verniz que faz o reflexo comprido e nítido escorrer pela lateral. O
   * `clearcoat` do `MeshPhysicalMaterial` é exatamente essa camada.
   */
  const pintura = new MeshPhysicalMaterial({
    color: 0xeef1f5,
    metalness: 0.0,
    roughness: 0.42,
    clearcoat: 1,
    clearcoatRoughness: 0.045,
    envMapIntensity: 1.35,
  });
  const preto = new MeshStandardMaterial({
    color: 0x14171c,
    metalness: 0.25,
    roughness: 0.5,
  });
  const borracha = new MeshStandardMaterial({
    color: 0x0d0e10,
    metalness: 0.0,
    roughness: 0.88,
  });
  // Liga polida: é o contraponto claro e duro da pintura macia.
  const roda = new MeshStandardMaterial({
    color: 0xb9c0c8,
    metalness: 1,
    roughness: 0.22,
    envMapIntensity: 1.5,
  });
  const aceso = new MeshBasicMaterial({ color: 0xfff3d8 });
  const brasa = new MeshBasicMaterial({ color: 0xe8433c });

  /* --- carroceria --- */

  const carro = new Group();
  cena.add(carro);

  const corpo = new Group();
  /*
   * A carroceria sobe um dedo em relação às rodas.
   *
   * A soleira do blueprint desce quase até o chão — é o que dá o ar de
   * esportivo no desenho chapado. Em três dimensões, com a roda sendo um sólido
   * e não um recorte, a mesma soleira cobria metade dela e o carro virava um
   * bloco pousado. Levantar um pouco devolve o vão embaixo da lateral, que é
   * onde o olho procura a roda.
   */
  corpo.position.y = 0.075;
  carro.add(corpo);

  corpo.add(new Mesh(construirCarroceria(), pintura));

  /*
   * O vidro: pouco verniz e mais rugosidade que a lataria, de propósito. Com o
   * mesmo verniz da pintura ele espelharia a softbox e voltaria a ficar claro —
   * ver `construirEstufa`.
   */
  const vidro = new MeshPhysicalMaterial({
    color: 0x090c12,
    metalness: 0.0,
    roughness: 0.22,
    clearcoat: 0.35,
    clearcoatRoughness: 0.16,
    envMapIntensity: 0.55,
    side: DoubleSide,
  });
  corpo.add(new Mesh(construirEstufa(), vidro));

  /*
   * O aerofólio do 3G, que é discreto.
   *
   * Antes era uma prateleira de asa de pista, e era ela que fazia o carro ler
   * como protótipo de Le Mans em vez de cupê de rua. Nas fotos do Eclipse o
   * aerofólio é baixo, colado na tampa e com pés curtos — some de longe e só
   * aparece no três-quartos.
   */
  const asa = new Mesh(new BoxGeometry(0.34, 0.035, 1.16), pintura);
  asa.position.set(-1.9, 0.94, 0);
  corpo.add(asa);
  for (const z of [-0.44, 0.44]) {
    const pe = new Mesh(new BoxGeometry(0.07, 0.1, 0.05), pintura);
    pe.position.set(-1.88, 0.885, z);
    corpo.add(pe);
  }

  // Retrovisores: pequenos, e é a ausência deles que mais faz um carro 3D
  // parecer maquete.
  for (const z of [-0.66, 0.66]) {
    const braco = new Mesh(new BoxGeometry(0.1, 0.035, 0.09), preto);
    braco.position.set(0.42, 0.86, z);
    corpo.add(braco);
    const concha = new Mesh(new BoxGeometry(0.17, 0.1, 0.07), pintura);
    concha.position.set(0.52, 0.885, z * 1.09);
    concha.rotation.y = z > 0 ? -0.18 : 0.18;
    corpo.add(concha);
  }

  // Saia lateral e a faixa escura embaixo: cortam a altura da lataria e é o que
  // faz o carro parecer baixo sem precisar deitar a soleira no chão.
  for (const z of [-0.71, 0.71]) {
    const saia = new Mesh(new BoxGeometry(2.5, 0.13, 0.05), preto);
    saia.position.set(-0.05, 0.2, z);
    corpo.add(saia);
  }

  // A grade e as entradas de ar do para-choque dianteiro.
  const grade = new Mesh(new BoxGeometry(0.06, 0.11, 0.62), preto);
  grade.position.set(2.16, 0.36, 0);
  corpo.add(grade);

  // Farol repuxado e lanterna: dois pontos acesos que dão escala ao resto.
  for (const z of [-0.48, 0.48]) {
    const farol = new Mesh(new BoxGeometry(0.1, 0.09, 0.34), aceso);
    farol.position.set(2.11, 0.6, z);
    corpo.add(farol);

    const lanterna = new Mesh(new BoxGeometry(0.07, 0.09, 0.3), brasa);
    lanterna.position.set(-2.15, 0.72, z);
    corpo.add(lanterna);
  }

  /* --- rodas --- */

  /*
   * O eixo da roda é o Z, e ele é deitado UMA vez só.
   *
   * O cilindro do three nasce em pé, com o eixo no Y. Deitar no Z é o que o
   * transforma em roda — e é fácil fazer isso duas vezes sem perceber, uma no
   * grupo e outra na malha: 90° mais 90° devolve o cilindro à vertical, e o
   * carro fica com quatro tambores enterrados no chão em vez de rodas. Foi
   * exatamente o que aconteceu na primeira versão, e só apareceu olhando.
   *
   * Com o eixo no Z, girar a roda é girar em Z — não em Y.
   */
  const rodas: Group[] = [];
  const { eixo } = CARRO;

  /*
   * A roda, do pneu ao miolo.
   *
   * A versão anterior era um cilindro com cinco caixas atravessadas, e era o que
   * mais denunciava que aquilo não era um carro: roda é a peça que todo mundo
   * conhece de cor. Aqui ela tem as camadas que se veem numa foto — flanco de
   * borracha fosco, aro de liga polida um pouco recuado, raios que afinam para
   * fora e um cubo no centro. O contraste entre borracha fosca e liga espelhada
   * é metade do efeito; a outra metade é o aro NÃO chegar até a borda do pneu.
   */
  const pneuGeo = new CylinderGeometry(eixo.raio, eixo.raio, eixo.largura, 28, 1);
  pneuGeo.rotateX(Math.PI / 2);

  // O flanco: um disco levemente menor, para o pneu não ser um tubo reto.
  const flancoGeo = new CylinderGeometry(
    eixo.raio * 0.995,
    eixo.raio * 0.93,
    eixo.largura * 0.24,
    28,
    1,
  );
  flancoGeo.rotateX(Math.PI / 2);

  const aroGeo = new CylinderGeometry(
    eixo.raio * 0.68,
    eixo.raio * 0.68,
    eixo.largura * 0.62,
    28,
    1,
  );
  aroGeo.rotateX(Math.PI / 2);

  const cuboGeo = new CylinderGeometry(
    eixo.raio * 0.2,
    eixo.raio * 0.2,
    eixo.largura * 0.7,
    16,
    1,
  );
  cuboGeo.rotateX(Math.PI / 2);

  for (const x of [eixo.traseiro, eixo.dianteiro]) {
    for (const z of [-eixo.bitola, eixo.bitola]) {
      const conjunto = new Group();
      conjunto.position.set(x, eixo.altura, z);

      conjunto.add(new Mesh(pneuGeo, borracha));
      for (const lado of [-1, 1]) {
        const flanco = new Mesh(flancoGeo, borracha);
        flanco.position.z = lado * eixo.largura * 0.38;
        flanco.rotation.x = lado > 0 ? 0 : Math.PI;
        conjunto.add(flanco);
      }
      conjunto.add(new Mesh(aroGeo, roda));
      conjunto.add(new Mesh(cuboGeo, roda));

      /*
       * Seis raios que afinam para fora, como a liga das fotos.
       *
       * O deslocamento vai na GEOMETRIA e não na posição da malha: objeto gira
       * em volta da própria origem, então um raio posicionado e depois girado
       * rodopiaria em torno de si mesmo em vez de abrir o leque a partir do
       * centro da roda.
       */
      for (let i = 0; i < 6; i++) {
        const g = new CylinderGeometry(
          eixo.raio * 0.075,
          eixo.raio * 0.14,
          eixo.raio * 0.66,
          6,
          1,
        );
        // O cilindro nasce em pé; deitá-lo em Z e depois deitar no plano da roda.
        g.rotateZ(Math.PI / 2);
        g.translate(eixo.raio * 0.4, 0, 0);
        const raio = new Mesh(g, roda);
        raio.rotation.z = (i * Math.PI * 2) / 6;
        raio.position.z = eixo.largura * 0.1;
        conjunto.add(raio);
      }

      carro.add(conjunto);
      rodas.push(conjunto);
    }
  }

  /* --- chão --- */

  /*
   * O piso do estúdio.
   *
   * Escuro e polido: ele não reflete o carro — reflexo de verdade custaria um
   * segundo passe de render, e a head unit não tem esse dinheiro —, mas reflete
   * o AMBIENTE, e é isso que dá o chão brilhante das fotos. O carro aparece nele
   * pela mancha de sombra, que é o que o olho procura para saber onde a roda
   * toca.
   */
  const piso = new Mesh(
    new CircleGeometry(4.4, 48),
    new MeshStandardMaterial({
      color: 0x14171b,
      metalness: 0.9,
      roughness: 0.3,
      envMapIntensity: 0.9,
      transparent: true,
      alphaMap: texturaDoPiso(),
      depthWrite: false,
    }),
  );
  piso.rotation.x = -Math.PI / 2;
  piso.scale.set(1, 0.66, 1);
  cena.add(piso);

  const sombra = new Mesh(
    new CircleGeometry(3.1, 32),
    new MeshBasicMaterial({
      map: texturaDaSombra(),
      transparent: true,
      depthWrite: false,
      side: BackSide,
    }),
  );
  sombra.rotation.x = Math.PI / 2;
  sombra.position.y = 0.006;
  sombra.scale.set(1, 0.62, 1);
  cena.add(sombra);

  // O anel de acento no chão: o mesmo truque da referência. Ele não ilumina
  // nada — só diz "o carro está pousado aqui" e amarra a cena ao perfil.
  const anel = new Mesh(
    new RingGeometry(2.62, 2.66, 64),
    new MeshBasicMaterial({
      color: new Color(acentoInicial),
      transparent: true,
      opacity: 0.42,
      side: DoubleSide,
      depthWrite: false,
    }),
  );
  anel.rotation.x = -Math.PI / 2;
  anel.position.y = 0.002;
  anel.scale.set(1, 0.62, 1);
  cena.add(anel);

  /* ------------------------------------------------------------------ */
  /* O laço                                                              */
  /* ------------------------------------------------------------------ */

  let giroDaRoda = 0;
  let vitrine = 0;
  // Suavizados, e não aplicados crus: o OBD entrega leitura a cada ~0,9 s, e
  // pendurar a carroceria direto no dado faria o carro dar solavancos a cada
  // amostra. Aqui cada quadro caminha um pouco em direção ao alvo.
  let mergulhoAtual = 0;
  let rolagemAtual = 0;
  let acentoAtual = acentoInicial;

  const cor = new Color();

  return {
    redimensionar(largura, altura) {
      // Teto no `devicePixelRatio`: numa head unit ele é 1, mas num celular
      // deitado ou num Mac ele é 2 ou 3 — e triplicar a área de pixel de uma
      // cena 3D por causa de um quadro de 500 px não paga.
      renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
      renderer.setSize(largura, altura, false);
      camera.aspect = largura / Math.max(1, altura);
      enquadrar(camera.aspect);
      camera.updateProjectionMatrix();
    },

    atualizar(estado, dt) {
      if (estado.acento !== acentoAtual) {
        acentoAtual = estado.acento;
        cor.set(acentoAtual);
        contorno.color.copy(cor);
        (anel.material as MeshBasicMaterial).color.copy(cor);
      }

      // Roda: velocidade real virando rotação real. A 100 km/h com pneu de 0,36 m
      // de raio, isso é ~12 voltas por segundo — bem além do que 30 fps mostram,
      // então a roda vai "andar para trás" como no cinema. É o comportamento
      // certo: uma roda que gira devagar a 100 km/h mentiria mais.
      const rad = estado.velocidade / 3.6 / CARRO.eixo.raio;
      giroDaRoda += rad * dt;
      for (const r of rodas) r.rotation.z = -giroDaRoda;

      // Mergulho de freada e rolagem de curva, com a mesma assimetria do
      // desenho em SVG: frear afunda o nariz mais do que acelerar o levanta.
      const mergulhoAlvo = limitar(-estado.aceleracao * 0.0032, -0.035, 0.055);
      const rolagemAlvo = limitar(estado.curva * 0.0022, -0.06, 0.06);
      const passo = Math.min(1, dt * 4);
      mergulhoAtual += (mergulhoAlvo - mergulhoAtual) * passo;
      rolagemAtual += (rolagemAlvo - rolagemAtual) * passo;
      corpo.rotation.z = mergulhoAtual;
      corpo.rotation.x = rolagemAtual;

      // Tremor de marcha lenta, na amplitude do giro do motor. Some andando: o
      // que balança um carro parado é o motor, e o que balança um carro andando
      // é a estrada, que aqui não existe.
      const tremor =
        estado.parado && !estado.menosMovimento
          ? limitar(((estado.rpm ?? 700) - 700) / 5200, 0, 1)
          : 0;
      corpo.position.y = 0.075 + tremor * 0.004 * Math.sin(performance.now() / 42);

      /*
       * O giro de vitrine.
       *
       * É o que prova que a cena é tridimensional. Um render 3D parado é
       * indistinguível de uma figura — e o motivo de trocar o SVG por isto era
       * justamente ter volume. Uma volta a cada 48 s: devagar o bastante para
       * não puxar o olho de quem dirige, rápido o bastante para que uma olhada
       * de dois segundos pegue o carro em outro ângulo.
       *
       * Só parado. Andando, quem manda no ângulo é a curva: o carro se apruma
       * para a frente e inclina para dentro do que o GPS está descrevendo.
       */
      if (estado.parado) {
        if (!estado.menosMovimento) vitrine += dt * ((Math.PI * 2) / 48);
      } else {
        const alvo = limitar(estado.curva * 0.004, -0.22, 0.22);
        vitrine += (alvo - vitrine) * Math.min(1, dt * 1.5);
      }
      carro.rotation.y = vitrine;
    },

    desenhar() {
      renderer.render(cena, camera);
    },

    destruir() {
      cena.traverse((o) => {
        const mesh = o as Mesh;
        if (mesh.geometry) mesh.geometry.dispose();
      });
      for (const mat of [pintura, vidro, preto, borracha, roda, aceso, brasa]) mat.dispose();
      cena.environment?.dispose();
      renderer.dispose();
    },
  };
}
