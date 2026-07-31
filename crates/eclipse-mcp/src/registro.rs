//! O catálogo: junta os provedores, lista as ferramentas, despacha pelo nome.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{Ferramenta, McpError};

/// Quem sabe fazer alguma coisa.
///
/// Um provedor agrupa ferramentas que compartilham a mesma dependência — o
/// provedor do carro segura o supervisor, o de imagem segura a chave do
/// OpenRouter. Assim a dependência aparece uma vez, no construtor, em vez de
/// atravessar cada ferramenta.
#[async_trait]
pub trait Provedor: Send + Sync {
    /// O que este provedor sabe fazer. Precisa ser estável entre chamadas: o
    /// catálogo é indexado uma vez, no registro, e vai inteiro no prefixo do
    /// prompt — lista que muda invalida o cache de prompt a cada chamada.
    fn ferramentas(&self) -> Vec<Ferramenta>;

    async fn chamar(&self, nome: &str, args: &Value) -> Result<Value, McpError>;
}

/// O que uma chamada devolve.
///
/// Nunca é `Result`: uma ferramenta que falhou tem que chegar ao modelo como
/// `is_error: true` **com o texto do erro**, para ele tentar outro caminho. Se a
/// falha virasse `Err` e subisse, o laço do agente morreria e o modelo nunca
/// saberia por quê.
#[derive(Clone, Debug, PartialEq)]
pub struct Resultado {
    /// O JSON que o modelo lê.
    pub conteudo: Value,
    pub erro: bool,
}

impl Resultado {
    pub fn ok(conteudo: Value) -> Self {
        Self {
            conteudo,
            erro: false,
        }
    }

    pub fn falha(motivo: impl std::fmt::Display) -> Self {
        Self {
            conteudo: Value::String(motivo.to_string()),
            erro: true,
        }
    }

    /// O texto que vai dentro do `tool_result`.
    pub fn texto(&self) -> String {
        match &self.conteudo {
            Value::String(s) => s.clone(),
            outro => outro.to_string(),
        }
    }
}

/// O catálogo inteiro.
#[derive(Default)]
pub struct Registro {
    provedores: Vec<Arc<dyn Provedor>>,
    /// nome da ferramenta -> índice do provedor. Montado no registro para o
    /// despacho não ter que perguntar `ferramentas()` a todo mundo a cada
    /// chamada — e o modelo chama várias por turno.
    indice: HashMap<String, usize>,
    catalogo: Vec<Ferramenta>,
}

impl Registro {
    pub fn nova() -> Self {
        Self::default()
    }

    /// Acrescenta um provedor.
    ///
    /// Recusa nome repetido: dois provedores declarando `carro_telemetria` é bug
    /// de programação, e é melhor descobrir na subida do app do que descobrir em
    /// viagem, quando o despacho escolher o errado em silêncio.
    pub fn registrar(&mut self, provedor: Arc<dyn Provedor>) -> Result<(), McpError> {
        let ferramentas = provedor.ferramentas();

        // Confere tudo antes de mexer no estado: registro que falha pela metade
        // deixaria o catálogo e o índice discordando.
        for f in &ferramentas {
            if self.indice.contains_key(&f.nome) {
                return Err(McpError::NomeDuplicado(f.nome.clone()));
            }
        }
        let mut vistos = HashMap::new();
        for f in &ferramentas {
            if vistos.insert(&f.nome, ()).is_some() {
                return Err(McpError::NomeDuplicado(f.nome.clone()));
            }
        }

        let idx = self.provedores.len();
        for f in ferramentas {
            self.indice.insert(f.nome.clone(), idx);
            self.catalogo.push(f);
        }
        self.provedores.push(provedor);
        Ok(())
    }

    /// Versão encadeável de [`Registro::registrar`], para montar o catálogo numa
    /// expressão só. Entra em pânico em nome duplicado — o que é apropriado
    /// aqui: é código de fiação, roda na subida, e um catálogo ambíguo não tem
    /// recuperação sensata em tempo de execução.
    pub fn com(mut self, provedor: Arc<dyn Provedor>) -> Self {
        self.registrar(provedor)
            .expect("catálogo de ferramentas com nome duplicado");
        self
    }

    /// O `tools/list` do MCP.
    pub fn listar(&self) -> &[Ferramenta] {
        &self.catalogo
    }

    pub fn conhece(&self, nome: &str) -> bool {
        self.indice.contains_key(nome)
    }

    /// O `tools/call` do MCP.
    pub async fn chamar(&self, nome: &str, args: &Value) -> Resultado {
        let Some(&idx) = self.indice.get(nome) else {
            return Resultado::falha(McpError::Desconhecida(nome.to_string()));
        };

        match self.provedores[idx].chamar(nome, args).await {
            Ok(valor) => Resultado::ok(valor),
            Err(err) => Resultado::falha(err),
        }
    }
}

#[cfg(test)]
pub(crate) mod testes {
    use super::*;
    use serde_json::json;

    /// Um provedor de mentira, para os testes deste crate.
    pub(crate) struct Dublê {
        pub nomes: Vec<String>,
        pub responde: Value,
        pub falha: bool,
    }

    impl Dublê {
        pub fn com_nomes(nomes: &[&str]) -> Self {
            Self {
                nomes: nomes.iter().map(|n| n.to_string()).collect(),
                responde: json!({ "ok": true }),
                falha: false,
            }
        }
    }

    #[async_trait]
    impl Provedor for Dublê {
        fn ferramentas(&self) -> Vec<Ferramenta> {
            self.nomes
                .iter()
                .map(|n| crate::sem_argumentos(n.clone(), "de mentira"))
                .collect()
        }

        async fn chamar(&self, _nome: &str, _args: &Value) -> Result<Value, McpError> {
            if self.falha {
                return Err(McpError::falhou("o barramento não respondeu"));
            }
            Ok(self.responde.clone())
        }
    }

    #[tokio::test]
    async fn despacha_para_o_provedor_dono_do_nome() {
        let registro = Registro::nova()
            .com(Arc::new(Dublê {
                responde: json!({ "quem": "a" }),
                ..Dublê::com_nomes(&["um"])
            }))
            .com(Arc::new(Dublê {
                responde: json!({ "quem": "b" }),
                ..Dublê::com_nomes(&["dois"])
            }));

        assert_eq!(registro.listar().len(), 2);
        assert_eq!(
            registro.chamar("dois", &json!({})).await.conteudo["quem"],
            "b"
        );
    }

    #[tokio::test]
    async fn falha_de_ferramenta_vira_resultado_marcado_e_nao_erro() {
        let registro = Registro::nova().com(Arc::new(Dublê {
            falha: true,
            ..Dublê::com_nomes(&["quebrada"])
        }));

        let r = registro.chamar("quebrada", &json!({})).await;
        assert!(r.erro, "o modelo precisa ver que falhou");
        assert!(
            r.texto().contains("barramento"),
            "e precisa ler o motivo: {}",
            r.texto()
        );
    }

    #[tokio::test]
    async fn ferramenta_inexistente_tambem_volta_como_erro_legivel() {
        let registro = Registro::nova();
        let r = registro.chamar("inventada", &json!({})).await;
        assert!(r.erro);
        assert!(r.texto().contains("inventada"));
    }

    #[test]
    fn nome_duplicado_entre_provedores_e_recusado() {
        let mut registro = Registro::nova();
        registro
            .registrar(Arc::new(Dublê::com_nomes(&["igual"])))
            .unwrap();

        let err = registro
            .registrar(Arc::new(Dublê::com_nomes(&["igual"])))
            .unwrap_err();
        assert!(matches!(err, McpError::NomeDuplicado(n) if n == "igual"));
    }

    #[test]
    fn registro_recusado_nao_deixa_catalogo_pela_metade() {
        let mut registro = Registro::nova();
        registro
            .registrar(Arc::new(Dublê::com_nomes(&["a"])))
            .unwrap();

        // O segundo provedor traz uma boa e uma repetida: nenhuma das duas entra.
        let _ = registro.registrar(Arc::new(Dublê::com_nomes(&["nova", "a"])));

        assert_eq!(registro.listar().len(), 1);
        assert!(!registro.conhece("nova"));
    }
}
