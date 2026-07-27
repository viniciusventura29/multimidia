//! Núcleo do Eclipse OS: o contrato de módulo, o barramento de estado e o
//! supervisor que mantém tudo no ar.
//!
//! Esta crate não sabe nada sobre Tauri, janelas ou React. Ela define como um
//! módulo se comporta e o que acontece quando um deles falha — o resto do
//! sistema é só transporte. O único I/O que ela faz é a persistência de perfis,
//! e mesmo essa recebe o caminho de fora.

pub mod module;
pub mod profile;
pub mod state;
pub mod supervisor;

pub use module::{
    factory, BoxError, Module, ModuleCommand, ModuleCtx, ModuleFactory, ModuleResult,
};
pub use profile::{Preferences, Profile, ProfileError, ProfileStore, Units};
pub use state::{ModuleId, ModuleState, StateEnvelope, Status};
pub use supervisor::Supervisor;
