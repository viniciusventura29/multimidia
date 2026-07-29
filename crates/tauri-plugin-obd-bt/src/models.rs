use serde::{Deserialize, Serialize};

/// Um adaptador Bluetooth pareado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtDevice {
    /// O nome amigável. Pode vir vazio se o Android não o tiver em cache.
    #[serde(default)]
    pub name: String,
    /// O MAC — é por ele que se conecta.
    pub address: String,
}
