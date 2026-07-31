//! O que pode dar errado numa ferramenta.

/// Falha ao executar (ou registrar) uma ferramenta.
///
/// Repare que quase nenhuma destas chega ao chamador como `Err`: o [`Registro`]
/// converte falha de execução em resultado marcado como erro, porque o modelo
/// precisa **ler** o que deu errado para tentar outra coisa. Erro que some é
/// erro que vira alucinação.
///
/// [`Registro`]: crate::Registro
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// Ninguém no registro declara esse nome.
    #[error("não existe ferramenta chamada `{0}`")]
    Desconhecida(String),

    /// Dois provedores declararam a mesma ferramenta. É bug de programação, não
    /// de execução — pega no registro, na subida, e não no meio de uma viagem.
    #[error("já existe uma ferramenta chamada `{0}`")]
    NomeDuplicado(String),

    /// O modelo mandou argumento faltando ou do tipo errado.
    #[error("argumento inválido: {0}")]
    Argumento(String),

    /// A ferramenta rodou e não deu certo (rede caiu, o carro não respondeu).
    #[error("{0}")]
    Falhou(String),
}

impl McpError {
    /// Atalho para `Falhou`, que é o caso comum dentro de um provedor.
    pub fn falhou(motivo: impl std::fmt::Display) -> Self {
        Self::Falhou(motivo.to_string())
    }

    /// Atalho para `Argumento`.
    pub fn argumento(motivo: impl std::fmt::Display) -> Self {
        Self::Argumento(motivo.to_string())
    }
}
