//! O que é do carro, e não de quem dirige.
//!
//! Tamanho do tanque, cilindrada e o fator de calibração do consumo não moram no
//! perfil de propósito: trocar de motorista não muda o tanque. São dois arquivos no
//! diretório de dados do app, gravados como os perfis — temporário mais `rename`,
//! porque uma head unit perde energia no meio de uma gravação sempre que a ignição
//! desliga.
//!
//! São dois e não um porque mudam em ritmos diferentes: [`Veiculo`] muda quando o
//! dono ajusta algo (raro, e sempre por um toque), e [`EstadoTanque`] muda a cada
//! leitura do barramento. Juntos, cada ajuste reescreveria o estado vivo e cada
//! gota de gasolina reescreveria a configuração.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A configuração do carro.
///
/// Os padrões são os do Mitsubishi Eclipse GT 2000 — o carro para o qual este painel
/// foi escrito. Quem instalar em outro carro ajusta na tela; ninguém precisa
/// configurar nada para o painel funcionar no dia da instalação.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Veiculo {
    /// Tanque cheio, em litros. 16 galões no Eclipse GT.
    pub capacidade_l: f32,
    /// Cilindrada, em litros. Só é usada quando o consumo é estimado sem MAF.
    pub cilindrada_l: f32,
    /// Eficiência volumétrica média, para a estimativa por pressão do coletor.
    ///
    /// É a maior fonte de erro do método sem MAF: varia com rotação e carga, e aqui
    /// é uma constante. O fator de calibração é quem paga essa conta.
    pub ve_media: f32,
    /// Proporção ar/combustível estequiométrica. Gasolina C premium (~27% de
    /// etanol) fica perto de 13,2:1 — gasolina pura seria 14,7.
    pub afr: f32,
    /// Densidade do combustível, em g/L.
    pub densidade_g_l: f32,
    /// Correção do consumo calculado, aferida contra a bomba. 1,0 = sem correção.
    pub calibracao: f32,
    /// O km/l assumido para estimar autonomia antes de existir histórico.
    pub km_por_litro_padrao: f32,
}

impl Default for Veiculo {
    fn default() -> Self {
        Self {
            capacidade_l: 61.0,
            cilindrada_l: 3.0,
            ve_media: 0.80,
            afr: 13.2,
            densidade_g_l: 750.0,
            calibracao: 1.0,
            km_por_litro_padrao: 9.0,
        }
    }
}

impl Veiculo {
    /// Mantém a configuração dentro do que é fisicamente plausível.
    ///
    /// Chamado ao carregar do disco e ao aplicar um ajuste: um arquivo editado à mão
    /// com `capacidadeL: 0` faria a autonomia virar `NaN` e o painel mentir.
    pub fn saneado(mut self) -> Self {
        let padrao = Self::default();
        self.capacidade_l = clamp_ou(self.capacidade_l, 5.0, 200.0, padrao.capacidade_l);
        self.cilindrada_l = clamp_ou(self.cilindrada_l, 0.5, 10.0, padrao.cilindrada_l);
        self.ve_media = clamp_ou(self.ve_media, 0.3, 1.2, padrao.ve_media);
        self.afr = clamp_ou(self.afr, 6.0, 20.0, padrao.afr);
        self.densidade_g_l = clamp_ou(self.densidade_g_l, 600.0, 1000.0, padrao.densidade_g_l);
        self.calibracao = clamp_ou(self.calibracao, 0.5, 2.0, padrao.calibracao);
        self.km_por_litro_padrao = clamp_ou(
            self.km_por_litro_padrao,
            1.0,
            40.0,
            padrao.km_por_litro_padrao,
        );
        self
    }
}

/// Um trecho percorrido: quantos km e quantos litros.
///
/// Serve tanto para a viagem (que o motorista zera quando quer) quanto para o tanque
/// (que zera ao abastecer). A média sai da divisão, e é `None` enquanto não houver
/// litro suficiente para a divisão significar algo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trecho {
    pub km: f32,
    pub litros: f32,
    /// Tempo de motor ligado, em segundos.
    pub segundos: f32,
}

impl Trecho {
    pub fn somar(&mut self, km: f32, litros: f32, segundos: f32) {
        self.km += km.max(0.0);
        self.litros += litros.max(0.0);
        self.segundos += segundos.max(0.0);
    }

    /// km/l do trecho.
    ///
    /// Só a partir de 0,1 L: antes disso o denominador é pequeno demais e a média
    /// pularia de 40 para 4 km/l entre duas leituras, o que não é informação.
    pub fn km_por_litro(self) -> Option<f32> {
        (self.litros >= 0.1 && self.km > 0.0).then(|| self.km / self.litros)
    }
}

/// O que precisa sobreviver a desligar o carro.
///
/// O supervisor reconstrói o módulo do OBD a cada reconexão do adaptador, então sem
/// isto um tranco no conector zeraria o tanque e a viagem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstadoTanque {
    /// Litros na estimativa. `None` = ainda não se sabe (nem leitura, nem tanque cheio).
    pub litros: Option<f32>,
    /// Desde o último abastecimento.
    pub tanque: Trecho,
    /// Desde o último "zerar viagem".
    pub viagem: Trecho,
}

/// Um arquivo JSON no diretório de dados do app.
///
/// Genérico porque o veículo e o estado do tanque querem exatamente o mesmo
/// tratamento: tolerar ausência, quarentenar corrompido, gravar atômico.
pub struct Arquivo<T> {
    path: PathBuf,
    pub dados: T,
}

impl<T: Default + Serialize + for<'de> Deserialize<'de>> Arquivo<T> {
    /// Carrega, tolerando arquivo ausente ou corrompido.
    ///
    /// Arquivo quebrado vai para `.corrompido` e o app começa do padrão: perder o
    /// estado do tanque é chato, não abrir o painel é pior — e o arquivo velho fica
    /// lá para olhar depois.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();

        let dados = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(dados) => dados,
                Err(err) => {
                    tracing::error!(?path, %err, "arquivo do carro corrompido, usando o padrão");
                    let _ = fs::rename(&path, path.with_extension("json.corrompido"));
                    T::default()
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => T::default(),
            Err(err) => {
                tracing::error!(?path, %err, "não consegui ler o arquivo do carro");
                T::default()
            }
        };

        Self { path, dados }
    }

    /// Grava por arquivo temporário e `rename`, que é atômico no sistema de
    /// arquivos: ou o antigo continua inteiro, ou o novo aparece inteiro.
    pub fn salvar(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temporario = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.dados)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&temporario, bytes)?;
        fs::rename(&temporario, &self.path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn clamp_ou(valor: f32, min: f32, max: f32, padrao: f32) -> f32 {
    if valor.is_finite() && valor >= min && valor <= max {
        valor
    } else {
        padrao
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eclipse-obd-teste-{nome}"));
        let _ = fs::remove_dir_all(&dir);
        dir.join("arquivo.json")
    }

    #[test]
    fn o_padrao_e_o_eclipse_e_e_plausivel() {
        let v = Veiculo::default();
        assert_eq!(v.capacidade_l, 61.0, "16 galões");
        assert_eq!(
            v.saneado(),
            v,
            "o padrão não pode ser saneado para outra coisa"
        );
    }

    #[test]
    fn configuracao_absurda_volta_para_o_padrao() {
        let bicho = Veiculo {
            capacidade_l: 0.0,
            calibracao: f32::NAN,
            ..Veiculo::default()
        }
        .saneado();

        // Tanque zero faria a autonomia virar 0 km para sempre; calibração NaN
        // contaminaria todos os litros integrados dali para frente.
        assert_eq!(bicho.capacidade_l, 61.0);
        assert_eq!(bicho.calibracao, 1.0);
    }

    #[test]
    fn media_so_existe_depois_de_gastar_algo() {
        let mut t = Trecho::default();
        assert_eq!(t.km_por_litro(), None);

        t.somar(1.0, 0.05, 60.0);
        assert_eq!(t.km_por_litro(), None, "0,05 L não dá média nenhuma");

        t.somar(9.0, 0.95, 600.0);
        assert_eq!(t.km_por_litro(), Some(10.0));
    }

    #[test]
    fn grava_morre_e_recarrega() {
        let path = temp("grava");
        let mut arq: Arquivo<EstadoTanque> = Arquivo::load(&path);
        arq.dados.litros = Some(42.5);
        arq.dados.tanque.somar(120.0, 12.0, 3600.0);
        arq.salvar().expect("gravar");

        // É o cenário do tranco no conector: o módulo reinicia inteiro e o tanque
        // não pode voltar a ser um mistério.
        let de_novo: Arquivo<EstadoTanque> = Arquivo::load(&path);
        assert_eq!(de_novo.dados.litros, Some(42.5));
        assert_eq!(de_novo.dados.tanque.km, 120.0);
    }

    #[test]
    fn arquivo_corrompido_nao_impede_o_boot() {
        let path = temp("corrompido");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ isto nao e json").unwrap();

        let arq: Arquivo<Veiculo> = Arquivo::load(&path);
        assert_eq!(arq.dados, Veiculo::default());
        assert!(
            path.with_extension("json.corrompido").exists(),
            "o arquivo quebrado fica de lado para recuperar à mão"
        );
    }
}
