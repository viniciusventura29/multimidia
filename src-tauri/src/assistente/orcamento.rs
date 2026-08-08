//! Quanto o assistente pode gastar.
//!
//! Ele fala sozinho, e cada fala é uma chamada paga. Sem teto, um bug de
//! detecção não vira um painel esquisito — vira uma fatura. Por isso o teto é
//! parte do desenho, e não um cuidado que se toma depois.
//!
//! **O contador é gravado em disco.** Numa head unit o app sobe junto com o
//! carro e morre junto: teto diário que vivesse só na memória seria zerado a
//! cada partida, ou seja, não seria teto nenhum.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::gatilho::Gatilho;

/// O que dá para ajustar sem recompilar, em `assistente.json`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub ligado: bool,
    pub chamadas_por_dia: u32,
    /// Gerar imagem custa bem mais que texto, então tem teto próprio.
    pub imagens_por_dia: u32,
    /// Validação estrita do esquema das ferramentas. Interruptor de emergência:
    /// se algum dia a API recusar a união de cartões, dá para desligar sem
    /// gerar APK novo.
    pub estrito: bool,
    /// O modelo de imagem no OpenRouter. Configurável porque o catálogo de lá
    /// muda mais rápido que o ciclo de gerar um APK novo.
    pub modelo_imagem: String,
    /// Servidores MCP remotos.
    pub mcp: Vec<ServidorMcp>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServidorMcp {
    pub nome: String,
    pub url: String,
    #[serde(default)]
    pub token: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ligado: true,
            // Chute deliberadamente conservador: dá uns dois dias normais de
            // uso. É para ser levantado depois de olhar a fatura, não antes.
            chamadas_por_dia: 40,
            imagens_por_dia: 4,
            estrito: true,
            modelo_imagem: "bytedance-seed/seedream-4.5".into(),
            mcp: Vec::new(),
        }
    }
}

impl Config {
    pub fn carregar(dir_dados: &Path) -> Self {
        let caminho = dir_dados.join("assistente.json");
        match fs::read(&caminho) {
            Err(_) => Self::default(),
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                // Mesma política dos perfis: arquivo quebrado não impede o app
                // de subir. Fica o padrão, e o original é preservado ao lado
                // para conserto à mão.
                tracing::error!(?caminho, %err, "assistente.json inválido, usando o padrão");
                let _ = fs::rename(&caminho, caminho.with_extension("json.corrompido"));
                Self::default()
            }),
        }
    }
}

/// O que já foi gasto. Gravado em `assistente_uso.json`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct Uso {
    dia: Option<NaiveDate>,
    chamadas: u32,
    imagens: u32,
    /// Última vez que cada gatilho falou. Precisa sobreviver ao desligamento:
    /// parar no posto e voltar não é ligar o carro pela primeira vez.
    ultima: HashMap<String, DateTime<Utc>>,
}

/// Por que uma fala foi barrada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recusa {
    Desligado,
    TetoDiario,
    Descansando,
}

pub struct Orcamento {
    caminho: PathBuf,
    config: Config,
    uso: Uso,
}

impl Orcamento {
    pub fn carregar(dir_dados: &Path, config: Config) -> Self {
        let caminho = dir_dados.join("assistente_uso.json");
        let uso = fs::read(&caminho)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        Self {
            caminho,
            config,
            uso,
        }
    }

    /// Zera os contadores quando vira o dia. O dia é o local, não o UTC:
    /// "40 chamadas por dia" tem que casar com o dia de quem dirige.
    fn virar_o_dia(&mut self, agora: DateTime<Utc>) {
        let hoje = agora.with_timezone(&Local).date_naive();
        if self.uso.dia != Some(hoje) {
            self.uso.dia = Some(hoje);
            self.uso.chamadas = 0;
            self.uso.imagens = 0;
        }
    }

    pub fn pode_falar(&mut self, gatilho: Gatilho, agora: DateTime<Utc>) -> Result<(), Recusa> {
        if !self.config.ligado {
            return Err(Recusa::Desligado);
        }

        self.virar_o_dia(agora);

        if self.uso.chamadas >= self.config.chamadas_por_dia {
            return Err(Recusa::TetoDiario);
        }

        if let Some(ultima) = self.uso.ultima.get(gatilho.como_texto()) {
            if agora - *ultima < Duration::minutes(gatilho.descanso_min()) {
                return Err(Recusa::Descansando);
            }
        }

        Ok(())
    }

    /// Registra que o gatilho falou. Chame **antes** da chamada à API, não
    /// depois: se a resposta demorar 40 segundos e outro gatilho disparar no
    /// meio, os dois passariam pelo teto ao mesmo tempo.
    pub fn registrar_fala(&mut self, gatilho: Gatilho, agora: DateTime<Utc>) {
        self.virar_o_dia(agora);
        self.uso.chamadas += 1;
        self.uso
            .ultima
            .insert(gatilho.como_texto().to_string(), agora);
        self.gravar();
    }

    pub fn pode_gerar_imagem(&mut self, agora: DateTime<Utc>) -> bool {
        self.virar_o_dia(agora);
        self.uso.imagens < self.config.imagens_por_dia
    }

    pub fn registrar_imagem(&mut self, agora: DateTime<Utc>) {
        self.virar_o_dia(agora);
        self.uso.imagens += 1;
        self.gravar();
    }

    /// Quantas chamadas já saíram **hoje**.
    ///
    /// Recebe o relógio em vez de olhar o contador cru: virada a meia-noite, o
    /// número gravado ainda é o de ontem até alguém chamar `pode_falar`, e o log
    /// diria "39/40" num dia que começou zerado.
    pub fn chamadas_hoje(&self, agora: DateTime<Utc>) -> u32 {
        let hoje = agora.with_timezone(&Local).date_naive();
        if self.uso.dia == Some(hoje) {
            self.uso.chamadas
        } else {
            0
        }
    }

    /// Grava com arquivo temporário e `rename`, como o cofre de perfis: um corte
    /// de ignição no meio da gravação deixaria um JSON pela metade, e o
    /// orçamento nasceria zerado no próximo boot.
    fn gravar(&self) {
        let Ok(json) = serde_json::to_vec_pretty(&self.uso) else {
            return;
        };
        let temporario = self.caminho.with_extension("json.tmp");

        if let Err(err) =
            fs::write(&temporario, &json).and_then(|_| fs::rename(&temporario, &self.caminho))
        {
            tracing::warn!(%err, "não consegui gravar o uso do assistente");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "eclipse-orcamento-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn quando(dia: u32, hora: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, dia, hora, 0, 0).unwrap()
    }

    #[test]
    fn teto_diario_barra_e_o_dia_seguinte_libera() {
        let d = dir();
        let mut o = Orcamento::carregar(
            &d,
            Config {
                chamadas_por_dia: 2,
                ..Config::default()
            },
        );

        // Gatilhos diferentes para o descanso não atrapalhar o teste do teto.
        o.registrar_fala(Gatilho::Ignicao, quando(10, 8));
        o.registrar_fala(Gatilho::RotaDefinida, quando(10, 9));

        assert_eq!(
            o.pode_falar(Gatilho::Chegada, quando(10, 10)),
            Err(Recusa::TetoDiario)
        );
        assert!(o.pode_falar(Gatilho::Chegada, quando(11, 10)).is_ok());
    }

    #[test]
    fn descanso_barra_o_mesmo_gatilho_e_libera_o_outro() {
        let d = dir();
        let mut o = Orcamento::carregar(&d, Config::default());

        o.registrar_fala(Gatilho::Periodico, quando(10, 8));

        // Periódico descansa 20 minutos.
        assert_eq!(
            o.pode_falar(Gatilho::Periodico, quando(10, 8) + Duration::minutes(5)),
            Err(Recusa::Descansando)
        );
        assert!(o
            .pode_falar(Gatilho::Periodico, quando(10, 8) + Duration::minutes(21))
            .is_ok());
        assert!(
            o.pode_falar(Gatilho::AlertaCarro, quando(10, 8)).is_ok(),
            "o descanso é por gatilho, não geral"
        );
    }

    /// O motivo de o contador ir para o disco: numa head unit o app morre com o
    /// carro. Parar no posto e voltar não pode virar uma saudação nova.
    #[test]
    fn descanso_e_contagem_sobrevivem_ao_desligamento() {
        let d = dir();

        {
            let mut o = Orcamento::carregar(&d, Config::default());
            o.registrar_fala(Gatilho::Ignicao, quando(10, 8));
        }

        // O app subiu de novo — instância nova, mesmo diretório.
        let mut depois = Orcamento::carregar(&d, Config::default());
        assert_eq!(depois.chamadas_hoje(quando(10, 8)), 1);
        assert_eq!(
            depois.pode_falar(Gatilho::Ignicao, quando(10, 8) + Duration::minutes(15)),
            Err(Recusa::Descansando),
            "voltar do posto não é ligar o carro pela primeira vez"
        );
        assert!(depois
            .pode_falar(Gatilho::Ignicao, quando(10, 8) + Duration::hours(7))
            .is_ok());
    }

    #[test]
    fn desligado_no_arquivo_barra_tudo() {
        let d = dir();
        let mut o = Orcamento::carregar(
            &d,
            Config {
                ligado: false,
                ..Config::default()
            },
        );
        assert_eq!(
            o.pode_falar(Gatilho::Ignicao, quando(10, 8)),
            Err(Recusa::Desligado)
        );
    }

    #[test]
    fn imagem_tem_teto_proprio() {
        let d = dir();
        let mut o = Orcamento::carregar(
            &d,
            Config {
                imagens_por_dia: 1,
                ..Config::default()
            },
        );

        assert!(o.pode_gerar_imagem(quando(10, 8)));
        o.registrar_imagem(quando(10, 8));
        assert!(!o.pode_gerar_imagem(quando(10, 9)));

        assert!(
            o.pode_falar(Gatilho::Ignicao, quando(10, 9)).is_ok(),
            "estourar o teto de imagem não pode calar o assistente"
        );
    }

    #[test]
    fn config_ausente_vira_padrao_e_config_parcial_completa_o_resto() {
        let d = dir();
        assert!(Config::carregar(&d).ligado);

        fs::write(d.join("assistente.json"), br#"{ "chamadasPorDia": 5 }"#).unwrap();
        let c = Config::carregar(&d);
        assert_eq!(c.chamadas_por_dia, 5);
        assert_eq!(c.imagens_por_dia, 4, "o resto continua no padrão");
        assert!(c.estrito);
    }

    #[test]
    fn config_corrompida_nao_impede_o_app_de_subir() {
        let d = dir();
        fs::write(d.join("assistente.json"), b"{ isto nao e json").unwrap();

        let c = Config::carregar(&d);
        assert_eq!(c.chamadas_por_dia, Config::default().chamadas_por_dia);
        assert!(
            d.join("assistente.json.corrompido").exists(),
            "o arquivo quebrado tem que ficar guardado para conserto"
        );
    }

    #[test]
    fn uso_corrompido_comeca_do_zero_em_vez_de_estourar() {
        let d = dir();
        fs::write(d.join("assistente_uso.json"), b"lixo").unwrap();

        let mut o = Orcamento::carregar(&d, Config::default());
        assert_eq!(o.chamadas_hoje(quando(10, 8)), 0);
        assert!(o.pode_falar(Gatilho::Ignicao, quando(10, 8)).is_ok());
    }
}
