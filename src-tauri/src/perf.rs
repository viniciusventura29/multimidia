//! Medição de performance — **TEMPORÁRIA**.
//!
//! # Como arrancar isto depois
//!
//! Este arquivo inteiro é descartável. Para removê-lo:
//!
//! 1. apague `src-tauri/src/perf.rs` e a linha `mod perf;` do `lib.rs`;
//! 2. `rg "perf::"` e apague cada chamada (são poucas e todas de uma linha);
//! 3. apague `src/core/perf.ts` e volte os `invoke` para
//!    `@tauri-apps/api/core`.
//!
//! Nada mais depende daqui: nenhum estado de módulo, nenhuma tela.
//!
//! # Por que resumo, e não uma linha por evento
//!
//! O dono pediu "log de tudo". Tudo, literalmente, se autodestrói: um
//! `tracing::warn!` grava em disco **de forma síncrona na thread que chamou**
//! (ver `Diario::anotar`) — serializa até 51 linhas, faz `create_dir_all` e um
//! `metadata` a cada gravação. E o servidor recusa lotes acima de 500 linhas
//! com HTTP 413. Medir a 30 Hz desse jeito tornaria a medição o gargalo, e
//! estaríamos cronometrando o cronômetro.
//!
//! Então mede-se **tudo**, o tempo todo, mas em memória; e a cada 30 s sai UM
//! marco por grandeza, com mediana, p95, pior caso e número de amostras. Para
//! achar gargalo isso é melhor que o detalhe: mostra padrão em vez de um caso
//! solto, e o pior caso continua lá, que é o que trava a tela.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// De quanto em quanto tempo os resumos sobem.
///
/// Trinta segundos é o mesmo passo do envio do diário: o marco fica pronto
/// pouco antes do lote sair, então ele viaja no envio seguinte sem pedir um
/// POST só para ele.
const JANELA: Duration = Duration::from_secs(30);

/// Teto de amostras guardadas por grandeza numa janela.
///
/// A mediana e o p95 precisam da lista ordenada, então a lista existe. 4096 a
/// 30 Hz dá mais de dois minutos de folga — muito além da janela — e custa
/// 32 KB por grandeza no pior caso. Estourando, as novas amostras são
/// descartadas em vez de crescer sem limite: perder amostra é melhor que
/// inchar a memória de uma head unit de 6 GB que já roda um mapa vetorial.
const TETO_AMOSTRAS: usize = 4096;

#[derive(Default)]
struct Serie {
    amostras: Vec<u64>,
    /// Quantas foram descartadas por teto. Vai no resumo: um número aqui
    /// significa que a mediana está calculada sobre um recorte, e quem lê
    /// precisa saber disso.
    descartadas: u64,
}

struct Acumulador {
    series: BTreeMap<&'static str, Serie>,
    abriu: Instant,
}

static ACUMULADOR: LazyLock<Mutex<Acumulador>> = LazyLock::new(|| {
    Mutex::new(Acumulador {
        series: BTreeMap::new(),
        abriu: Instant::now(),
    })
});

/// Guarda uma medição. Barato de propósito: um lock e um push.
///
/// O nome é `&'static str` para não alocar no caminho quente — são todos
/// literais no código.
pub fn medir(nome: &'static str, levou: Duration) {
    anotar_ms(nome, levou.as_millis() as u64);
}

/// Idem, para quem já tem o número (o front manda milissegundos prontos).
pub fn anotar_ms(nome: &'static str, ms: u64) {
    let Ok(mut acc) = ACUMULADOR.lock() else {
        // Mutex envenenado por um pânico em outra thread. Medição não é motivo
        // para derrubar mais nada — desiste desta amostra e segue.
        return;
    };
    let serie = acc.series.entry(nome).or_default();
    if serie.amostras.len() >= TETO_AMOSTRAS {
        serie.descartadas += 1;
        return;
    }
    serie.amostras.push(ms);
}

/// Cronômetro de escopo: mede do `novo` até sair de escopo.
///
/// Serve para os pontos com vários caminhos de saída (`?`, `return`, `break`),
/// onde cronometrar na mão erra justamente no caminho de erro — que costuma
/// ser o lento.
pub struct Cronometro {
    nome: &'static str,
    comecou: Instant,
}

impl Cronometro {
    pub fn novo(nome: &'static str) -> Self {
        Self {
            nome,
            comecou: Instant::now(),
        }
    }
}

impl Drop for Cronometro {
    fn drop(&mut self) {
        medir(self.nome, self.comecou.elapsed());
    }
}

/// Fecha a janela e devolve os resumos, se já deu o tempo.
///
/// Devolve `None` antes da hora para o chamador poder chamar à vontade de
/// dentro de um laço sem pensar no relógio.
fn fechar_janela() -> Option<Vec<(&'static str, Resumo)>> {
    let mut acc = ACUMULADOR.lock().ok()?;
    if acc.abriu.elapsed() < JANELA {
        return None;
    }
    acc.abriu = Instant::now();

    let series = std::mem::take(&mut acc.series);
    // Solta o lock antes de ordenar: quem está medindo não precisa esperar a
    // estatística.
    drop(acc);

    let mut fora = Vec::new();
    for (nome, mut serie) in series {
        if serie.amostras.is_empty() {
            continue;
        }
        serie.amostras.sort_unstable();
        fora.push((nome, Resumo::de(&serie.amostras, serie.descartadas)));
    }
    Some(fora)
}

#[derive(Debug, PartialEq)]
struct Resumo {
    n: u64,
    p50: u64,
    p95: u64,
    pior: u64,
    descartadas: u64,
}

impl Resumo {
    /// `ordenadas` precisa estar ordenada e não-vazia.
    fn de(ordenadas: &[u64], descartadas: u64) -> Self {
        Self {
            n: ordenadas.len() as u64,
            p50: percentil(ordenadas, 50),
            p95: percentil(ordenadas, 95),
            // O pior caso é o número que importa para "travou": a mediana pode
            // estar ótima e a tela ainda engasgar uma vez por segundo.
            pior: *ordenadas.last().expect("não-vazia por contrato"),
            descartadas,
        }
    }
}

/// Percentil pelo método do vizinho mais próximo, sem interpolar.
///
/// Interpolar inventaria um valor que nenhuma volta do laço realmente levou —
/// e aqui o que interessa é "existiu um quadro que custou isso", não uma
/// estatística bonita.
fn percentil(ordenadas: &[u64], p: u64) -> u64 {
    debug_assert!(!ordenadas.is_empty());
    let n = ordenadas.len();
    // Regra do vizinho mais próximo: posição = teto(p/100 * n), 1-indexada.
    //
    // O `n`, e NÃO `n - 1`: com `n - 1` o p95 de 1..100 dá 96, um degrau acima
    // do que todo mundo chama de p95. O `.max(1)` é para p = 0 não estourar
    // por baixo ao subtrair — e o `.min` fecha o outro lado.
    let posicao = ((n as u64 * p).div_ceil(100).max(1) as usize).min(n);
    ordenadas[posicao - 1]
}

/// Liga o relógio que publica os resumos.
///
/// Relógio PRÓPRIO, e não carona no laço de algum módulo. A primeira versão
/// disto pendurou a publicação no laço do OBD — e o laço do OBD só gira se o
/// adaptador conectar. Numa sessão sem adaptador (ou com ele fora do carro,
/// que é como se testa mapa em casa) nenhum resumo subiria, justamente quando
/// o que se quer medir é a tela.
pub fn ligar_relogio() {
    tauri::async_runtime::spawn(async {
        // Bem mais curto que a JANELA de propósito: assim a janela fecha
        // perto dos 30 s de verdade, em vez de virar 60 no pior caso.
        let mut passo = tokio::time::interval(Duration::from_secs(5));
        loop {
            passo.tick().await;
            talvez_publicar();
        }
    });
}

/// Se a janela fechou, manda os resumos para o diário.
///
/// Chamar de qualquer lugar e com qualquer frequência: antes da hora não faz
/// nada.
pub fn talvez_publicar() {
    let Some(resumos) = fechar_janela() else {
        return;
    };
    let Some(diario) = crate::diario::atual() else {
        return;
    };

    for (nome, r) in resumos {
        // `marco` e não `warn!`: precisa chegar ao servidor numa sessão em que
        // NADA deu errado — que é justamente a sessão que queremos medir.
        let mut linha = crate::diario::Linha::nova(
            crate::diario::Nivel::Info,
            "perf",
            format!("tempos de {nome}"),
        );
        linha.dados.insert("n".into(), r.n.into());
        linha.dados.insert("p50_ms".into(), r.p50.into());
        linha.dados.insert("p95_ms".into(), r.p95.into());
        linha.dados.insert("pior_ms".into(), r.pior.into());
        if r.descartadas > 0 {
            linha
                .dados
                .insert("descartadas".into(), r.descartadas.into());
        }
        diario.marco(linha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_pior_caso_nao_some_no_meio_da_mediana() {
        // O caso que este módulo existe para pegar: 99 quadros ótimos e um
        // horrível. A mediana diz "está tudo bem" e a tela engasga do mesmo
        // jeito — é o `pior` que denuncia.
        let mut v: Vec<u64> = vec![10; 99];
        v.push(900);
        v.sort_unstable();
        let r = Resumo::de(&v, 0);
        assert_eq!(r.p50, 10, "a mediana continua ótima");
        assert_eq!(r.pior, 900, "e o engasgo aparece mesmo assim");
        assert_eq!(r.n, 100);
    }

    #[test]
    fn o_p95_pega_a_cauda_e_nao_o_extremo() {
        let v: Vec<u64> = (1..=100).collect();
        let r = Resumo::de(&v, 0);
        assert_eq!(r.p95, 95);
        assert_eq!(r.pior, 100);
    }

    #[test]
    fn uma_amostra_so_nao_estoura() {
        // Um módulo que rodou uma volta e caiu ainda precisa reportar.
        let r = Resumo::de(&[42], 0);
        assert_eq!((r.n, r.p50, r.p95, r.pior), (1, 42, 42, 42));
    }

    #[test]
    fn o_percentil_nunca_sai_do_vetor() {
        // Proteção contra o clássico off-by-one de percentil: com qualquer
        // tamanho e qualquer p, o índice tem que existir.
        for tamanho in 1..50usize {
            let v: Vec<u64> = (0..tamanho as u64).collect();
            for p in [0, 1, 50, 95, 99, 100] {
                let val = percentil(&v, p);
                assert!(
                    val < tamanho as u64,
                    "p{p} de {tamanho} amostras saiu do vetor"
                );
            }
        }
    }

    #[test]
    fn o_teto_descarta_em_vez_de_crescer_sem_fim() {
        // Numa head unit, medição que incha memória vira o problema que ela
        // veio medir.
        let mut serie = Serie::default();
        for i in 0..(TETO_AMOSTRAS as u64 + 10) {
            if serie.amostras.len() >= TETO_AMOSTRAS {
                serie.descartadas += 1;
            } else {
                serie.amostras.push(i);
            }
        }
        assert_eq!(serie.amostras.len(), TETO_AMOSTRAS);
        assert_eq!(serie.descartadas, 10);
    }
}

/// O painel manda uma medição.
///
/// TEMPORÁRIO — ver o topo deste arquivo.
///
/// O nome vem como `String` e precisa virar `&'static str` para entrar no
/// acumulador. Em vez de vazar memória com `Box::leak` a cada chamada — o que
/// numa tela a 30 Hz seria um vazamento de verdade —, só nomes de uma lista
/// conhecida são aceitos. Nome desconhecido é descartado em silêncio: é a
/// tela que está errada, e derrubar a medição por isso seria pior.
#[tauri::command]
pub fn medir_do_painel(nome: String, ms: f64) {
    // `ms` chega como `f64` porque o `performance.now()` é fracionário.
    // Negativo não existe; `as u64` de um negativo daria um número gigante.
    let ms = ms.max(0.0).round() as u64;

    const CONHECIDOS: &[&str] = &[
        "tela.quadro",
        // Os comandos que a tela realmente chama — conferido com um grep em
        // `src/`, não de cabeça.
        "ipc.active_profile",
        "ipc.baixar_atualizacao",
        "ipc.checar_atualizacao",
        "ipc.connect_spotify",
        "ipc.dispatch_action",
        "ipc.get_snapshot",
        "ipc.imagem_ia",
        "ipc.list_profiles",
        "ipc.push_location",
        "ipc.push_location_error",
        "ipc.spotify_access_token",
        "ipc.versao_rodando",
    ];
    if let Some(conhecido) = CONHECIDOS.iter().find(|c| **c == nome) {
        anotar_ms(conhecido, ms);
    }
}
