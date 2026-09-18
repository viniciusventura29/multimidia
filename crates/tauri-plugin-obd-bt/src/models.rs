use serde::{Deserialize, Serialize};

/// Por onde se fala com o adaptador.
///
/// Não é detalhe de implementação que dê para esconder: um adaptador BLE **não
/// aparece** na busca clássica nem pareia, e um clássico não tem GATT. O tipo
/// acompanha o aparelho desde que ele é visto até ser gravado em disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BtKind {
    /// Bluetooth clássico, socket SPP/RFCOMM.
    #[default]
    Spp,
    /// Bluetooth Low Energy, característica GATT.
    Ble,
}

impl BtKind {
    pub fn como_texto(self) -> &'static str {
        match self {
            Self::Spp => "spp",
            Self::Ble => "ble",
        }
    }
}

/// Um adaptador Bluetooth: pareado, ou só visto numa busca.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BtDevice {
    /// O nome amigável. Pode vir vazio se o Android não o tiver em cache.
    #[serde(default)]
    pub name: String,
    /// O MAC — é por ele que se conecta.
    pub address: String,
    /// Clássico ou BLE.
    #[serde(default)]
    pub kind: BtKind,
    /// Já tem vínculo com o Android? BLE não pareia, então vem sempre `false`.
    #[serde(default)]
    pub bonded: bool,
    /// Força do sinal, quando veio de uma busca. `None` para quem só está pareado.
    #[serde(default)]
    pub rssi: Option<i32>,
}

/// O que o rádio do aparelho é capaz de fazer.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BtInfo {
    /// A versão do Android. Abaixo de 31, buscar exige permissão de localização.
    #[serde(default)]
    pub sdk_int: i32,
    /// O aparelho tem rádio Bluetooth?
    #[serde(default)]
    pub existe: bool,
    /// E está ligado?
    #[serde(default)]
    pub ligado: bool,
}
