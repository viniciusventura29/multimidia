//! O Bluetooth do adaptador OBD no Android — clássico (SPP) e BLE.
//!
//! O WebView do Tauri não expõe Bluetooth (só o GPS vem de graça, via
//! `navigator.geolocation`). Então quem busca, pareia e abre o canal é código
//! Kotlin nativo, e este plugin é a ponte: o Rust (o `src-tauri`) chama estes
//! métodos por `run_mobile_plugin`, e o lado Kotlin fala com o adaptador.
//!
//! Nada aqui é exposto ao JS de propósito — a UI nunca fala com o rádio direto;
//! ela toca em ações de módulo, e os módulos é que chamam isto.

use tauri::{
    plugin::{Builder, PluginApi, TauriPlugin},
    AppHandle, Manager, Runtime,
};

mod error;
mod models;

pub use error::{Error, Result};
pub use models::{BtDevice, BtInfo, BtKind};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.eclipseos.obdbt";

fn init_plugin<R: Runtime>(app: &AppHandle<R>, api: PluginApi<R, ()>) -> crate::Result<ObdBt<R>> {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "ObdBtPlugin")?;
        Ok(ObdBt {
            plugin_handle: handle,
        })
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = api;
        Ok(ObdBt::novo(app.clone()))
    }
}

#[cfg(target_os = "android")]
mod imp {
    use serde::{Deserialize, Serialize};
    use tauri::{plugin::PluginHandle, Runtime};

    use crate::models::{BtDevice, BtInfo, BtKind};

    /// Acesso ao rádio Bluetooth.
    pub struct ObdBt<R: Runtime> {
        pub(crate) plugin_handle: PluginHandle<R>,
    }

    #[derive(Serialize)]
    struct RequestPermissions {
        permissions: Vec<String>,
    }

    #[derive(Deserialize)]
    struct PermStatus {
        #[serde(default)]
        bluetooth: Option<String>,
        #[serde(default)]
        location: Option<String>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ConnectArgs<'a> {
        address: &'a str,
        kind: &'a str,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct BondArgs<'a> {
        address: &'a str,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CommandArgs<'a> {
        cmd: &'a str,
        timeout_ms: u32,
    }

    #[derive(Deserialize)]
    struct DevicesResponse {
        #[serde(default)]
        devices: Vec<BtDevice>,
        #[serde(default)]
        scanning: bool,
    }

    #[derive(Deserialize)]
    struct CommandResponse {
        #[serde(default)]
        response: String,
    }

    impl<R: Runtime> ObdBt<R> {
        /// O que o rádio deste aparelho é: versão do Android, existe, ligado.
        pub fn info(&self) -> crate::Result<BtInfo> {
            Ok(self.plugin_handle.run_mobile_plugin("info", ())?)
        }

        /// Garante as permissões de Bluetooth.
        ///
        /// ⚠️ **`BLUETOOTH_CONNECT` e `BLUETOOTH_SCAN` nasceram na API 31.** No
        /// Android 11 e abaixo elas não existem: `checkSelfPermission` devolve
        /// `denied` para sempre, `requestPermissions` não mostra diálogo nenhum
        /// (o sistema ignora permissão que não conhece), e exigir `granted`
        /// trava o módulo OBD num laço de reinício por uma permissão que aquele
        /// aparelho não tem como conceder. Foi assim que a primeira ignição de
        /// verdade terminou — o diário do carro gravou oito reinícios em 60s
        /// com "permissão de Bluetooth negada", e a permissão não existia.
        ///
        /// Lá quem vale é `BLUETOOTH`/`BLUETOOTH_ADMIN`, declaradas no manifesto
        /// com `maxSdkVersion=30`: são normais, entram na instalação e não têm
        /// diálogo. A única de runtime que importa no Android antigo é a de
        /// localização, e ela é **desejável, não obrigatória** — sem ela a busca
        /// volta vazia, mas conectar num adaptador já escolhido continua
        /// funcionando, que é o que acontece em toda ignição.
        pub fn ensure_permissions(&self) -> crate::Result<()> {
            let info = self.info()?;
            let moderno = info.sdk_int >= 31;

            let atual: PermStatus = self
                .plugin_handle
                .run_mobile_plugin("checkPermissions", ())?;

            let mut faltando = Vec::new();
            if moderno && atual.bluetooth.as_deref() != Some("granted") {
                faltando.push("bluetooth".to_string());
            }
            if !moderno && atual.location.as_deref() != Some("granted") {
                faltando.push("location".to_string());
            }
            if faltando.is_empty() {
                return Ok(());
            }

            let depois: PermStatus = self.plugin_handle.run_mobile_plugin(
                "requestPermissions",
                RequestPermissions {
                    permissions: faltando,
                },
            )?;

            if !moderno {
                if depois.location.as_deref() != Some("granted") {
                    // Não é erro: é o aviso que explica uma lista vazia mais
                    // tarde, e agora ele sobe no diário em vez de sumir.
                    tracing::warn!(
                        sdk = info.sdk_int,
                        "sem permissão de localização: neste Android a BUSCA volta vazia, \
                         mas conectar no adaptador já escolhido segue funcionando"
                    );
                }
                return Ok(());
            }

            if depois.bluetooth.as_deref() == Some("granted") {
                Ok(())
            } else {
                Err(crate::Error::PermissionDenied {
                    sdk: info.sdk_int,
                    bluetooth: depois.bluetooth.unwrap_or_else(|| "?".into()),
                    localizacao: depois.location.unwrap_or_else(|| "?".into()),
                })
            }
        }

        /// Os adaptadores já pareados com o Android.
        pub fn list_bonded(&self) -> crate::Result<Vec<BtDevice>> {
            let r: DevicesResponse = self.plugin_handle.run_mobile_plugin("listBonded", ())?;
            Ok(r.devices)
        }

        /// Começa a procurar (clássico e BLE ao mesmo tempo).
        pub fn start_scan(&self) -> crate::Result<()> {
            self.plugin_handle
                .run_mobile_plugin::<()>("startScan", ())?;
            Ok(())
        }

        /// O que a busca achou até agora, e se ela ainda está correndo.
        pub fn scan_results(&self) -> crate::Result<(Vec<BtDevice>, bool)> {
            let r: DevicesResponse = self.plugin_handle.run_mobile_plugin("scanResults", ())?;
            Ok((r.devices, r.scanning))
        }

        pub fn stop_scan(&self) -> crate::Result<()> {
            self.plugin_handle.run_mobile_plugin::<()>("stopScan", ())?;
            Ok(())
        }

        /// Cria o vínculo com o adaptador, respondendo o PIN de fábrica.
        pub fn bond(&self, address: &str) -> crate::Result<()> {
            self.plugin_handle
                .run_mobile_plugin::<()>("bond", BondArgs { address })?;
            Ok(())
        }

        /// Abre o canal com o adaptador — socket SPP ou GATT, conforme o tipo.
        pub fn connect(&self, address: &str, kind: BtKind) -> crate::Result<()> {
            self.plugin_handle.run_mobile_plugin::<()>(
                "connect",
                ConnectArgs {
                    address,
                    kind: kind.como_texto(),
                },
            )?;
            Ok(())
        }

        /// Manda um comando (sem `\r`) e devolve a resposta crua até o prompt `>`.
        pub fn command(&self, cmd: &str, timeout_ms: u32) -> crate::Result<String> {
            let r: CommandResponse = self
                .plugin_handle
                .run_mobile_plugin("command", CommandArgs { cmd, timeout_ms })?;
            Ok(r.response)
        }

        /// Fecha o canal.
        pub fn disconnect(&self) -> crate::Result<()> {
            self.plugin_handle
                .run_mobile_plugin::<()>("disconnect", ())?;
            Ok(())
        }
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    use std::sync::Mutex;
    use std::time::Instant;

    use tauri::{AppHandle, Runtime};

    use crate::models::{BtDevice, BtInfo, BtKind};

    /// No desktop não há rádio: todo método falha com
    /// [`crate::Error::UnsupportedPlatform`]. O módulo OBD trata isso rodando o
    /// carro simulado, e a tela de adaptador mostra o porquê.
    ///
    /// **Menos com `ECLIPSE_BT_FAKE=1`**, que acende um rádio de mentira: uma
    /// busca que vai revelando aparelhos, um pareamento que às vezes falha, e um
    /// MAC que se lembra. É o que permite construir a tela de escolha no Mac em
    /// vez de gerando APK e subindo no carro a cada ajuste de pixel.
    pub struct ObdBt<R: Runtime> {
        pub(crate) _app: AppHandle<R>,
        fake: Option<Mutex<Fake>>,
    }

    struct Fake {
        busca: Option<Instant>,
        pareados: Vec<String>,
    }

    /// O elenco do rádio de mentira: o adaptador certo, um que não é, e um BLE.
    /// Vêm com atraso para a tela ter que lidar com a lista crescendo.
    const ELENCO: [(&str, &str, BtKind, u64); 4] = [
        ("Galaxy Buds", "11:22:33:44:55:66", BtKind::Spp, 0),
        ("V-LINK", "AA:BB:CC:DD:EE:FF", BtKind::Spp, 1),
        ("iCar Pro BLE", "A0:E6:F8:11:22:33", BtKind::Ble, 3),
        ("OBDII", "00:1D:A5:68:98:8B", BtKind::Spp, 6),
    ];

    impl<R: Runtime> ObdBt<R> {
        pub(crate) fn novo(app: AppHandle<R>) -> Self {
            let fake = std::env::var("ECLIPSE_BT_FAKE")
                .is_ok_and(|v| v == "1")
                .then(|| {
                    Mutex::new(Fake {
                        busca: None,
                        pareados: Vec::new(),
                    })
                });
            Self { _app: app, fake }
        }

        fn fake(&self) -> crate::Result<std::sync::MutexGuard<'_, Fake>> {
            self.fake
                .as_ref()
                .ok_or(crate::Error::UnsupportedPlatform)
                .map(|m| m.lock().unwrap_or_else(|e| e.into_inner()))
        }

        pub fn info(&self) -> crate::Result<BtInfo> {
            drop(self.fake()?);
            Ok(BtInfo {
                sdk_int: 30,
                existe: true,
                ligado: true,
            })
        }

        pub fn ensure_permissions(&self) -> crate::Result<()> {
            self.fake().map(drop)
        }

        pub fn list_bonded(&self) -> crate::Result<Vec<BtDevice>> {
            let fake = self.fake()?;
            Ok(ELENCO
                .iter()
                .filter(|(_, mac, _, _)| fake.pareados.iter().any(|p| p == mac))
                .map(|(nome, mac, kind, _)| dispositivo(nome, mac, *kind, true, None))
                .collect())
        }

        pub fn start_scan(&self) -> crate::Result<()> {
            self.fake()?.busca = Some(Instant::now());
            Ok(())
        }

        pub fn scan_results(&self) -> crate::Result<(Vec<BtDevice>, bool)> {
            let fake = self.fake()?;
            let Some(desde) = fake.busca else {
                return Ok((Vec::new(), false));
            };
            let s = desde.elapsed().as_secs();
            let achados = ELENCO
                .iter()
                .filter(|(_, _, _, atraso)| s >= *atraso)
                .map(|(nome, mac, kind, atraso)| {
                    let pareado = fake.pareados.iter().any(|p| p == mac);
                    dispositivo(nome, mac, *kind, pareado, Some(-40 - (*atraso as i32) * 7))
                })
                .collect();
            Ok((achados, true))
        }

        pub fn stop_scan(&self) -> crate::Result<()> {
            self.fake()?.busca = None;
            Ok(())
        }

        pub fn bond(&self, address: &str) -> crate::Result<()> {
            let mut fake = self.fake()?;
            // O fone nunca pareia: é o caso de erro que a tela precisa saber pintar.
            if address.starts_with("11:22") {
                return Err(crate::Error::PermissionDenied {
                    sdk: 30,
                    bluetooth: "denied".into(),
                    localizacao: "granted".into(),
                });
            }
            if !fake.pareados.iter().any(|p| p == address) {
                fake.pareados.push(address.to_string());
            }
            Ok(())
        }

        pub fn connect(&self, _address: &str, _kind: BtKind) -> crate::Result<()> {
            // Mesmo com o rádio de mentira não há ELM327 do outro lado: no desktop
            // a telemetria vem do carro simulado, que é melhor que um falso.
            drop(self.fake()?);
            Err(crate::Error::UnsupportedPlatform)
        }

        pub fn command(&self, _cmd: &str, _timeout_ms: u32) -> crate::Result<String> {
            Err(crate::Error::UnsupportedPlatform)
        }

        pub fn disconnect(&self) -> crate::Result<()> {
            Ok(())
        }
    }

    fn dispositivo(
        nome: &str,
        mac: &str,
        kind: BtKind,
        bonded: bool,
        rssi: Option<i32>,
    ) -> BtDevice {
        BtDevice {
            name: nome.to_string(),
            address: mac.to_string(),
            kind,
            bonded,
            rssi,
        }
    }
}

pub use imp::ObdBt;

/// Extensão para pegar o [`ObdBt`] a partir de qualquer `Manager` (ex.: `AppHandle`).
pub trait ObdBtExt<R: Runtime> {
    fn obd_bt(&self) -> &ObdBt<R>;
}

impl<R: Runtime, T: Manager<R>> ObdBtExt<R> for T {
    fn obd_bt(&self) -> &ObdBt<R> {
        self.state::<ObdBt<R>>().inner()
    }
}

/// Inicializa o plugin.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("obd-bt")
        .setup(|app, api| {
            app.manage(init_plugin(app, api)?);
            Ok(())
        })
        .build()
}
