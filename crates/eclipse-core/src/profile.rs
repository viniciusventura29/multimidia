//! Perfis: quem está dirigindo, e o que muda por causa disso.
//!
//! Vale registrar o que um perfil **não** é. Só o Spotify troca de conta de
//! verdade por perfil (cada um com seu refresh token, na Fase 5). WhatsApp é uma
//! conta por aparelho e o mapa embutido não fica logado numa conta Google — para
//! esses, perfil é preferência local.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub units: Units,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: Uuid,
    pub name: String,
    /// Cor de destaque, em hex. É o sinal mais rápido de quem está dirigindo.
    pub color: String,
    #[serde(default)]
    pub preferences: Preferences,
}

impl Profile {
    pub fn new(name: impl Into<String>, color: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            color: color.into(),
            preferences: Preferences::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("perfil não encontrado")]
    NotFound,
    #[error("falha ao gravar perfis: {0}")]
    Io(#[from] io::Error),
    #[error("falha ao serializar perfis: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Disco {
    profiles: Vec<Profile>,
    active: Option<Uuid>,
}

/// Os perfis em disco.
pub struct ProfileStore {
    path: PathBuf,
    dados: Disco,
}

impl ProfileStore {
    /// Carrega os perfis, tolerando arquivo ausente ou corrompido.
    ///
    /// Um painel de carro perde energia no meio de uma gravação, então o arquivo
    /// pode chegar quebrado. Nesse caso ele é movido para `.corrompido` e o app
    /// começa vazio: perder os perfis é ruim, mas não abrir é pior — e o arquivo
    /// velho fica lá para recuperar à mão.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();

        let dados = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(dados) => dados,
                Err(err) => {
                    tracing::error!(?path, %err, "perfis corrompidos, começando vazio");
                    let _ = fs::rename(&path, path.with_extension("json.corrompido"));
                    Disco::default()
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => Disco::default(),
            Err(err) => {
                tracing::error!(?path, %err, "não consegui ler os perfis");
                Disco::default()
            }
        };

        Self { path, dados }
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.dados.profiles
    }

    pub fn active(&self) -> Option<&Profile> {
        let id = self.dados.active?;
        self.dados.profiles.iter().find(|p| p.id == id)
    }

    pub fn create(
        &mut self,
        name: impl Into<String>,
        color: impl Into<String>,
    ) -> Result<Profile, ProfileError> {
        let profile = Profile::new(name, color);
        self.dados.profiles.push(profile.clone());

        // O primeiro perfil criado já entra como ativo: ninguém quer criar um
        // perfil e ainda ter que escolhê-lo.
        if self.dados.active.is_none() {
            self.dados.active = Some(profile.id);
        }

        self.save()?;
        Ok(profile)
    }

    pub fn select(&mut self, id: Uuid) -> Result<Profile, ProfileError> {
        let profile = self
            .dados
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or(ProfileError::NotFound)?;

        self.dados.active = Some(id);
        self.save()?;
        Ok(profile)
    }

    pub fn remove(&mut self, id: Uuid) -> Result<(), ProfileError> {
        let antes = self.dados.profiles.len();
        self.dados.profiles.retain(|p| p.id != id);

        if self.dados.profiles.len() == antes {
            return Err(ProfileError::NotFound);
        }

        if self.dados.active == Some(id) {
            self.dados.active = self.dados.profiles.first().map(|p| p.id);
        }

        self.save()
    }

    /// Grava por arquivo temporário e `rename`.
    ///
    /// `rename` é atômico no sistema de arquivos: ou o arquivo antigo continua
    /// inteiro, ou o novo aparece inteiro. Escrever por cima direto deixaria uma
    /// janela em que cortar a ignição corromperia os perfis.
    fn save(&self) -> Result<(), ProfileError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }

        let temporario = self.path.with_extension("json.tmp");
        fs::write(&temporario, serde_json::to_vec_pretty(&self.dados)?)?;
        fs::rename(&temporario, &self.path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Diretório temporário próprio, para não trazer uma dependência só de teste.
    fn caminho_temporario() -> PathBuf {
        std::env::temp_dir()
            .join(format!("eclipse-perfis-{}", Uuid::new_v4()))
            .join("profiles.json")
    }

    #[test]
    fn cria_persiste_e_recarrega() {
        let caminho = caminho_temporario();

        let criado = {
            let mut store = ProfileStore::load(&caminho);
            assert!(store.profiles().is_empty());
            store.create("Vinicius", "#3ddc97").unwrap()
        };

        let store = ProfileStore::load(&caminho);
        assert_eq!(store.profiles(), std::slice::from_ref(&criado));
        assert_eq!(
            store.active(),
            Some(&criado),
            "o primeiro perfil criado já entra como ativo"
        );

        let _ = fs::remove_dir_all(caminho.parent().unwrap());
    }

    #[test]
    fn troca_de_perfil_sobrevive_ao_reinicio() {
        let caminho = caminho_temporario();
        let mut store = ProfileStore::load(&caminho);

        store.create("Vinicius", "#3ddc97").unwrap();
        let segundo = store.create("Convidado", "#f5a524").unwrap();
        store.select(segundo.id).unwrap();

        let recarregado = ProfileStore::load(&caminho);
        assert_eq!(recarregado.active().map(|p| p.id), Some(segundo.id));

        let _ = fs::remove_dir_all(caminho.parent().unwrap());
    }

    #[test]
    fn selecionar_perfil_inexistente_falha() {
        let mut store = ProfileStore::load(caminho_temporario());
        assert!(matches!(
            store.select(Uuid::new_v4()),
            Err(ProfileError::NotFound)
        ));
    }

    #[test]
    fn remover_o_ativo_promove_outro() {
        let caminho = caminho_temporario();
        let mut store = ProfileStore::load(&caminho);

        let primeiro = store.create("Vinicius", "#3ddc97").unwrap();
        let segundo = store.create("Convidado", "#f5a524").unwrap();

        store.select(segundo.id).unwrap();
        store.remove(segundo.id).unwrap();

        assert_eq!(
            store.active().map(|p| p.id),
            Some(primeiro.id),
            "não pode ficar sem perfil ativo enquanto houver perfis"
        );

        let _ = fs::remove_dir_all(caminho.parent().unwrap());
    }

    /// Cortar a ignição no meio de uma gravação não pode impedir o painel de abrir.
    #[test]
    fn arquivo_corrompido_nao_impede_o_boot() {
        let caminho = caminho_temporario();
        fs::create_dir_all(caminho.parent().unwrap()).unwrap();
        fs::write(&caminho, b"{ isto nao e json").unwrap();

        let store = ProfileStore::load(&caminho);
        assert!(store.profiles().is_empty());
        assert!(
            caminho.with_extension("json.corrompido").exists(),
            "o arquivo quebrado precisa ser preservado para recuperação"
        );

        let _ = fs::remove_dir_all(caminho.parent().unwrap());
    }
}
