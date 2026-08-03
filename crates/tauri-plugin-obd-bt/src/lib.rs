//! Bluetooth clássico (SPP/RFCOMM) para o ELM327 no Android.
//!
//! O WebView do Tauri não expõe Bluetooth clássico (só o GPS vem de graça, via
//! `navigator.geolocation`). Então quem abre o socket SPP é código Kotlin nativo,
//! e este plugin é a ponte: o módulo OBD (em Rust, no `src-tauri`) chama estes
//! métodos por `run_mobile_plugin`, e o lado Kotlin fala com o adaptador.
//!
//! Nada aqui é exposto ao JS de propósito — a UI nunca fala com o adaptador
//! direto; ela só vê as leituras que o módulo OBD publica no barramento.

use tauri::{
    plugin::{Builder, PluginApi, TauriPlugin},
    AppHandle, Manager, Runtime,
};

mod error;
mod models;

pub use error::{Error, Result};
pub use models::BtDevice;

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
        Ok(ObdBt { _app: app.clone() })
    }
}

#[cfg(target_os = "android")]
mod imp {
    use serde::{Deserialize, Serialize};
    use tauri::{plugin::PluginHandle, Runtime};

    use crate::models::BtDevice;

    /// Acesso ao Bluetooth do adaptador OBD.
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
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ConnectArgs<'a> {
        address: &'a str,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CommandArgs<'a> {
        cmd: &'a str,
        timeout_ms: u32,
    }

    #[derive(Deserialize)]
    struct BondedResponse {
        #[serde(default)]
        devices: Vec<BtDevice>,
    }

    #[derive(Deserialize)]
    struct CommandResponse {
        #[serde(default)]
        response: String,
    }

    impl<R: Runtime> ObdBt<R> {
        /// Garante a permissão `BLUETOOTH_CONNECT` (runtime no Android 12+).
        ///
        /// Bloqueia até o usuário responder o diálogo. Sem a permissão não dá nem
        /// para listar os pareados, então isto vem antes de tudo.
        pub fn ensure_permissions(&self) -> crate::Result<()> {
            let atual: PermStatus = self
                .plugin_handle
                .run_mobile_plugin("checkPermissions", ())?;
            if atual.bluetooth.as_deref() == Some("granted") {
                return Ok(());
            }

            let depois: PermStatus = self.plugin_handle.run_mobile_plugin(
                "requestPermissions",
                RequestPermissions {
                    permissions: vec!["bluetooth".to_string()],
                },
            )?;

            if depois.bluetooth.as_deref() == Some("granted") {
                Ok(())
            } else {
                Err(crate::Error::PermissionDenied)
            }
        }

        /// Os adaptadores já pareados nas configurações do Android.
        pub fn list_bonded(&self) -> crate::Result<Vec<BtDevice>> {
            let r: BondedResponse = self.plugin_handle.run_mobile_plugin("listBonded", ())?;
            Ok(r.devices)
        }

        /// Abre o socket RFCOMM/SPP com o adaptador do endereço dado.
        pub fn connect(&self, address: &str) -> crate::Result<()> {
            self.plugin_handle
                .run_mobile_plugin::<()>("connect", ConnectArgs { address })?;
            Ok(())
        }

        /// Manda um comando (sem `\r`) e devolve a resposta crua até o prompt `>`.
        pub fn command(&self, cmd: &str, timeout_ms: u32) -> crate::Result<String> {
            let r: CommandResponse = self
                .plugin_handle
                .run_mobile_plugin("command", CommandArgs { cmd, timeout_ms })?;
            Ok(r.response)
        }

        /// Fecha o socket.
        pub fn disconnect(&self) -> crate::Result<()> {
            self.plugin_handle
                .run_mobile_plugin::<()>("disconnect", ())?;
            Ok(())
        }
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    use tauri::{AppHandle, Runtime};

    use crate::models::BtDevice;

    /// No desktop não há Bluetooth clássico: todo método falha com
    /// [`crate::Error::UnsupportedPlatform`]. O módulo OBD trata isso parando
    /// quieto (mostradores escuros) em vez de ficar reiniciando à toa.
    pub struct ObdBt<R: Runtime> {
        pub(crate) _app: AppHandle<R>,
    }

    impl<R: Runtime> ObdBt<R> {
        pub fn ensure_permissions(&self) -> crate::Result<()> {
            Err(crate::Error::UnsupportedPlatform)
        }
        pub fn list_bonded(&self) -> crate::Result<Vec<BtDevice>> {
            Err(crate::Error::UnsupportedPlatform)
        }
        pub fn connect(&self, _address: &str) -> crate::Result<()> {
            Err(crate::Error::UnsupportedPlatform)
        }
        pub fn command(&self, _cmd: &str, _timeout_ms: u32) -> crate::Result<String> {
            Err(crate::Error::UnsupportedPlatform)
        }
        pub fn disconnect(&self) -> crate::Result<()> {
            Err(crate::Error::UnsupportedPlatform)
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
