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
  ExtrudeGeometry,
  Group,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  PerspectiveCamera,
  PMREMGenerator,
  RingGeometry,
  Scene,
  WebGLRenderer,
} from "three";

import {
  afunilar,
  CARRO,
  perfilDaCarroceria,
  perfilDoVidro,
} from "./blueprint";

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
function ambiente(renderer: WebGLRenderer, acento: string) {
  const cv = document.createElement("canvas");
  cv.width = 64;
  cv.height = 32;
  const ctx = cv.getContext("2d")!;

  const g = ctx.createLinearGradient(0, 0, 0, 32);
  g.addColorStop(0, "#8f9bb3");
  g.addColorStop(0.42, "#39414f");
  g.addColorStop(0.5, acento);
  g.addColorStop(0.58, "#12151a");
  g.addColorStop(1, "#05070a");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, 64, 32);

  const textura = new CanvasTexture(cv);
  textura.mapping = EquirectangularReflectionMapping;

  const pmrem = new PMREMGenerator(renderer);
  const alvo = pmrem.fromEquirectangular(textura);
  pmrem.dispose();
  textura.dispose();

  return alvo.texture;
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
  cena.environment = ambiente(renderer, acentoInicial);

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
  camera.position.set(8.6, 1.24, 5.8);
  camera.lookAt(0, 0.62, 0);

  /* --- luzes --- */

  // Preenchimento baixo: quem revela a forma é o ambiente e as duas direcionais.
  cena.add(new AmbientLight(0xffffff, 0.28));

  const principal = new DirectionalLight(0xffffff, 1.9);
  principal.position.set(5, 6.5, 4.5);
  cena.add(principal);

  /*
   * A luz de contorno, na cor do perfil, vindo de trás e de baixo.
   *
   * É ela que faz o carro existir. Numa carroceria escura sobre fundo escuro, a
   * silhueta se perderia; o contorno aceso pela borda separa o carro do fundo
   * sem precisar clarear a pintura, e amarra o desenho ao resto do painel —
   * porque é a mesma cor de quem está dirigindo.
   */
  const contorno = new DirectionalLight(new Color(acentoInicial), 2.6);
  contorno.position.set(-6, 1.6, -4);
  cena.add(contorno);

  /* --- materiais --- */

  const pintura = new MeshStandardMaterial({
    color: 0x0e1116,
    metalness: 0.62,
    roughness: 0.32,
  });
  const vidro = new MeshStandardMaterial({
    color: 0x05070a,
    metalness: 0.1,
    roughness: 0.06,
    transparent: true,
    opacity: 0.62,
    side: DoubleSide,
  });
  const borracha = new MeshStandardMaterial({
    color: 0x08090b,
    metalness: 0.0,
    roughness: 0.92,
  });
  const roda = new MeshStandardMaterial({
    color: 0x2a2f38,
    metalness: 0.85,
    roughness: 0.28,
  });
  const aceso = new MeshBasicMaterial({ color: 0xffcf7a });
  const brasa = new MeshBasicMaterial({ color: 0xff4d4d });

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

  const lata = new ExtrudeGeometry(perfilDaCarroceria(), {
    depth: CARRO.meiaLargura * 2,
    bevelEnabled: true,
    // O chanfro é o que apaga a quina viva da extrusão e dá à lateral uma
    // superfície virando — sem ele não existe reflexo escorrendo, e sem reflexo
    // escorrendo não existe carro.
    bevelThickness: 0.075,
    bevelSize: 0.06,
    bevelSegments: 3,
    curveSegments: 14,
  });
  lata.translate(0, 0, -CARRO.meiaLargura);
  afunilar(lata.attributes.position.array as Float32Array, CARRO.meiaLargura, CARRO.teto);
  lata.computeVertexNormals();
  corpo.add(new Mesh(lata, pintura));

  const estufa = new ExtrudeGeometry(perfilDoVidro(), {
    depth: CARRO.meiaLargura * 2 * 0.84,
    bevelEnabled: true,
    bevelThickness: 0.03,
    bevelSize: 0.025,
    bevelSegments: 2,
    curveSegments: 12,
  });
  estufa.translate(0, 0, -CARRO.meiaLargura * 0.84);
  afunilar(
    estufa.attributes.position.array as Float32Array,
    CARRO.meiaLargura,
    CARRO.teto,
  );
  estufa.computeVertexNormals();
  corpo.add(new Mesh(estufa, vidro));

  // A asa sobre a rabeta — o traço que mais entrega o carro de longe.
  const asa = new Mesh(new BoxGeometry(0.5, 0.04, 1.1), pintura);
  asa.position.set(-1.94, 1.05, 0);
  corpo.add(asa);
  for (const z of [-0.42, 0.42]) {
    const pe = new Mesh(new BoxGeometry(0.06, 0.24, 0.045), pintura);
    pe.position.set(-1.92, 0.92, z);
    corpo.add(pe);
  }

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

  const pneuGeo = new CylinderGeometry(eixo.raio, eixo.raio, eixo.largura, 24, 1);
  pneuGeo.rotateX(Math.PI / 2);
  const aroGeo = new CylinderGeometry(
    eixo.raio * 0.6,
    eixo.raio * 0.6,
    eixo.largura * 1.04,
    20,
    1,
  );
  aroGeo.rotateX(Math.PI / 2);

  for (const x of [eixo.traseiro, eixo.dianteiro]) {
    for (const z of [-eixo.bitola, eixo.bitola]) {
      const conjunto = new Group();
      conjunto.position.set(x, eixo.altura, z);

      conjunto.add(new Mesh(pneuGeo, borracha));
      conjunto.add(new Mesh(aroGeo, roda));

      // Cinco raios, como no desenho de sempre. São eles que mostram o giro —
      // um aro liso girando é indistinguível de um aro parado.
      //
      // O deslocamento vai na GEOMETRIA e não na posição da malha: objeto gira
      // em volta da própria origem, então um raio posicionado e depois girado
      // rodopiaria em torno de si mesmo em vez de abrir o leque a partir do
      // centro da roda.
      for (let i = 0; i < 5; i++) {
        const g = new BoxGeometry(eixo.raio * 0.92, 0.05, eixo.largura * 1.06);
        g.translate(eixo.raio * 0.46, 0, 0);
        const raio = new Mesh(g, roda);
        raio.rotation.z = (i * Math.PI * 2) / 5;
        conjunto.add(raio);
      }

      carro.add(conjunto);
      rodas.push(conjunto);
    }
  }

  /* --- chão --- */

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
  sombra.position.y = 0.004;
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
      for (const mat of [pintura, vidro, borracha, roda, aceso, brasa]) mat.dispose();
      cena.environment?.dispose();
      renderer.dispose();
    },
  };
}
