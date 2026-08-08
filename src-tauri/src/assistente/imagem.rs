//! Imagem: foto de lugar de verdade primeiro, geração só quando não há foto.
//!
//! **Tudo vira arquivo local.** As duas ferramentas daqui baixam os bytes e
//! gravam em `ia_imagens/`, devolvendo `arquivo:<nome>`. Isso resolve três
//! coisas de uma vez:
//!
//! - a URL da foto do Places carrega a chave do Maps na query, e ela não pode
//!   passar pelo contexto do modelo nem ficar guardada no estado do módulo;
//! - imagem gerada volta em base64, e base64 no barramento seria reenviado a
//!   cada republicação (o `Bus::publish` herda o `data` anterior);
//! - depois de baixada, a imagem continua na tela mesmo entrando num túnel.
//!
//! Capa de álbum é a exceção: é URL pública de CDN, sem segredo, e o `<img>`
//! busca direto.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use eclipse_mcp::{campo, esquema_objeto, Ferramenta, McpError, Provedor};
use serde_json::{json, Value};
use uuid::Uuid;

use super::orcamento::Orcamento;

/// Quantas imagens guardar antes de apagar as mais antigas.
///
/// O painel fica horas ligado e o armazenamento de uma head unit é pequeno.
const ARQUIVOS_GUARDADOS: usize = 20;

/// A imagem da demonstração (`ECLIPSE_IA_DEMO=1`).
///
/// Mora aqui, e não no módulo que a grava, porque quem precisa **não** apagá-la
/// é a poda — e uma constante duplicada é uma constante que um dia diverge.
pub const ARQUIVO_DEMO: &str = "demonstracao.svg";

const URL_BUSCA_LUGAR: &str = "https://places.googleapis.com/v1/places:searchText";
const URL_IMAGENS_OPENROUTER: &str = "https://openrouter.ai/api/v1/images";

/// Largura máxima da foto baixada. A coluna do assistente tem poucos
/// centímetros; pedir 4800px seria banda e disco jogados fora.
const LARGURA_FOTO: u32 = 720;

pub struct ProvedorImagem {
    http: reqwest::Client,
    chave_maps: Option<String>,
    chave_openrouter: Option<String>,
    modelo_imagem: String,
    dir: PathBuf,
    orcamento: Arc<Mutex<Orcamento>>,
}

impl ProvedorImagem {
    pub fn novo(
        http: reqwest::Client,
        chave_maps: Option<String>,
        chave_openrouter: Option<String>,
        modelo_imagem: String,
        dir_dados: &std::path::Path,
        orcamento: Arc<Mutex<Orcamento>>,
    ) -> Self {
        let dir = dir_dados.join("ia_imagens");
        let _ = std::fs::create_dir_all(&dir);

        Self {
            http,
            chave_maps,
            chave_openrouter,
            modelo_imagem,
            dir,
            orcamento,
        }
    }

    /// Grava os bytes e devolve o `arquivo:<nome>` que vai no cartão.
    fn gravar(&self, bytes: &[u8], extensao: &str) -> Result<String, McpError> {
        let nome = format!("{}.{extensao}", Uuid::new_v4());
        std::fs::write(self.dir.join(&nome), bytes)
            .map_err(|e| McpError::falhou(format!("não consegui gravar a imagem: {e}")))?;
        self.podar();
        Ok(format!("arquivo:{nome}"))
    }

    /// Apaga as mais antigas. Melhor esforço: falhar em limpar não pode
    /// derrubar a ferramenta que acabou de dar certo.
    ///
    /// A imagem da demonstração é a mais antiga do diretório por construção (é
    /// gravada na subida do módulo) e some na primeira poda — deixando o modo
    /// demo, que existe justamente para trabalhar no layout, sem o cartão de
    /// imagem. Ela fica de fora da conta.
    fn podar(&self) {
        let Ok(entradas) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut arquivos: Vec<(std::time::SystemTime, PathBuf)> = entradas
            .flatten()
            .filter(|e| e.file_name() != ARQUIVO_DEMO)
            .filter_map(|e| {
                let quando = e.metadata().ok()?.modified().ok()?;
                Some((quando, e.path()))
            })
            .collect();

        if arquivos.len() <= ARQUIVOS_GUARDADOS {
            return;
        }

        arquivos.sort_by_key(|(quando, _)| *quando);
        for (_, caminho) in arquivos.iter().take(arquivos.len() - ARQUIVOS_GUARDADOS) {
            let _ = std::fs::remove_file(caminho);
        }
    }

    async fn foto_do_lugar(&self, args: &Value) -> Result<Value, McpError> {
        let consulta = args["consulta"]
            .as_str()
            .ok_or_else(|| McpError::argumento("falta `consulta`"))?;

        let chave = self
            .chave_maps
            .as_deref()
            .ok_or_else(|| McpError::falhou("não há chave do Google Maps configurada"))?;

        // O FieldMask é obrigatório na Places API nova, e pedir só o que se usa
        // é o que mantém a cobrança na faixa barata.
        let busca: Value = self
            .http
            .post(URL_BUSCA_LUGAR)
            .header("X-Goog-Api-Key", chave)
            .header("X-Goog-FieldMask", "places.displayName,places.photos")
            .json(&json!({ "textQuery": consulta, "languageCode": "pt-BR" }))
            .send()
            .await
            .map_err(|e| McpError::falhou(format!("busca de lugar falhou: {e}")))?
            .error_for_status()
            .map_err(|e| McpError::falhou(format!("busca de lugar recusada: {e}")))?
            .json()
            .await
            .map_err(|e| McpError::falhou(format!("resposta do Places ilegível: {e}")))?;

        let lugar = busca["places"]
            .get(0)
            .ok_or_else(|| McpError::falhou(format!("não achei nenhum lugar para `{consulta}`")))?;

        let nome_foto = lugar["photos"][0]["name"].as_str().ok_or_else(|| {
            McpError::falhou(format!(
                "achei `{consulta}`, mas ele não tem foto publicada"
            ))
        })?;

        // Sem `skipHttpRedirect`: a API redireciona para os bytes e o reqwest
        // segue o redirecionamento sozinho. Um pedido em vez de dois.
        let url = format!(
            "https://places.googleapis.com/v1/{nome_foto}/media\
             ?maxWidthPx={LARGURA_FOTO}&key={chave}"
        );

        let bytes = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| McpError::falhou(format!("download da foto falhou: {e}")))?
            .error_for_status()
            .map_err(|e| McpError::falhou(format!("foto recusada: {e}")))?
            .bytes()
            .await
            .map_err(|e| McpError::falhou(format!("foto veio quebrada: {e}")))?;

        let arquivo = self.gravar(&bytes, "jpg")?;

        Ok(json!({
            "url": arquivo,
            "lugar": lugar["displayName"]["text"].as_str(),
            "use_assim": "ponha este valor de `url` no cartão de imagem, exatamente como veio.",
        }))
    }

    async fn gerar_imagem(&self, args: &Value) -> Result<Value, McpError> {
        let descricao = args["descricao"]
            .as_str()
            .ok_or_else(|| McpError::argumento("falta `descricao`"))?;

        let chave = self
            .chave_openrouter
            .as_deref()
            .ok_or_else(|| McpError::falhou("não há chave do OpenRouter configurada"))?;

        let agora = Utc::now();
        {
            let mut orcamento = self.orcamento.lock().unwrap_or_else(|e| e.into_inner());
            if !orcamento.pode_gerar_imagem(agora) {
                return Err(McpError::falhou(
                    "o teto diário de geração de imagem acabou — use uma foto de verdade \
                     ou escreva sem imagem",
                ));
            }
            // Debita antes de gerar, não depois: se a resposta demorar, uma
            // segunda chamada não pode passar pelo mesmo teto.
            orcamento.registrar_imagem(agora);
        }

        let resposta: Value = self
            .http
            .post(URL_IMAGENS_OPENROUTER)
            .bearer_auth(chave)
            .json(&json!({
                "model": self.modelo_imagem,
                "prompt": descricao,
                "n": 1,
                "output_format": "png",
            }))
            .send()
            .await
            .map_err(|e| McpError::falhou(format!("geração de imagem falhou: {e}")))?
            .error_for_status()
            .map_err(|e| McpError::falhou(format!("o OpenRouter recusou: {e}")))?
            .json()
            .await
            .map_err(|e| McpError::falhou(format!("resposta do OpenRouter ilegível: {e}")))?;

        let base64 = resposta["data"][0]["b64_json"]
            .as_str()
            .ok_or_else(|| McpError::falhou("o OpenRouter não devolveu imagem"))?;

        let bytes = decodificar_base64(base64)
            .ok_or_else(|| McpError::falhou("a imagem veio em base64 inválido"))?;
        let arquivo = self.gravar(&bytes, "png")?;

        Ok(json!({
            "url": arquivo,
            "use_assim": "ponha este valor de `url` no cartão de imagem, exatamente como veio.",
        }))
    }
}

/// Base64 padrão, sem dependência nova.
///
/// São trinta linhas contra mais um crate na árvore de compilação do Android —
/// e o único uso é decodificar um PNG por vez.
fn decodificar_base64(texto: &str) -> Option<Vec<u8>> {
    const ALFABETO: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut valor: u32 = 0;
    let mut bits = 0u32;
    let mut saida = Vec::with_capacity(texto.len() / 4 * 3);

    for byte in texto.bytes() {
        // Quebras de linha aparecem em base64 de várias fontes.
        if byte.is_ascii_whitespace() || byte == b'=' {
            continue;
        }
        let indice = ALFABETO.iter().position(|c| *c == byte)? as u32;

        valor = (valor << 6) | indice;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            saida.push((valor >> bits) as u8);
        }
    }

    Some(saida)
}

#[async_trait]
impl Provedor for ProvedorImagem {
    fn ferramentas(&self) -> Vec<Ferramenta> {
        vec![
            Ferramenta::nova(
                "foto_do_lugar",
                "Chame quando quiser mostrar COMO É um lugar — a cidade de destino, um \
                 mirante, um restaurante no caminho. Busca uma foto real e a baixa para o \
                 aparelho. Prefira sempre esta a `gerar_imagem`: é grátis e é o lugar de \
                 verdade.",
                esquema_objeto(&[(
                    "consulta",
                    campo(
                        "string",
                        "o lugar, como você diria a alguém: `Campos do Jordão SP`",
                    ),
                )]),
            ),
            Ferramenta::nova(
                "gerar_imagem",
                "Cria uma imagem do zero. **Custa dinheiro e demora alguns segundos**, e há \
                 teto diário. Use só quando a imagem for o ponto do cartão e não existir foto \
                 que sirva — nunca como enfeite, e nunca para ilustrar um lugar real (para \
                 isso existe `foto_do_lugar`).",
                esquema_objeto(&[(
                    "descricao",
                    campo("string", "o que desenhar, em uma ou duas frases"),
                )]),
            ),
        ]
    }

    async fn chamar(&self, nome: &str, args: &Value) -> Result<Value, McpError> {
        match nome {
            "foto_do_lugar" => self.foto_do_lugar(args).await,
            "gerar_imagem" => self.gerar_imagem(args).await,
            outro => Err(McpError::Desconhecida(outro.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistente::orcamento::Config;

    fn dir_temporario() -> PathBuf {
        let d = std::env::temp_dir().join(format!("eclipse-imagem-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn provedor(dir: &std::path::Path, config: Config) -> ProvedorImagem {
        ProvedorImagem::novo(
            reqwest::Client::new(),
            None,
            None,
            "bytedance-seed/seedream-4.5".into(),
            dir,
            Arc::new(Mutex::new(Orcamento::carregar(dir, config))),
        )
    }

    #[test]
    fn base64_decodifica_com_e_sem_padding() {
        assert_eq!(decodificar_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decodificar_base64("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decodificar_base64("aGVs\nbG8=").unwrap(), b"hello");
        assert_eq!(decodificar_base64("").unwrap(), b"");
    }

    #[test]
    fn base64_invalido_e_recusado_em_vez_de_gravar_lixo() {
        assert!(decodificar_base64("não é base64!").is_none());
    }

    #[test]
    fn gravar_devolve_o_prefixo_que_a_tela_entende() {
        let d = dir_temporario();
        let p = provedor(&d, Config::default());

        let url = p.gravar(b"bytes", "png").unwrap();
        assert!(url.starts_with("arquivo:"), "veio {url}");

        let nome = url.trim_start_matches("arquivo:");
        assert_eq!(
            std::fs::read(d.join("ia_imagens").join(nome)).unwrap(),
            b"bytes"
        );
    }

    /// O painel fica horas ligado; a pasta não pode crescer sem fim.
    #[test]
    fn podar_mantem_so_as_mais_recentes() {
        let d = dir_temporario();
        let p = provedor(&d, Config::default());

        for i in 0..ARQUIVOS_GUARDADOS + 7 {
            p.gravar(format!("{i}").as_bytes(), "png").unwrap();
            // Sem isto os `mtime` empatam e a ordenação fica arbitrária.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let quantos = std::fs::read_dir(d.join("ia_imagens")).unwrap().count();
        assert_eq!(quantos, ARQUIVOS_GUARDADOS);
    }

    #[tokio::test]
    async fn sem_chave_a_ferramenta_explica_em_vez_de_estourar() {
        let d = dir_temporario();
        let p = provedor(&d, Config::default());

        let err = p
            .chamar("foto_do_lugar", &json!({ "consulta": "Santos" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Google Maps"), "veio: {err}");

        let err = p
            .chamar("gerar_imagem", &json!({ "descricao": "um carro" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("OpenRouter"), "veio: {err}");
    }

    /// O teto de imagem tem que barrar antes de a chamada paga sair.
    #[tokio::test]
    async fn teto_de_imagem_barra_antes_de_chamar_o_openrouter() {
        let d = dir_temporario();
        let mut p = provedor(
            &d,
            Config {
                imagens_por_dia: 0,
                ..Config::default()
            },
        );
        // Com chave configurada, para provar que o barrado vem do teto e não da
        // falta de credencial.
        p.chave_openrouter = Some("sk-de-mentira".into());

        let err = p
            .chamar("gerar_imagem", &json!({ "descricao": "um carro" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("teto diário"), "veio: {err}");
    }

    #[tokio::test]
    async fn argumento_faltando_e_recusado_com_motivo() {
        let d = dir_temporario();
        let p = provedor(&d, Config::default());

        let err = p.chamar("foto_do_lugar", &json!({})).await.unwrap_err();
        assert!(matches!(err, McpError::Argumento(_)));
    }
}
