use serde::Serialize;

/// Os PIDs que o painel lê.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pid {
    Rpm,
    Speed,
    Coolant,
    Fuel,
    Voltage,
}

impl Pid {
    /// Estes mudam rápido e merecem prioridade na varredura.
    pub fn is_rapido(self) -> bool {
        matches!(self, Pid::Rpm | Pid::Speed)
    }
}

/// O conjunto de leituras que a UI recebe.
///
/// Todo campo é anulável, e isso não é frescura: o barramento entrega um PID por
/// vez, então nos primeiros segundos a temperatura está genuinamente vazia
/// enquanto o RPM já anda. Mostrar `0` no lugar de `--` seria mentir sobre o carro.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Readings {
    pub rpm: Option<u32>,
    pub speed_kmh: Option<u32>,
    pub coolant_c: Option<i32>,
    pub fuel_pct: Option<u8>,
    pub voltage: Option<f32>,
}

impl Readings {
    pub fn apply(&mut self, pid: Pid, valor: f32) {
        match pid {
            Pid::Rpm => self.rpm = Some(valor.max(0.0).round() as u32),
            Pid::Speed => self.speed_kmh = Some(valor.max(0.0).round() as u32),
            Pid::Coolant => self.coolant_c = Some(valor.round() as i32),
            Pid::Fuel => self.fuel_pct = Some(valor.clamp(0.0, 100.0).round() as u8),
            Pid::Voltage => self.voltage = Some((valor * 10.0).round() / 10.0),
        }
    }
}
