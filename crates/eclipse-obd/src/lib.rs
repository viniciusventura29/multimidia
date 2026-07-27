//! OBD-II do Eclipse.
//!
//! A crate separa duas coisas que costumam vir grudadas:
//!
//! - **A cadência** ([`Poller`]) — a ordem em que os PIDs são varridos. É igual
//!   para o simulador e para o ELM327, porque quem impõe o ritmo é o barramento
//!   do carro, não a fonte.
//! - **A fonte** ([`ObdSource`]) — de onde sai o número. Hoje só existe o
//!   [`SimulatedSource`]; o adaptador de verdade entra implementando o mesmo trait.
//!
//! O Eclipse GT 2000 fala ISO 9141-2 a 10.400 baud, half-duplex, um round-trip
//! por PID. Não dá 10 Hz — dá cerca de 1 a 3 leituras por segundo no total. Todo
//! o desenho daqui parte disso.

pub mod pid;
pub mod poller;
pub mod sim;
pub mod source;

pub use pid::{Pid, Readings};
pub use poller::{Poller, SCHEDULE};
pub use sim::{SimulatedSource, PID_ROUNDTRIP};
pub use source::{ObdError, ObdSource};
