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
  /** Quanto o dedo já girou o carro, acumulado em radianos. */
  arrasto: number;
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

/* ------------------------------------------------------------------ */
/* Encaixando o modelo                                                 */
/* ------------------------------------------------------------------ */

/** Comprimento real de um Eclipse 3G, em metros. É o que dá a escala. */
const COMPRIMENTO_REAL = 4.45;

/**
 * As três listras, em fração da largura do carro (1,90 m).
 *
 * Uma grossa no meio e duas finas de cada lado, que é o desenho do carro. A
 * grossa é o assunto; as finas são o contorno que dá o ar de faixa de corrida,
 * e num painel do tamanho deste elas viram quase um fio — o que está certo.
 */
const GROSSA_ATE = 0.056;
const FINA_DE = 0.076;
const FINA_ATE = 0.094;


/**
 * Põe o carro na escala e no chão.
 *
 * ## Escala e apoio
 *
 * O modelo vem em unidade de modelador — 69,9 de comprimento, sem dizer de quê.
 * Em vez de adivinhar o fator, mede-se a caixa e divide-se pelo comprimento que
 * um Eclipse tem de verdade: qualquer modelo que entre aqui sai no tamanho
 * certo, venha em polegada, centímetro ou nada. O mesmo vale para a altura — o
 * carro vem flutuando, e é a base da caixa que o assenta no chão.
 *
 * ## As rodas ficam paradas, e ficam INTEIRAS
 *
 * Elas chegaram a girar. Vinham num grupo só com os acessórios, então era
 * preciso recortá-las por posição e por raio — e todo recorte de geometria é um
 * palpite sobre onde uma peça acaba e a outra começa. Os palpites erravam: ora
 * o para-lama interno ia junto e orbitava o pneu, ora o corte comia uma meia-lua
 * da borracha.
 *
 * Sem girar, não é preciso recortar. E sem recortar, a roda fica exatamente como
 * o modelador a construiu, que é melhor do que qualquer aproximação que eu
 * conseguisse costurar. Trocou-se movimento por integridade — num quadro em que
 * o carro está parado a maior parte do tempo, é troca boa.
 *
 * A telemetria que sobrou é a que vale: mergulho de freada, rolagem de curva e o
 * balanço, que são o corpo inteiro e nunca dependeram do recorte.
 */
function encaixar(modelo: Object3D): void {
  modelo.updateWorldMatrix(true, true);
  const caixa = new Box3().setFromObject(modelo);
  const tamanho = caixa.getSize(new Vector3());

  // O maior lado horizontal é o comprimento, seja ele X ou Z.
  const escala = COMPRIMENTO_REAL / Math.max(tamanho.x, tamanho.z);
  modelo.scale.setScalar(escala);
  modelo.position.y = -caixa.min.y * escala;

  envernizar(modelo, caixa, tamanho);
}

/**
 * Onde os emblemas moram no atlas da textura, em coordenadas de 0 a 1.
 *
 * São dois, e foi preciso caçar os dois: o da tampa traseira, entre a terceira
 * luz de freio e o "GTS", e o do bico do capô, que é bem mais discreto e mora
 * numa parte completamente diferente do atlas — costura de fotogrametria não
 * tem, mas costura de modelador também não segue ordem nenhuma.
 *
 * Em fração e não em pixel porque a textura é reduzida antes de entrar no APK:
 * a mesma caixa vale em 2048, em 1024 ou no que vier.
 */
const EMBLEMAS: { x0: number; x1: number; y0: number; y1: number; modo: "claro" | "contorno" }[] = [
  { x0: 0.193, x1: 0.223, y0: 0.283, y1: 0.308, modo: "claro" },
  { x0: 0.697, x1: 0.73, y0: 0.428, y1: 0.462, modo: "contorno" },
];

/** Pinta um pixel de vermelho, guardando a luz que ele tinha. */
function pintarVermelho(p: Uint8ClampedArray, i: number): void {
  const luz = 0.3 * p[i] + 0.59 * p[i + 1] + 0.11 * p[i + 2];
  // A luminosidade de cada pixel é preservada no vermelho, de modo que o relevo
  // e a borda do emblema continuam lá — vermelho chapado apagaria o desenho e
  // deixaria uma mancha.
  p[i] = Math.min(255, 96 + luz * 0.62);
  p[i + 1] = luz * 0.1;
  p[i + 2] = luz * 0.1;
}

/**
 * O emblema da Mitsubishi, em vermelho.
 *
 * Os dois losangos — o da tampa e o do bico do capô — vêm na textura como o
 * carro de fábrica, e os do carro do dono são vermelhos. Não dá para trocar por
 * material — o emblema não é peça, é um desenho pintado no mesmo atlas da
 * lataria —, então a troca acontece nos pixels, uma vez, no carregamento.
 *
 * Os dois se destacam da chapa de maneiras diferentes, e por isso têm modos
 * diferentes. Isso não é capricho: é o que a textura tem, medido no atlas.
 *
 * - **O DE TRÁS** é um losango branco sólido sobre uma tampa escura — 205 a 234
 *   de luz contra 85 do fundo. O que o separa é a LUZ, e um corte por
 *   luminosidade o preenche inteiro. Modo `"claro"`.
 * - **O DA FRENTE** é do tom exato do capô no MIOLO (154,157,154 contra
 *   153,153,153): indistinguível. Só o CONTORNO dos losangos tem a tinta
 *   avermelhada de fábrica. Um corte por cor pinta só esse fio — foi o que
 *   deixou o logo "pela metade". Modo `"contorno"`: acha o fio vermelho e cresce
 *   a partir dele para dentro, enchendo os losangos, que são finos e cercados
 *   pelo próprio contorno.
 */
function emblemaVermelho(mapa: Texture | null): Texture | null {
  const img = mapa?.image as CanvasImageSource | undefined;
  if (!mapa || !img) return mapa;

  const largura = (img as { width: number }).width;
  const altura = (img as { height: number }).height;
  const cv = document.createElement("canvas");
  cv.width = largura;
  cv.height = altura;
  const ctx = cv.getContext("2d")!;
  ctx.drawImage(img, 0, 0);

  for (const caixa of EMBLEMAS) {
    const x0 = Math.floor(caixa.x0 * largura);
    const y0 = Math.floor(caixa.y0 * altura);
    const w = Math.ceil(caixa.x1 * largura) - x0;
    const h = Math.ceil(caixa.y1 * altura) - y0;
    const dados = ctx.getImageData(x0, y0, w, h);
    const p = dados.data;

    if (caixa.modo === "claro") {
      for (let i = 0; i < p.length; i += 4) {
        const luz = 0.3 * p[i] + 0.59 * p[i + 1] + 0.11 * p[i + 2];
        if (luz >= 168) pintarVermelho(p, i);
      }
    } else {
      /*
       * Semente: o fio avermelhado do contorno dos losangos.
       */
      const semente = new Uint8Array(w * h);
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const i = (y * w + x) * 4;
          const croma = p[i] - Math.min(p[i + 1], p[i + 2]);
          if (croma >= 8) semente[y * w + x] = 1;
        }
      }

      /*
       * Cresce a semente para dentro dos losangos. O raio acompanha a textura —
       * o losango é fino, e a metade de sua espessura é o que precisa ser
       * alcançada a partir do contorno. Só pinta pixel de chapa (nem o vão
       * escuro à direita, nem sombra): o miolo do losango tem a luz do capô.
       */
      const raio = Math.max(2, Math.round(largura * 0.003));
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const i = (y * w + x) * 4;
          const luz = 0.3 * p[i] + 0.59 * p[i + 1] + 0.11 * p[i + 2];
          if (luz < 90 || luz > 215) continue;

          let perto = semente[y * w + x] === 1;
          for (let dy = -raio; dy <= raio && !perto; dy++) {
            const yy = y + dy;
            if (yy < 0 || yy >= h) continue;
            for (let dx = -raio; dx <= raio; dx++) {
              const xx = x + dx;
              if (xx < 0 || xx >= w) continue;
              if (semente[yy * w + xx] === 1) {
                perto = true;
                break;
              }
            }
          }
          if (perto) pintarVermelho(p, i);
        }
      }
    }

    ctx.putImageData(dados, x0, y0);
  }

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

/**
 * As duas listras, pintadas NO SHADER.
 *
 * A primeira tentativa foi por cor de vértice, e ela falhou por um motivo que
 * só aparece com o modelo na mão: esta carroceria tem pouco mais de dois mil
 * vértices. Um capô inteiro são poucos polígonos, e os vértices ficam a mais de
 * vinte centímetros um do outro — uma faixa de treze centímetros simplesmente
 * cai no vão entre eles. Cor de vértice pinta os CANTOS e interpola o meio: ela
 * só sabe desenhar manchas maiores que a malha.
 *
 * Pintar no fragmento resolve porque a conta passa a ser por pixel, e a nitidez
 * da borda deixa de ter relação com a densidade de triângulos. O custo é uma
 * multiplicação por pixel, que numa área de 500x280 não é custo.
 *
 * A posição e a normal chegam ao fragmento em espaço de OBJETO, por varying
 * próprio: as que o three já oferece estão em espaço de vista, e ali o eixo do
 * carro se perde assim que a câmera ou o balanço mexem.
 */
function listrar(
  mat: MeshPhysicalMaterial,
  caixa: Box3,
  tamanho: Vector3,
  paraRaiz: Matrix4,
): void {
  const meio = (caixa.min.x + caixa.max.x) / 2;
  // Uma borda de meio centímetro de carro: nítida sem serrilhar.
  const suave = tamanho.x * 0.0025;

  mat.onBeforeCompile = (shader) => {
    shader.uniforms.uMeio = { value: meio };
    shader.uniforms.uGrossa = { value: GROSSA_ATE * tamanho.x };
    shader.uniforms.uFinaDe = { value: FINA_DE * tamanho.x };
    shader.uniforms.uFinaAte = { value: FINA_ATE * tamanho.x };
    shader.uniforms.uSuave = { value: suave };
    /*
     * A matriz que leva do espaço da malha para o do CARRO.
     *
     * Sem ela a listra assume que o eixo X da malha é a largura do carro — e
     * basta o exportador ter deixado um nó girado no meio do caminho para a
     * faixa sair atravessada, ou não sair. Passando a transformação, a conta
     * acontece onde ela faz sentido: no carro, não na peça.
     */
    shader.uniforms.uParaRaiz = { value: paraRaiz };

    shader.vertexShader = `
      uniform mat4 uParaRaiz;
      varying vec3 vLocal;
      varying vec3 vNormalLocal;
    ` + shader.vertexShader.replace(
      "#include <begin_vertex>",
      `#include <begin_vertex>
       vLocal = (uParaRaiz * vec4(position, 1.0)).xyz;
       vNormalLocal = mat3(uParaRaiz) * normal;`,
    );

    shader.fragmentShader = `
      uniform float uMeio;
      uniform float uGrossa;
      uniform float uFinaDe;
      uniform float uFinaAte;
      uniform float uSuave;
      varying vec3 vLocal;
      varying vec3 vNormalLocal;
    ` + shader.fragmentShader.replace(
      "#include <map_fragment>",
      `#include <map_fragment>
       {
         float d = abs(vLocal.x - uMeio);

         // A grossa do meio, e as duas finas de cada lado.
         float grossa = 1.0 - smoothstep(uGrossa - uSuave, uGrossa + uSuave, d);
         float fina = smoothstep(uFinaDe - uSuave, uFinaDe + uSuave, d)
                    * (1.0 - smoothstep(uFinaAte - uSuave, uFinaAte + uSuave, d));
         float faixa = max(grossa, fina);

         // Só onde a chapa não olha para o lado: a listra corre por cima e
         // desce pelas pontas, mas não vira na lateral do carro.
         float deCima = 1.0 - smoothstep(0.55, 0.85, abs(normalize(vNormalLocal).x));

         /*
          * E NUNCA no vidro.
          *
          * A carroceria e os vidros são a mesma malha com o mesmo material —
          * não há o que desligar por peça. Mas há como distinguir pelo que já
          * está na tela: a lataria é prata e o vidro é quase preto. Pular o que
          * é escuro deixa a listra na chapa e fora do para-brisa, e de quebra
          * protege grade, borracha e frisos, que são escuros pelo mesmo motivo.
          */
         float luz = dot(diffuseColor.rgb, vec3(0.3333));
         float ehChapa = smoothstep(0.09, 0.2, luz);

         diffuseColor.rgb *= mix(1.0, 0.3, faixa * deCima * ehChapa);
       }`,
    );
  };
  mat.needsUpdate = true;
}

/**
 * Verniz na pintura e a listra do meio.
 *
 * O modelo chega com material de brilho antigo (difuso mais especular), que o
 * conversor traduz para metálico-rugosidade do jeito conservador: sai uma
 * pintura fosca, de argila. Um carro tem verniz, e verniz é uma camada
 * espelhada por cima da cor — sem ela a lataria não devolve nada do ambiente e
 * o olho lê maquete.
 *
 * A listra vai por cor de vértice, e não na textura, porque o eixo dela é o eixo
 * do CARRO: é a faixa onde a largura está no meio. Mexer na textura exigiria
 * saber como o `.tga` foi costurado, e ele foi feito para outro carro que não
 * tem listra.
 */
function envernizar(modelo: Object3D, caixa: Box3, tamanho: Vector3): void {

  modelo.traverse((no) => {
    const malha = no as Mesh;
    if (!malha.isMesh) return;

    const ehRoda = /wheel|roda/i.test(malha.name + (malha.parent?.name ?? ""));
    const mat = malha.material as MeshStandardMaterial;

    const novo = new MeshPhysicalMaterial({
      map: ehRoda ? mat.map : emblemaVermelho(mat.map),
      normalMap: mat.normalMap,
      roughnessMap: mat.roughnessMap,
      metalnessMap: mat.metalnessMap,
      // A roda do modelo é de liga clara e a do carro é preta. `color`
      // multiplica a textura, então um cinza bem escuro apaga o prata e deixa
      // o desenho do aro — que é o que se vê de um aro preto na sombra.
      color: ehRoda ? new Color(0x3e4247) : mat.color,
      metalness: ehRoda ? 0.7 : 0.05,
      roughness: ehRoda ? 0.42 : 0.34,
      clearcoat: ehRoda ? 0.2 : 1,
      clearcoatRoughness: 0.06,
      envMapIntensity: ehRoda ? 1.5 : 1.15,
    });
    malha.material = novo;
    mat.dispose();

    if (ehRoda) return;
    listrar(novo, caixa, tamanho, malha.matrixWorld.clone());

  });
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
  const direcaoDaCamera = new Vector3(8.4, 2.9, 5.6).normalize();

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

  /** Raio do piso antes de caber no quadro. Quem o ajusta é `enquadrar`. */
  const RAIO_DO_PISO = 4.4;

  /*
   * Quanto da largura visível o chão pode ocupar.
   *
   * O resto é a faixa onde já não existe piso — e é nela que a luz termina de
   * morrer. Sem essa folga, o desvanecimento das bordas do piso acontece fora da
   * tela e o que se vê é o brilho batendo na beirada do canvas, cortado a seco
   * numa linha reta. Era o que fazia o herói ler como um card colado por cima do
   * painel em vez de fazer parte dele.
   */
  const FOLGA_DO_CHAO = 0.76;

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

    /*
     * O chão é dimensionado pelo que a câmera VÊ, não por um número escrito.
     *
     * A largura visível não é `CENA_LARGA`: aquilo é só o mínimo que precisa
     * caber. Num quadro largo e baixo como o herói, quem manda na distância é a
     * altura, e sobra muita largura — quase dez metros, contra os seis pedidos.
     * Dimensionar o piso pelo número pedido o deixaria pequeno demais; dimensionar
     * por um valor fixo grande o faria estourar quando a proporção mudasse. Medir
     * resolve os dois casos, e continua resolvendo quando o quadro mudar de forma.
     */
    const larguraVisivel = 2 * distancia * tanV * aspecto;
    chao.scale.setScalar((larguraVisivel * FOLGA_DO_CHAO) / (RAIO_DO_PISO * 2));
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
      encaixar(modelo);
      // O scan tem o comprimento no eixo Z, com o nariz no +Z; esta cena
      // trabalha com o carro apontando para +X, que é o lado de onde a câmera
      // olha. Um quarto de volta no sentido certo — o outro sentido mostra a
      // traseira, que foi o que aconteceu na primeira tentativa.
      /*
       * A pose de descanso é o três-quartos dianteiro, como nas fotos de
       * referência — e não o perfil. Perfil mostra a silhueta mas esconde tudo
       * que identifica o carro: grade, faróis e, principalmente, as listras,
       * que moram no capô e no teto.
       */
      modelo.rotation.y = 0.19;
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
   * Só a MANCHA DE LUZ entra no grupo que o enquadramento dimensiona.
   *
   * A sombra e o anel pertencem ao carro: são do tamanho dele, e encolher os
   * dois junto com o quadro faria o anel deixar de circundá-lo — vira uma
   * elipse pequena debaixo do carro em vez do apoio que a referência tem. O
   * piso é outra coisa: ele é o ambiente, e ambiente é do tamanho do que se vê.
   */
  const chao = new Group();
  cena.add(chao);

  /*
   * O piso do estúdio.
   *
   * Escuro e polido: ele não reflete o carro — reflexo de verdade custaria um
   * segundo passe de render, e a head unit não tem esse dinheiro —, mas reflete
   * o AMBIENTE, e é isso que dá o chão brilhante das fotos. O carro aparece nele
   * pela mancha de sombra, que é o que o olho procura para saber onde a roda
   * toca.
   *
   * ## Por que ele CABE no quadro, e por que isso importa
   *
   * O piso tinha 8,8 m de diâmetro num quadro que enquadra 6,1 m. O
   * desvanecimento das bordas dele — que existe justamente para a luz morrer
   * suave — acontecia fora da tela, e o que se via era o brilho batendo na
   * beirada do canvas e sendo cortado a seco, numa linha reta. Lia como um card
   * colado por cima do painel.
   *
   * Dimensionado a partir de `CENA_LARGA`, o piso morre por conta própria antes
   * da borda: não existe corte porque não existe nada para cortar. É o conserto
   * na origem. Uma máscara de CSS por cima do canvas trataria o sintoma, e ainda
   * comeria o teto do carro — que vive perto da borda de cima.
   */
  const piso = new Mesh(
    new CircleGeometry(RAIO_DO_PISO, 48),
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
  chao.add(piso);

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
       *
       * O que o dedo girou SOMA com isto, em vez de substituir. Somar é o que
       * faz o carro ficar onde alguém o deixou: se o balanço voltasse a mandar
       * sozinho, desfaria em segundos o que a pessoa acabou de fazer — e um
       * carro que volta ao lugar é pior do que um carro que não gira.
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
      carro.rotation.y = vitrine + estado.arrasto;
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
