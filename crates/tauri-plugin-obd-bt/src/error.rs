use serde::{ser::Serializer, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Bluetooth clássico só está disponível no Android")]
    UnsupportedPlatform,
    /// O erro carrega os fatos de propósito.
    ///
    /// "permissão negada" sozinho mandou o módulo OBD para um laço de reinício e
    /// não disse por quê — só com o `sdkInt` e o estado de cada permissão na
    /// mensagem dá para saber, lendo o diário daqui, se o dono recusou o diálogo
    /// ou se o app pediu algo que aquele Android nem tem.
    #[error("permissão de Bluetooth negada (Android sdk {sdk}; bluetooth={bluetooth}, localizacao={localizacao})")]
    PermissionDenied {
        sdk: i32,
        bluetooth: String,
        localizacao: String,
    },
    #[cfg(mobile)]
    #[error(transparent)]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}
