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
  CanvasTexture,
  CircleGeometry,
  Color,
  BufferAttribute,
  BufferGeometry,
  Box3,
  DirectionalLight,
  DoubleSide,
  EquirectangularReflectionMapping,
  Group,
  Matrix4,
  Mesh,
  Object3D,
  MeshBasicMaterial,
  MeshPhysicalMaterial,
  MeshStandardMaterial,
  PerspectiveCamera,
  PMREMGenerator,
  RingGeometry,
  Scene,
  Texture,
  Vector3,
  WebGLRenderer,
} from "three";

import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

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

/** Transição suave entre dois limites — sem ela, toda máscara vira uma quina. */
const suavizar = (a: number, b: number, x: number) => {
  const t = limitar((x - a) / (b - a), 0, 1);
  return t * t * (3 - 2 * t);
};

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

/* ------------------------------------------------------------------ */
/* Consertando o scan                                                  */
/* ------------------------------------------------------------------ */

/**
 * Os dois consertos que transformam um scan num carro.
 *
 * ## O vidro
 *
 * Fotogrametria **não captura vidro**: a câmera atravessa, o algoritmo não acha
 * correspondência e devolve ruído. É por isso que o para-brisa e o teto vêm com
 * aquelas manchas brancas — não é falha do modelo, é o limite da técnica que o
 * produziu. Nenhum ajuste de luz esconde isso, porque a mancha está na textura.
 *
 * O conserto é escurecer a região da cabine por COR DE VÉRTICE, que multiplica a
 * textura: onde havia mancha branca passa a haver vidro escuro. Não custa um
 * triângulo, não depende de saber como o atlas de UV foi montado, e o degradê
 * suave nas bordas evita uma faixa preta com quina.
 *
 * E o teto ficar escuro junto não é acidente: o 3G tem o painel do teto em preto
 * de fábrica, então a cabine inteira escura é o que o carro tem de verdade.
 *
 * ## O brilho
 *
 * O scan vem `KHR_materials_unlit`, que o three carrega como material sem
 * iluminação nenhuma — a textura é desenhada crua na tela. Fica correto e fica
 * morto: carro sem reflexo é carro de papel. Trocando por material com verniz e
 * mapa de ambiente, a mesma textura passa a ganhar brilho especular e a
 * responder ao balanço, que é o que faz a lataria parecer lataria.
 *
 * A luz direta fica baixa de propósito: a iluminação do dia da captura já está
 * assada na textura, e somar as duas deixaria o carro estourado.
 */
function envidracarEDarBrilho(modelo: Object3D): void {
  /*
   * TUDO AQUI É EM FRAÇÃO DA CAIXA DO CARRO, e não em metros. A razão é uma
   * pegadinha que custou um carro partido ao meio na tela.
   *
   * O modelo passou por `quantize` para caber no APK: as posições deixaram de
   * ser float em metros e viraram inteiros de 16 bits normalizados, com a escala
   * de volta guardada na matriz do nó. Ler `position.getY()` devolve, então, um
   * número entre -1 e 1 que não tem relação nenhuma com altura — e um corte
   * escrito em metros cai num lugar arbitrário da malha.
   *
   * Medindo a caixa depois das matrizes aplicadas e trabalhando em fração dela,
   * a conta passa a ser independente de escala, de unidade e de quanto o modelo
   * foi comprimido. Trocar o `.glb` por outro continua funcionando.
   */
  modelo.updateWorldMatrix(true, true);
  const caixa = new Box3().setFromObject(modelo);
  const tamanho = caixa.getSize(new Vector3());
  const v = new Vector3();

  modelo.traverse((no) => {
    const malha = no as Mesh;
    if (!malha.isMesh) return;

    const geo = malha.geometry;
    const pos = geo.attributes.position;
    const cores = new Float32Array(pos.count * 3);

    for (let i = 0; i < pos.count; i++) {
      v.fromBufferAttribute(pos, i).applyMatrix4(malha.matrixWorld);
      // 0 a 1 dentro da caixa: largura, altura e comprimento.
      const fx = (v.x - caixa.min.x) / tamanho.x;
      const fy = (v.y - caixa.min.y) / tamanho.y;
      const fz = (v.z - caixa.min.z) / tamanho.z;

      /*
       * A cabine, no comprimento, e acima da cintura.
       *
       * A faixa saiu da conta, não do olho: o carro tem 4,46 m, a base do
       * para-brisa fica a ~2,2 m do nariz e o fim do vidro traseiro a ~3,9 m.
       * Com o nariz na ponta 1 do eixo, isso dá a cabine entre 0,12 e 0,52. A
       * primeira tentativa chutou 0,24 a 0,66 e escureceu o capô em vez das
       * janelas — o vidro continuou branco e ninguém entendeu por quê.
       */
      const naCabine = suavizar(0.09, 0.16, fz) * (1 - suavizar(0.5, 0.58, fz));
      const acima = suavizar(0.5, 0.62, fy);
      const vidro = naCabine * acima;

      // 1 é a lataria como veio; 0,12 é vidro. Nunca zero: preto absoluto
      // apagaria o contorno da coluna e a cabine viraria um buraco.
      let t = 1 - vidro * 0.88;

      /*
       * A listra do meio, do capô à tampa.
       *
       * Em cor de vértice e não na textura porque o eixo dela é o eixo do
       * CARRO: a faixa é onde a largura está no meio. Fosse na textura, seria
       * preciso saber como o atlas de UV foi costurado — e atlas de
       * fotogrametria é picotado, sem costura previsível.
       *
       * Só em cima: listra de teto não desce pela lateral nem passa por baixo.
       */
      const naFaixa = 1 - suavizar(0.062, 0.088, Math.abs(fx - 0.5));
      const emCima = suavizar(0.42, 0.58, fy);
      t *= 1 - naFaixa * emCima * 0.55;

      cores[i * 3] = t;
      cores[i * 3 + 1] = t;
      cores[i * 3 + 2] = t * 1.04;
    }

    geo.setAttribute("color", new BufferAttribute(cores, 3));
    descascarOChao(geo, malha.matrixWorld, caixa.min.y, tamanho.y);

    const antigo = malha.material as MeshBasicMaterial;
    malha.material = new MeshPhysicalMaterial({
      map: pratear(antigo.map),
      vertexColors: true,
      metalness: 0.0,
      roughness: 0.4,
      clearcoat: 0.95,
      clearcoatRoughness: 0.07,
      envMapIntensity: 1.0,
    });
    antigo.dispose();
  });
}

/**
 * Tira a crosta de chão que veio grudada no carro.
 *
 * Fotogrametria captura o que está em volta junto: o asfalto embaixo do carro
 * virou uma saia irregular e escura presa nas soleiras e nos pneus. Não dá para
 * limpar por cor — a crosta e o pneu são igualmente escuros —, mas dá por
 * ALTURA: abaixo de quatro centímetros do chão não existe carro, existe chão. O
 * triângulo cujo centro cai ali é descartado, e a mancha de sombra da cena cobre
 * o corte.
 */
function descascarOChao(
  geo: BufferGeometry,
  matriz: Matrix4,
  baseY: number,
  alturaTotal: number,
): void {
  const pos = geo.attributes.position;
  const idx = geo.index;
  if (!idx) return;

  // Os 2,5% de baixo da caixa. Em fração, pelo mesmo motivo do resto.
  const CORTE = 0.025;
  const v = new Vector3();
  const alturas = new Float32Array(pos.count);
  for (let i = 0; i < pos.count; i++) {
    v.fromBufferAttribute(pos, i).applyMatrix4(matriz);
    alturas[i] = (v.y - baseY) / alturaTotal;
  }

  const mantidos: number[] = [];
  for (let t = 0; t < idx.count; t += 3) {
    const a = idx.getX(t);
    const b = idx.getX(t + 1);
    const c = idx.getX(t + 2);
    if ((alturas[a] + alturas[b] + alturas[c]) / 3 >= CORTE) mantidos.push(a, b, c);
  }

  geo.setIndex(mantidos);
}

/**
 * O carro fica prata.
 *
 * O scan é de um Eclipse vinho, e o carro do dono é prata com listra. Cor de
 * vértice não resolveria: ela MULTIPLICA, e multiplicação não tira saturação —
 * vinho vezes qualquer coisa continua vinho. Então a troca acontece na textura,
 * pixel a pixel, uma vez no carregamento.
 *
 * A seleção é por saturação e por canal dominante: pinta-se de prata o que é
 * avermelhado e medianamente saturado, que é a lataria. Fica de fora o que já é
 * neutro (roda, pneu, vidro, asfalto) e o que é MUITO saturado — que são as
 * lanternas, e lanterna prateada seria pior que carro vinho.
 */
function pratear(mapa: Texture | null): Texture | null {
  const img = mapa?.image as CanvasImageSource | undefined;
  if (!mapa || !img) return mapa;

  const largura = (img as { width: number }).width;
  const altura = (img as { height: number }).height;
  const cv = document.createElement("canvas");
  cv.width = largura;
  cv.height = altura;
  const ctx = cv.getContext("2d", { willReadFrequently: false })!;
  ctx.drawImage(img, 0, 0);

  const dados = ctx.getImageData(0, 0, largura, altura);
  const p = dados.data;

  for (let i = 0; i < p.length; i += 4) {
    const r = p[i];
    const g = p[i + 1];
    const b = p[i + 2];

    const maior = Math.max(r, g, b);
    const menor = Math.min(r, g, b);
    if (maior < 12) continue;
    const saturacao = (maior - menor) / maior;

    // Lataria: avermelhada, saturada mas não gritante.
    const ehPintura = r === maior && saturacao > 0.16 && saturacao < 0.52;
    if (!ehPintura) continue;

    // Guarda a sombra da foto e joga fora a cor: o prata é o mesmo desenho de
    // luz, sem matiz. Um toque de azul, que é o que separa prata de cinza.
    const cru = 0.3 * r + 0.59 * g + 0.11 * b;
    // Curva de contraste: afunda a sujeira da captura e levanta o realce, que é
    // o que separa prata de cinza encardido.
    const luz = Math.min(255, Math.pow(cru / 255, 0.82) * 232 + 20);
    p[i] = luz * 0.985;
    p[i + 1] = luz * 0.995;
    p[i + 2] = Math.min(255, luz * 1.025);
  }

  ctx.putImageData(dados, 0, 0);

  const nova = new CanvasTexture(cv);
  // Textura de glTF não é espelhada no eixo vertical, e canvas por padrão é —
  // sem isto o carro sai com a textura de cabeça para baixo.
  nova.flipY = false;
  nova.colorSpace = mapa.colorSpace;
  nova.wrapS = mapa.wrapS;
  nova.wrapT = mapa.wrapT;
  nova.needsUpdate = true;
  return nova;
}

/** Onde o modelo mora. Em `public/`, então o Vite o copia cru para o `dist`. */
const CAMINHO_DO_MODELO = "/carro.glb";

export function montarCena(
  canvas: HTMLCanvasElement,
  acentoInicial: string,
  aoCarregar?: () => void,
  aoFalhar?: () => void,
): Cena {
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
  const direcaoDaCamera = new Vector3(8.4, 1.55, 5.6).normalize();

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
  const CENA_LARGA = 6.1;
  const CENA_ALTA = 2.9;

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
  cena.add(new AmbientLight(0xffffff, 0.5));

  const principal = new DirectionalLight(0xffffff, 0.75);
  principal.position.set(4.5, 6.2, 5.2);
  cena.add(principal);

  const preenchimento = new DirectionalLight(0xcfd8e6, 0.25);
  preenchimento.position.set(-5.5, 2.2, 4);
  cena.add(preenchimento);

  const contorno = new DirectionalLight(new Color(acentoInicial), 0.75);
  contorno.position.set(-5, 2.4, -5.5);
  cena.add(contorno);

  /* --- o carro --- */

  const carro = new Group();
  cena.add(carro);

  const corpo = new Group();
  carro.add(corpo);

  /*
   * O carro agora é um SCAN, e não geometria escrita.
   *
   * O que estava aqui antes eram seções transversais interpoladas — uma
   * carroceria de verdade em matemática, e o mais longe que dá para chegar
   * escrevendo curva em código. Não chegava nem perto de uma foto, e não ia
   * chegar: o que separa as duas coisas são dezenas de milhares de vértices que
   * alguém posicionou um a um, mais faróis, grade, frisos e vinco de porta.
   *
   * O modelo é fotogrametria: malha capturada de um Eclipse real com a textura
   * tirada das mesmas fotos. Por isso o material vem `KHR_materials_unlit` — a
   * iluminação está ASSADA na textura, e as luzes desta cena não têm efeito
   * sobre ele. É uma troca consciente: perde-se poder relightar, ganha-se o
   * carro parecendo um carro.
   *
   * Ele chega por rede e demora; até chegar, o quadro fica com o desenho em SVG,
   * que é o mesmo plano B de sempre.
   */
  const carregador = new GLTFLoader();
  carregador.load(
    CAMINHO_DO_MODELO,
    (gltf) => {
      const modelo = gltf.scene;
      envidracarEDarBrilho(modelo);
      // O scan tem o comprimento no eixo Z, com o nariz no +Z; esta cena
      // trabalha com o carro apontando para +X, que é o lado de onde a câmera
      // olha. Um quarto de volta no sentido certo — o outro sentido mostra a
      // traseira, que foi o que aconteceu na primeira tentativa.
      modelo.rotation.y = Math.PI / 2 + 0.55;
      // O piso do scan fica 2,8 cm abaixo de zero — sobe para a roda tocar o chão.
      modelo.position.y = 0.028;
      corpo.add(modelo);
      aoCarregar?.();
    },
    undefined,
    (err) => {
      console.warn("[eclipse] não deu para carregar o modelo do carro", err);
      aoFalhar?.();
    },
  );


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

  let vitrine = 0;
  let relogio = 0;
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

      /*
       * As rodas não giram, e a perda é declarada.
       *
       * O scan é uma malha única: não existe "a roda" para girar, existe uma
       * superfície contínua que inclui o pneu. Separá-la exigiria recortar
       * geometria por posição e torcer para o corte cair no lugar certo em
       * quatro cantos — frágil, e caro para o que entrega. O que sobrou de
       * telemetria continua: mergulho de freada, rolagem de curva e o giro de
       * vitrine, que são o corpo inteiro e funcionam igual.
       */

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
      corpo.position.y = tremor * 0.004 * Math.sin(performance.now() / 42);

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
        // Balanço de vitrine, e não volta completa.
        //
        // Um scan de fotogrametria tem lados bons e lados ruins: o vidro não
        // fotografa, então o para-brisa e o teto vêm com manchas brancas, e a
        // traseira tem menos cobertura. Girar 360° exibiria justamente isso de
        // graça. Balançando vinte graus para cada lado em volta do três-quartos
        // dianteiro, o quadro prova que é tridimensional mostrando só a parte
        // que ficou boa.
        if (!estado.menosMovimento) {
          relogio += dt;
          vitrine = Math.sin(relogio * ((Math.PI * 2) / 26)) * 0.34;
        }
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
      // Os materiais do modelo vêm do próprio GLTF; o `traverse` acima já solta
      // as geometrias, e aqui soltam-se as texturas junto.
      cena.traverse((o) => {
        const mat = (o as Mesh).material;
        for (const m of Array.isArray(mat) ? mat : [mat]) m?.dispose();
      });
      cena.environment?.dispose();
      renderer.dispose();
    },
  };
}
