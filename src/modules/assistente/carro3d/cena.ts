/**
 * O Eclipse em três dimensões.
 *
 * ## Por que não um modelo pronto
 *
 * O caminho óbvio seria baixar um GLTF de cupê genérico. Foi descartado, e vale
 * registrar para ninguém tentar de novo sem saber: um modelo decente pesa de 2 a
 * 15 MB num APK que já tem 66; a licença raramente permite redistribuir; e
 * nenhum deles é um Eclipse — seria outro carro no lugar do carro que o painel
 * inteiro fala sobre. O que está em `public/carro.glb` é fotogrametria de um
 * Eclipse 3G de verdade: 384 KB, oito mil triângulos, texturas em webp.
 *
 * ## O carro é PRATA, e é o painel que o escurece
 *
 * A carroceria do scan é clara — prata de fábrica, com as listras de corrida em
 * cinza. Não é decisão desta cena e não dá para reverter por material: é a
 * fotografia da lataria, colada no atlas. Quem escurece o quadro é o painel
 * atrás dele, e quem devolve a identidade do perfil é a luz de contorno na cor
 * do acento, que corre pelo teto e pelas caixas de roda.
 *
 * ## O orçamento
 *
 * Oito mil triângulos, sem sombra projetada (três manchas no chão fazem o
 * serviço), sem piso e sem mapa de ambiente de arquivo — o ambiente é um
 * degradê de 512x256 gerado em memória. O laço roda a 30 fps, não 60: numa roda
 * girando ninguém vê a diferença, e é metade do custo.
 *
 * O que se gasta, se gasta em NITIDEZ: antialias, filtro anisotrópico e tone
 * mapping. São os três itens que separam "render" de "foto de estúdio", e os
 * três são baratos numa área de 500x280. O que ficou de fora é o caro: reflexo
 * do carro no chão, que pede uma segunda passada do modelo inteiro.
 */

import {
  ACESFilmicToneMapping,
  AdditiveBlending,
  AmbientLight,
  BackSide,
  CanvasTexture,
  CircleGeometry,
  Color,
  Box3,
  DirectionalLight,
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
  // 512x256 e não 256x128: o horizonte é uma BORDA, e borda em textura baixa
  // chega ao PMREM já derretida — some justamente o risco reto que a lataria
  // devolve. É custo de carregamento, não de quadro.
  const L = 512;
  const A = 256;
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

  /*
   * A régua de luz.
   *
   * Uma faixa comprida e estreita logo acima do horizonte. É ela que vira o
   * RISCO que escorre da caixa de roda dianteira à traseira nas fotos de
   * catálogo — uma softbox redonda não sabe desenhar isso: ela devolve um brilho
   * redondo. O que dá o risco é uma fonte de luz que já é um risco.
   */
  const regua = ctx.createLinearGradient(0, A * 0.375, 0, A * 0.455);
  regua.addColorStop(0, "rgba(255,255,255,0)");
  regua.addColorStop(0.5, "rgba(255,255,255,0.9)");
  regua.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = regua;
  ctx.fillRect(L * 0.02, A * 0.375, L * 0.56, A * 0.08);

  const textura = new CanvasTexture(cv);
  textura.mapping = EquirectangularReflectionMapping;

  const pmrem = new PMREMGenerator(renderer);
  const alvo = pmrem.fromEquirectangular(textura);
  pmrem.dispose();
  textura.dispose();

  return alvo.texture;
}

/**
 * Uma mancha redonda que morre nas bordas, branca. Serve de sombra e de poça.
 *
 * `nucleo` é o quanto ela fecha no miolo e `meio` onde ela já está morrendo. A
 * cor sai do material — aqui só se desenha a FORMA, no canal alfa, e a mesma
 * textura serve para a poça de luz e para as duas sombras.
 */
function texturaDaMancha(nucleo: number, meio: number): CanvasTexture {
  const cv = document.createElement("canvas");
  cv.width = cv.height = 128;
  const ctx = cv.getContext("2d")!;

  const g = ctx.createRadialGradient(64, 64, 0, 64, 64, 64);
  g.addColorStop(0, `rgba(255,255,255,${nucleo})`);
  g.addColorStop(0.55, `rgba(255,255,255,${meio})`);
  g.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, 128, 128);

  return new CanvasTexture(cv);
}

/* ------------------------------------------------------------------ */
/* A montagem                                                          */
/* ------------------------------------------------------------------ */

export interface Cena {
  /**
   * O canvas mudou de tamanho.
   *
   * `largura` e `altura` são as do CANVAS, que é maior que o quadro do painel —
   * ele sangra para fora do card de propósito, para a luz do chão não acabar
   * numa linha reta na beirada. As sangrias são a razão entre um e outro (1 =
   * sem sangria), e servem para o carro ficar do MESMO tamanho de antes: sem
   * elas, um canvas maior só aproximaria a câmera e a sangria não sangraria
   * nada. Ver `.carro3d` no CSS.
   */
  redimensionar(
    largura: number,
    altura: number,
    sangriaX: number,
    sangriaY: number,
  ): void;
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
function encaixar(modelo: Object3D, anisotropia: number): void {
  modelo.updateWorldMatrix(true, true);
  const caixa = new Box3().setFromObject(modelo);
  const tamanho = caixa.getSize(new Vector3());

  // O maior lado horizontal é o comprimento, seja ele X ou Z.
  const escala = COMPRIMENTO_REAL / Math.max(tamanho.x, tamanho.z);
  modelo.scale.setScalar(escala);
  modelo.position.y = -caixa.min.y * escala;

  envernizar(modelo, caixa, tamanho, anisotropia);
}

/**
 * O emblema da TAMPA TRASEIRA, e onde ele mora no atlas.
 *
 * Um losango branco sólido sobre uma tampa escura — 205 a 234 de luz contra 85
 * do fundo. O que o separa da chapa é a LUZ, e um corte por luminosidade o
 * preenche inteiro, com a forma exata que o fotógrafo capturou. Recortar é o
 * certo aqui: a informação está na textura.
 *
 * Em fração e não em pixel porque a textura é reduzida antes de entrar no APK:
 * a mesma caixa vale em 2048, em 1024 ou no que vier.
 */
const EMBLEMA_DE_TRAS = {
  x0: 0.193,
  x1: 0.223,
  y0: 0.283,
  y1: 0.308,
  /** Acima disto o pixel é emblema. Entre os 234 dele e os 85 da tampa, sobra. */
  luzMinima: 168,
};

/**
 * O emblema do BICO DO CAPÔ, que não se recorta — se DESENHA.
 *
 * ## Por que recortar não funciona aqui
 *
 * A marca inteira ocupa **13 por 13 pixels** no atlas de 1024, e o miolo dos
 * losangos tem o tom exato do capô em volta (154,157,154 contra 153,153,153).
 * Só o contorno guarda um fio da tinta avermelhada de fábrica, e ainda assim
 * com croma da ordem do ruído do webp.
 *
 * A tentativa anterior semeava nesse fio e crescia a partir dele. Num quadrado
 * de treze pixels, crescer três pixels transforma UMA semente falsa numa mancha
 * de sete por sete — metade da marca. Não existe limiar que salve: a forma não
 * está na textura, e nenhum recorte inventa o que não foi fotografado.
 *
 * ## Então desenha-se
 *
 * Três losangos de 60° encostados no centro, que é a marca da Mitsubishi.
 * `giro` não é gosto: a ilha de UV do bico chega girada no atlas, e os números
 * abaixo saíram de medir onde as três pontas caem na textura de verdade — a de
 * cima-direita a -55°, a de baixo a 65°, a da esquerda a 185°.
 *
 * Antes do carimbo, `limpeza` alisa o borrão original (tira o croma e puxa a
 * luz para a da chapa em volta); sem isso o fio avermelhado de fábrica ficaria
 * como um fantasma em volta da marca nova.
 *
 * Tudo em fração do atlas, pelo mesmo motivo da caixa de trás.
 */
const EMBLEMA_DA_FRENTE = {
  cx: 0.7132,
  cy: 0.4441,
  /** Do centro até a ponta de um losango. */
  raio: 0.00703,
  giro: (-55 * Math.PI) / 180,
  /** Até onde a chapa é alisada antes de receber o carimbo. */
  limpeza: 0.0103,
  /** A largura da transição do alisamento. Curta demais deixa um disco visível. */
  esfumado: 0.0044,
};

/**
 * O vermelho do emblema para um pixel de dada luminosidade.
 *
 * A luz do pixel é preservada no vermelho, de modo que o relevo e a sombra da
 * chapa continuam lá — vermelho chapado apagaria o desenho e viraria adesivo.
 */
function vermelhoDoEmblema(luz: number): [number, number, number] {
  return [Math.min(255, 96 + luz * 0.62), luz * 0.1, luz * 0.1];
}

/** O corte por luz que preenche o losango da tampa traseira. */
function pintarOEmblemaDeTras(
  ctx: CanvasRenderingContext2D,
  largura: number,
  altura: number,
): void {
  const e = EMBLEMA_DE_TRAS;
  const x0 = Math.floor(e.x0 * largura);
  const y0 = Math.floor(e.y0 * altura);
  const w = Math.ceil(e.x1 * largura) - x0;
  const h = Math.ceil(e.y1 * altura) - y0;

  const dados = ctx.getImageData(x0, y0, w, h);
  const p = dados.data;
  for (let i = 0; i < p.length; i += 4) {
    const luz = 0.3 * p[i] + 0.59 * p[i + 1] + 0.11 * p[i + 2];
    if (luz < e.luzMinima) continue;
    const [r, g, b] = vermelhoDoEmblema(luz);
    p[i] = r;
    p[i + 1] = g;
    p[i + 2] = b;
  }
  ctx.putImageData(dados, x0, y0);
}

/**
 * Os três losangos de 60° da marca, como caminho.
 *
 * Cada um é um losango cuja diagonal longa mede `raio` e sai do centro: ponta
 * interna no centro, ponta externa em `raio`, e os dois vértices de lado na
 * metade do caminho, afastados de `raio/(2·√3)` — a conta de um losango de
 * 60/120 graus. Três deles a 120° um do outro, e está feita a marca.
 */
function losangosDaMarca(
  cx: number,
  cy: number,
  raio: number,
  giro: number,
): Path2D {
  const caminho = new Path2D();
  const lado = raio / (2 * Math.sqrt(3));
  for (let k = 0; k < 3; k++) {
    const a = giro + (k * 2 * Math.PI) / 3;
    const ux = Math.cos(a);
    const uy = Math.sin(a);
    caminho.moveTo(cx, cy);
    caminho.lineTo(cx + (raio / 2) * ux - lado * uy, cy + (raio / 2) * uy + lado * ux);
    caminho.lineTo(cx + raio * ux, cy + raio * uy);
    caminho.lineTo(cx + (raio / 2) * ux + lado * uy, cy + (raio / 2) * uy - lado * ux);
    caminho.closePath();
  }
  return caminho;
}

/**
 * Alisa o borrão de fábrica e carimba a marca por cima, no bico do capô.
 *
 * Quem antisserrilha é o próprio canvas: a marca é desenhada numa máscara e o
 * canal alfa dela vira a cobertura de cada pixel. Em treze pixels isso é o mais
 * nítido que a textura comporta — e é outro planeta em relação a uma dilatação
 * de vizinhança, que só sabe pintar pixel inteiro.
 */
function carimbarOEmblemaDaFrente(
  ctx: CanvasRenderingContext2D,
  largura: number,
  altura: number,
): void {
  const e = EMBLEMA_DA_FRENTE;
  const cx = e.cx * largura;
  const cy = e.cy * altura;
  const raio = e.raio * largura;
  const limpeza = e.limpeza * largura;
  const esfumado = e.esfumado * largura;

  const x0 = Math.max(0, Math.floor(cx - limpeza - 2));
  const y0 = Math.max(0, Math.floor(cy - limpeza - 2));
  const w = Math.min(largura, Math.ceil(cx + limpeza + 2)) - x0;
  const h = Math.min(altura, Math.ceil(cy + limpeza + 2)) - y0;
  if (w <= 0 || h <= 0) return;

  // A máscara: a marca desenhada em branco, e o alfa dela é a cobertura.
  const mcv = document.createElement("canvas");
  mcv.width = w;
  mcv.height = h;
  const mctx = mcv.getContext("2d")!;
  mctx.fillStyle = "#fff";
  mctx.fill(losangosDaMarca(cx - x0, cy - y0, raio, e.giro));
  const mascara = mctx.getImageData(0, 0, w, h).data;

  const dados = ctx.getImageData(x0, y0, w, h);
  const p = dados.data;

  /*
   * A luz da chapa em volta, medida num anel logo fora da área alisada.
   *
   * É para ela que o borrão é puxado. Um valor escrito à mão daria um remendo
   * mais claro ou mais escuro que o capô assim que a exposição da textura
   * mudasse; a mediana do anel é o capô dizendo o próprio tom.
   */
  const anel: number[] = [];
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const d = Math.hypot(x0 + x - cx, y0 + y - cy);
      if (d <= limpeza || d >= limpeza + 6) continue;
      const i = (y * w + x) * 4;
      anel.push(0.3 * p[i] + 0.59 * p[i + 1] + 0.11 * p[i + 2]);
    }
  }
  anel.sort((a, b) => a - b);
  const base = anel.length ? anel[anel.length >> 1] : 148;

  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      let r = p[i];
      let g = p[i + 1];
      let b = p[i + 2];
      let luz = 0.3 * r + 0.59 * g + 0.11 * b;

      // 1. alisa: tira o croma de fábrica e puxa a luz para a da chapa.
      const d = Math.hypot(x0 + x - cx, y0 + y - cy);
      const k = limitar((limpeza - d) / esfumado, 0, 1);
      if (k > 0) {
        r -= (r - Math.min(g, b)) * k;
        const f = (luz + (base - luz) * 0.85 * k) / Math.max(1, luz);
        r *= f;
        g *= f;
        b *= f;
        luz = 0.3 * r + 0.59 * g + 0.11 * b;
      }

      // 2. carimba, na cobertura que a máscara mandar.
      const a = mascara[i + 3] / 255;
      if (a > 0) {
        const [vr, vg, vb] = vermelhoDoEmblema(luz);
        r += (vr - r) * a;
        g += (vg - g) * a;
        b += (vb - b) * a;
      }

      p[i] = r;
      p[i + 1] = g;
      p[i + 2] = b;
    }
  }
  ctx.putImageData(dados, x0, y0);
}

/**
 * Os emblemas da Mitsubishi, em vermelho.
 *
 * O carro de fábrica os tem prateados e o do dono os tem vermelhos. Não dá para
 * trocar por material — o emblema não é peça, é um desenho pintado no mesmo
 * atlas da lataria —, então a troca acontece nos pixels, uma vez, no
 * carregamento.
 *
 * Os dois recebem tratamentos diferentes porque estão na textura de maneiras
 * diferentes: o de trás foi fotografado e se recorta; o da frente não sobreviveu
 * à resolução e se desenha. O porquê de cada um está em `EMBLEMA_DE_TRAS` e
 * `EMBLEMA_DA_FRENTE`.
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

  pintarOEmblemaDeTras(ctx, largura, altura);
  carimbarOEmblemaDaFrente(ctx, largura, altura);

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
    ).replace(
      "#include <lights_physical_fragment>",
      `#include <lights_physical_fragment>
       {
         /*
          * VERNIZ SÓ NA CHAPA.
          *
          * Uma malha só carrega lataria, vidro, grade, borracha e pneu, e o
          * verniz é do MATERIAL, não da peça: envernizar o material envernizava
          * o pneu junto. Pneu lustrado é exatamente a leitura de "brinquedo de
          * plástico" que este arquivo passou a existir para evitar.
          *
          * O discriminador é o mesmo da listra — a lataria é clara e o resto é
          * quase preto —, só que aqui ele governa acabamento em vez de cor. E o
          * que é escuro fica fosco, que é o que borracha e plástico texturizado
          * fazem com a luz.
          */
         float chapa = smoothstep(0.06, 0.22, dot(diffuseColor.rgb, vec3(0.3333)));
         material.clearcoat *= chapa;
         material.roughness = mix(0.78, material.roughness, chapa);
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
 * A listra vai no shader, e não na textura, porque o eixo dela é o eixo do
 * CARRO: é a faixa onde a largura está no meio. Mexer na textura exigiria saber
 * como o atlas foi costurado, e ele foi capturado de um carro que não tem
 * listra.
 *
 * ## O filtro anisotrópico
 *
 * Capô e teto são vistos de raspão desta câmera, e é justamente aí que o filtro
 * normal desiste: ele amostra um quadrado onde o pixel na tela é um retângulo
 * comprido, e o que sobra é mingau. Numa textura de fotogrametria — que é toda
 * detalhe fino — isso apaga metade do que se pagou para ter. Custa só nos pixels
 * de raspão, e é o item mais barato desta lista.
 */
function envernizar(
  modelo: Object3D,
  caixa: Box3,
  tamanho: Vector3,
  anisotropia: number,
): void {

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

    for (const t of [novo.map, novo.normalMap, novo.roughnessMap, novo.metalnessMap]) {
      if (t && t.anisotropy !== anisotropia) {
        t.anisotropy = anisotropia;
        t.needsUpdate = true;
      }
    }

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
    /*
     * COM antialias, e isto é uma reversão consciente.
     *
     * Ele estava desligado por custo, apostando que carro escuro sobre fundo
     * escuro esconderia a serrilha. Não esconde: o contorno na cor do perfil
     * desenha justamente a silhueta, e é a silhueta que serrilha. Um degrau de
     * escada na linha do teto é o detalhe que grita "render" numa tela em que
     * tudo mais é vetor.
     *
     * O que paga a conta é o tamanho: MSAA num buffer de 500x280 numa GPU de
     * tile resolve dentro da própria tile, e o quadro do carro já é menos de um
     * sexto do mapa. É o item mais caro da lista de nitidez e ainda assim é
     * barato.
     */
    antialias: true,
    powerPreference: "low-power",
  });
  renderer.setClearAlpha(0);

  /*
   * Tone mapping, que é o que faz o verniz parecer verniz.
   *
   * Sem ele, tudo que passa de 1.0 vira branco puro — o brilho da softbox na
   * lataria satura num borrão chapado, sem miolo e sem beirada. O ACES ROLA a
   * saturação: o brilho continua estourando, mas estoura passando por um rosa
   * quente antes de chegar ao branco, que é exatamente o que um filme (e um
   * sensor) faz. É uma operação por pixel para o item que mais muda a leitura
   * de "3D" para "foto".
   *
   * A exposição sobe junto porque o ACES escurece a imagem média: 1.0 nele é
   * mais escuro que 1.0 sem tone mapping nenhum.
   */
  renderer.toneMapping = ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.25;

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
   * O quanto a cena precisa caber, em metros: o carro de ponta a ponta com a
   * sombra, e a altura do teto com uma folga em cima.
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
  const enquadrar = (aspecto: number, sangriaX: number, sangriaY: number) => {
    const tanV = Math.tan(((camera.fov * Math.PI) / 180) / 2);
    // A caixa cresce junto com a sangria: é isso que faz os pixels de sobra
    // serem SOBRA, e não um zoom. O carro continua do tamanho que tinha dentro
    // do quadro do painel; o que o canvas ganhou é chão em volta.
    const porAltura = (CENA_ALTA * sangriaY) / 2 / tanV;
    const porLargura = (CENA_LARGA * sangriaX) / 2 / (tanV * aspecto);
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
   * superfície. Por isso a ambiente aqui é baixa: ela só tira o preto do fundo
   * da sombra.
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
   * O modelo é fotogrametria: malha capturada de um Eclipse real, com a textura
   * tirada das mesmas fotos. Ele chega com `pbrMetallicRoughness` normal — cor,
   * normal e rugosidade —, e `envernizar` troca tudo por material físico para
   * pôr verniz por cima. As luzes desta cena, portanto, VALEM.
   *
   * O que vem de brinde é a iluminação assada na textura: sombra de para-lama,
   * reflexo de céu, tudo que estava lá no dia da captura. É luz em cima de luz,
   * e é por isso que a `AmbientLight` daqui é baixa — quem já iluminou o carro
   * foi o fotógrafo.
   *
   * Ele chega por rede e demora; até chegar, o quadro fica com o desenho em SVG,
   * que é o mesmo plano B de sempre.
   */
  const carregador = new GLTFLoader();
  carregador.load(
    CAMINHO_DO_MODELO,
    (gltf) => {
      const modelo = gltf.scene;
      // Quatro é o joelho da curva: dobra a nitidez de raspão e não é o 16 que
      // faz uma GPU de tile reclamar. Aparelho que não tem, devolve 1 e segue.
      encaixar(modelo, Math.min(4, renderer.capabilities.getMaxAnisotropy()));
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
   * Não existe chão. Existe uma POÇA DE LUZ e a sombra dentro dela.
   *
   * Havia aqui um disco de piso polido e um anel na cor do perfil. Os dois
   * saíram: numa foto de carro sobre painel escuro não há piso nenhum — há o
   * carro e a marca que ele deixa no preto. O disco pedia um enquadramento
   * próprio, uma folga calculada para o desvanecimento dele não ser cortado pela
   * beirada do canvas, e ainda assim lia como um card colado por cima do painel.
   *
   * ## A poça não é enfeite: sem ela o carro FLUTUA
   *
   * Sombra é subtração, e não há o que subtrair de um fundo que já é #0d1117 —
   * preto sobre preto é preto. Com o piso, a sombra tinha uma superfície clara
   * para escurecer; sem ele, ela some, e some justamente a única coisa que dizia
   * onde o pneu encosta. Foi o que aconteceu na primeira tentativa: o carro
   * pairando no vazio.
   *
   * A poça devolve o que o piso dava sem devolver o piso: uma mancha aditiva,
   * fraca e sem borda, que levanta o fundo só onde a sombra precisa acontecer.
   * Ela não tem quina para ser cortada pela beirada — morre sozinha muito antes.
   *
   * Por cima dela vêm duas sombras: a LARGA, que é a luz do estúdio contornando
   * o carro, e a de CONTATO, curta e quase preta, onde o pneu tapa o chão. Com
   * só a larga o carro paira num borrão; com só a de contato ele fica recortado
   * com tesoura.
   *
   * ## As medidas vão em METROS, e o eixo estava trocado
   *
   * As três manchas vinham escritas como raio mais fator de achatamento, e nessa
   * forma cabia um erro de 90°: o carro deste scan tem o COMPRIMENTO no eixo Z,
   * não no X — a rotação que ele leva ao entrar é de 11°, não de um quarto de
   * volta. As manchas tinham, portanto, seis metros atravessados na largura de
   * um carro de 1,90 m e três metros e pouco no comprimento de um carro de
   * 4,45 m. Ninguém enxerga isso numa linha `scale.set(1, 0.62, 1)`; escrevendo
   * "4,9 de comprimento por 2,5 de largura", fica difícil errar.
   *
   * O plano sai do `CircleGeometry` no XY e é deitado no XZ, então o raio vem da
   * LARGURA (que é o X) e a escala local em Y estica o Z, que é o comprimento.
   *
   * A câmera está a 16° acima do chão, e nessa inclinação o chão é visto quase
   * de fio: mancha larga demais não fica embaixo do carro, esparrama por meia
   * tela.
   */
  let ordem = 0;
  const mancha = (
    comprimento: number,
    largura: number,
    cor: number,
    opacidade: number,
    nucleo: number,
    meio: number,
    aditiva: boolean,
  ) => {
    const m = new Mesh(
      new CircleGeometry(largura / 2, 40),
      new MeshBasicMaterial({
        color: cor,
        opacity: opacidade,
        map: texturaDaMancha(nucleo, meio),
        transparent: true,
        depthWrite: false,
        side: BackSide,
        ...(aditiva ? { blending: AdditiveBlending } : {}),
        // Um degradê escuro sobre um painel escuro é o caso de livro de
        // banding: sem ruído, as faixas do degradê viram anéis visíveis.
        dithering: true,
      }),
    );
    // Deitar em X e alinhar em Z. Na ordem `XYZ` do Euler o giro em Z acontece
    // ANTES do tombo em X, e girar a mancha no próprio plano antes de deitá-la
    // é o mesmo que girá-la no chão depois: são os 11° do carro.
    m.rotation.set(Math.PI / 2, 0, -0.19);
    // Um milímetro de escada entre elas, só para não brigarem por profundidade.
    m.position.y = 0.002 + ordem * 0.002;
    m.scale.set(1, comprimento / largura, 1);
    // Todas escrevem cor e nenhuma escreve profundidade: quem manda na ordem é
    // isto, e não a distância à câmera, que muda com o balanço.
    m.renderOrder = ordem++;
    /*
     * Dentro de `carro`, e não da cena: assim a mancha GIRA junto.
     *
     * Uma sombra comprida presa ao mundo fica certa na pose de descanso e
     * atravessada assim que o dedo vira o carro — o carro aponta para um lado e
     * a mancha continua apontando para o outro. Em `carro` ela acompanha o giro
     * de vitrine e o arrasto; e como o mergulho e a rolagem moram em `corpo`,
     * ela não inclina junto, que é o certo: sombra fica no chão.
     */
    carro.add(m);
  };

  /*
   * A poça é MAIOR que as sombras, e isso não é detalhe.
   *
   * Na primeira tentativa ela tinha quase o tamanho da sombra larga, e o
   * resultado foi as duas se anularem: sobrava luz só na franja de fora, uma
   * cunha esparramada para um lado só, e embaixo do carro continuava tudo preto.
   * A poça precisa transbordar a sombra por todos os lados — é ela que dá o
   * chão, e a sombra é o que ela perde onde o carro tapa.
   */
  /*
   * E a poça tem de morrer DENTRO da tela.
   *
   * Ela sangra para fora do card justamente para não acabar num corte reto (ver
   * `.carro3d` no CSS); se ela chegasse viva na beirada da tela, o corte só
   * teria mudado de lugar — de cima do card para dois centímetros ao lado dele,
   * que é pior, porque ali não há quina nenhuma que justifique a linha. Sete
   * metros e meio é o maior tamanho que ainda cabe com folga: medido no canto
   * esquerdo, que é onde o eixo comprido dela chega mais perto da borda.
   */
  mancha(7.5, 4.4, 0x9fb6d4, 0.46, 0.85, 0.32, true);
  // A larga: a luz do estúdio contornando a carroceria.
  mancha(4.9, 2.5, 0x000000, 0.8, 0.8, 0.34, false);
  // A de contato: onde o pneu tapa o chão. Curta, estreita e quase preta.
  mancha(3.4, 1.2, 0x000000, 0.95, 0.95, 0.4, false);

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
    redimensionar(largura, altura, sangriaX, sangriaY) {
      // Teto no `devicePixelRatio`: numa head unit ele é 1, mas num celular
      // deitado ou num Mac ele é 2 ou 3 — e triplicar a área de pixel de uma
      // cena 3D por causa de um quadro de 500 px não paga.
      renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
      renderer.setSize(largura, altura, false);
      camera.aspect = largura / Math.max(1, altura);
      enquadrar(camera.aspect, sangriaX, sangriaY);
      camera.updateProjectionMatrix();
    },

    atualizar(estado, dt) {
      if (estado.acento !== acentoAtual) {
        acentoAtual = estado.acento;
        cor.set(acentoAtual);
        contorno.color.copy(cor);
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
