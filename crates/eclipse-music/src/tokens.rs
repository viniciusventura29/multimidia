//! O cofre de refresh tokens, um por perfil.
//!
//! Duas regras do Spotify moldam este arquivo inteiro, e errar qualquer uma
//! desloga o usuário sem explicação semanas depois:
//!
//! 1. **O refresh token expira em 6 meses**, contados da autorização original.
//!    Renovar o access token não estende o prazo.
//! 2. **No fluxo PKCE o refresh token é rotacionado**: cada renovação pode
//!    devolver um novo, e o anterior morre. Mas a resposta nem sempre traz um —
//!    e o rspotify desserializa a resposta crua, entregando `None` nesse caso.
//!    Persistir esse `None` apagaria a sessão.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Validade do refresh token. O Spotify fala em 6 meses; 180 dias avisa um
/// pouco antes, que é o lado certo para errar.
const VALIDADE_DIAS: i64 = 180;

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("falha ao gravar os tokens: {0}")]
    Io(#[from] io::Error),
    #[error("falha ao serializar os tokens: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredToken {
    pub refresh_token: String,
    /// Quando o usuário autorizou pela primeira vez.
    ///
    /// É deste instante que os 6 meses correm. Guardar aqui a data da última
    /// renovação daria uma falsa sensação de sessão eterna e faria o app ser
    /// pego de surpresa por um `invalid_grant`.
    pub authorized_at: DateTime<Utc>,
}

impl StoredToken {
    pub fn expira_em(&self) -> DateTime<Utc> {
        self.authorized_at + Duration::days(VALIDADE_DIAS)
    }

    pub fn venceu(&self, agora: DateTime<Utc>) -> bool {
        agora >= self.expira_em()
    }

    pub fn dias_restantes(&self, agora: DateTime<Utc>) -> i64 {
        (self.expira_em() - agora).num_days()
    }
}

pub struct TokenStore {
    path: PathBuf,
    tokens: HashMap<Uuid, StoredToken>,
}

impl TokenStore {
    /// Carrega o cofre, tolerando arquivo ausente ou corrompido.
    ///
    /// Perder os tokens custa uma reautenticação; não abrir o painel custa a
    /// viagem inteira. O arquivo quebrado é preservado para inspeção.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();

        let tokens = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                tracing::error!(?path, %err, "cofre de tokens corrompido");
                let _ = fs::rename(&path, path.with_extension("json.corrompido"));
                HashMap::new()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(err) => {
                tracing::error!(?path, %err, "não consegui ler o cofre de tokens");
                HashMap::new()
            }
        };

        Self { path, tokens }
    }

    pub fn get(&self, perfil: Uuid) -> Option<&StoredToken> {
        self.tokens.get(&perfil)
    }

    /// Guarda o resultado de uma autorização nova, zerando o relógio dos 6 meses.
    pub fn autorizou(
        &mut self,
        perfil: Uuid,
        refresh_token: impl Into<String>,
        quando: DateTime<Utc>,
    ) -> Result<(), TokenError> {
        self.tokens.insert(
            perfil,
            StoredToken {
                refresh_token: refresh_token.into(),
                authorized_at: quando,
            },
        );
        self.save()
    }

    /// Registra o resultado de uma renovação.
    ///
    /// Chamado do `token_callback_fn` do rspotify, a cada renovação bem-sucedida.
    /// Duas coisas que precisam estar exatamente assim:
    ///
    /// - `None` **não apaga** o que está guardado. Uma renovação que não devolva
    ///   refresh token novo significa "continue usando o mesmo", não "esqueça".
    /// - A data de autorização **não** é atualizada. Os 6 meses correm desde o
    ///   login original, e mexer nisso esconderia o vencimento até ele estourar.
    pub fn renovou(&mut self, perfil: Uuid, novo: Option<&str>) -> Result<(), TokenError> {
        let Some(novo) = novo.filter(|t| !t.is_empty()) else {
            return Ok(());
        };

        let Some(guardado) = self.tokens.get_mut(&perfil) else {
            return Ok(());
        };

        if guardado.refresh_token == novo {
            return Ok(());
        }

        guardado.refresh_token = novo.to_string();
        self.save()
    }

    pub fn esqueceu(&mut self, perfil: Uuid) -> Result<(), TokenError> {
        if self.tokens.remove(&perfil).is_some() {
            return self.save();
        }
        Ok(())
    }

    /// Grava por temporário e `rename`, como os perfis, e restringe a permissão:
    /// é um segredo de longa duração num aparelho que fica dentro do carro.
    fn save(&self) -> Result<(), TokenError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }

        let temporario = self.path.with_extension("json.tmp");
        fs::write(&temporario, serde_json::to_vec_pretty(&self.tokens)?)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporario, fs::Permissions::from_mode(0o600))?;
        }

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

    fn caminho() -> PathBuf {
        std::env::temp_dir()
            .join(format!("eclipse-tokens-{}", Uuid::new_v4()))
            .join("tokens.json")
    }

    fn limpar(p: &Path) {
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn rotacao_substitui_o_token_e_mantem_a_data_de_autorizacao() {
        let p = caminho();
        let perfil = Uuid::new_v4();
        let autorizado_em = Utc::now() - Duration::days(30);

        let mut store = TokenStore::load(&p);
        store.autorizou(perfil, "refresh-1", autorizado_em).unwrap();
        store.renovou(perfil, Some("refresh-2")).unwrap();

        let guardado = store.get(perfil).unwrap();
        assert_eq!(guardado.refresh_token, "refresh-2");
        assert_eq!(
            guardado.authorized_at, autorizado_em,
            "renovar não pode reiniciar o relógio dos 6 meses"
        );

        limpar(&p);
    }

    /// O caso que desloga o usuário em silêncio. O rspotify entrega `None`
    /// quando a resposta do Spotify não traz refresh token novo.
    #[test]
    fn renovacao_sem_token_novo_nao_apaga_o_guardado() {
        let p = caminho();
        let perfil = Uuid::new_v4();

        let mut store = TokenStore::load(&p);
        store.autorizou(perfil, "refresh-1", Utc::now()).unwrap();
        store.renovou(perfil, None).unwrap();
        store.renovou(perfil, Some("")).unwrap();

        assert_eq!(
            store.get(perfil).unwrap().refresh_token,
            "refresh-1",
            "sem token novo, o antigo continua valendo"
        );

        limpar(&p);
    }

    #[test]
    fn a_rotacao_sobrevive_ao_reinicio() {
        let p = caminho();
        let perfil = Uuid::new_v4();

        {
            let mut store = TokenStore::load(&p);
            store.autorizou(perfil, "refresh-1", Utc::now()).unwrap();
            store.renovou(perfil, Some("refresh-2")).unwrap();
        }

        let store = TokenStore::load(&p);
        assert_eq!(store.get(perfil).unwrap().refresh_token, "refresh-2");

        limpar(&p);
    }

    #[test]
    fn os_seis_meses_correm_da_autorizacao_e_nao_da_renovacao() {
        let p = caminho();
        let perfil = Uuid::new_v4();
        let autorizado_em = Utc::now() - Duration::days(179);

        let mut store = TokenStore::load(&p);
        store.autorizou(perfil, "refresh-1", autorizado_em).unwrap();

        // Muitas renovações ao longo dos meses...
        for i in 0..50 {
            store.renovou(perfil, Some(&format!("refresh-{i}"))).unwrap();
        }

        let guardado = store.get(perfil).unwrap();
        assert!(!guardado.venceu(Utc::now()), "ainda dentro dos 180 dias");
        assert_eq!(guardado.dias_restantes(Utc::now()), 0, "está no último dia");
        assert!(
            guardado.venceu(Utc::now() + Duration::days(2)),
            "dois dias depois tem que estar vencido, por mais que tenha renovado"
        );

        limpar(&p);
    }

    #[test]
    fn perfis_tem_tokens_independentes() {
        let p = caminho();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());

        let mut store = TokenStore::load(&p);
        store.autorizou(a, "token-a", Utc::now()).unwrap();
        store.autorizou(b, "token-b", Utc::now()).unwrap();
        store.renovou(a, Some("token-a2")).unwrap();

        assert_eq!(store.get(a).unwrap().refresh_token, "token-a2");
        assert_eq!(
            store.get(b).unwrap().refresh_token,
            "token-b",
            "renovar um perfil não pode mexer no outro"
        );

        store.esqueceu(a).unwrap();
        assert!(store.get(a).is_none());
        assert!(store.get(b).is_some());

        limpar(&p);
    }

    #[test]
    fn renovar_perfil_desconhecido_nao_cria_entrada() {
        let p = caminho();
        let mut store = TokenStore::load(&p);
        store.renovou(Uuid::new_v4(), Some("orfao")).unwrap();
        assert!(store.tokens.is_empty());
        limpar(&p);
    }

    #[cfg(unix)]
    #[test]
    fn o_arquivo_de_tokens_nao_fica_legivel_para_todo_mundo() {
        use std::os::unix::fs::PermissionsExt;

        let p = caminho();
        let mut store = TokenStore::load(&p);
        store.autorizou(Uuid::new_v4(), "segredo", Utc::now()).unwrap();

        let modo = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(modo, 0o600, "segredo de longa duração não pode ficar aberto");

        limpar(&p);
    }

    #[test]
    fn cofre_corrompido_nao_impede_o_boot() {
        let p = caminho();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"nao sou json").unwrap();

        let store = TokenStore::load(&p);
        assert!(store.tokens.is_empty());
        assert!(p.with_extension("json.corrompido").exists());

        limpar(&p);
    }
}
