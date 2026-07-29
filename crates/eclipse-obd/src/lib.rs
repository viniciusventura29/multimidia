//! OBD-II do Eclipse.
//!
//! A crate separa duas coisas que costumam vir grudadas:
//!
//! - **A cadência** ([`Poller`]) — a ordem em que os PIDs são varridos. É igual
//!   para o simulador e para o ELM327, porque quem impõe o ritmo é o barramento
//!   do carro, não a fonte.
//! - **A fonte** ([`ObdSource`]) — de onde sai o número. É o [`Elm327Source`],
//!   falando com o adaptador Bluetooth por um [`Elm327Transport`]; o transporte
//!   em si (o socket) mora no `src-tauri`, porque é ele que fala com o Android.
//!
//! O Eclipse GT 2000 fala ISO 9141-2 a 10.400 baud, half-duplex, um round-trip
//! por PID. Não dá 10 Hz — dá cerca de 1 a 3 leituras por segundo no total. Todo
//! o desenho daqui parte disso.

pub mod elm327;
pub mod pid;
pub mod poller;
pub mod source;

pub use elm327::{Elm327Source, Elm327Transport};
pub use pid::{Pid, Readings};
pub use poller::{Poller, SCHEDULE};
pub use source::{ObdError, ObdSource};
