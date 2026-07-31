use serde::Serialize;

/// Os PIDs que o painel lê.
///
/// Os cinco primeiros alimentam mostradores; os cinco últimos existem para a conta
/// de consumo, e só entram na varredura se o carro disser que os responde. Num
/// barramento de 10.400 baud, pedir um PID que ninguém atende é leitura de RPM
/// jogada fora.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pid {
    Rpm,
    Speed,
    Coolant,
    Fuel,
    Voltage,
    /// Massa de ar admitida, em g/s (`0110`). A melhor fonte de consumo.
    Maf,
    /// Carga calculada, em % (`0104`). É definida como "fluxo de ar atual ÷ máximo",
    /// então serve de fonte de consumo quando não há MAF.
    Carga,
    /// Pressão absoluta no coletor, em kPa (`010B`).
    Map,
    /// Temperatura do ar admitido, em °C (`010F`).
    Iat,
    /// Vazão de combustível que o próprio carro calcula, em L/h (`015E`). Raro em
    /// carro de 2000, mas quando existe dispensa toda a estimativa.
    VazaoComb,
}

impl Pid {
    /// Estes mudam rápido e merecem prioridade na varredura.
    ///
    /// A fonte de ar (MAF ou carga) entra aqui junto com RPM e velocidade porque é
    /// ela que é integrada em litros: amostrá-la devagar não deixa o número
    /// atrasado, deixa a conta **errada**.
    pub fn is_rapido(self) -> bool {
        matches!(self, Pid::Rpm | Pid::Speed | Pid::Maf | Pid::Carga)
    }

    /// O número do PID no modo 01, para consultar a máscara de suportados.
    ///
    /// `None` na voltagem: `ATRV` é uma medida do adaptador, não do carro, e por
    /// isso não aparece em máscara nenhuma.
    pub fn codigo(self) -> Option<u8> {
        Some(match self {
            Pid::Rpm => 0x0C,
            Pid::Speed => 0x0D,
            Pid::Coolant => 0x05,
            Pid::Fuel => 0x2F,
            Pid::Maf => 0x10,
            Pid::Carga => 0x04,
            Pid::Map => 0x0B,
            Pid::Iat => 0x0F,
            Pid::VazaoComb => 0x5E,
            Pid::Voltage => return None,
        })
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
    pub maf_gs: Option<f32>,
    pub carga_pct: Option<u8>,
    pub map_kpa: Option<u16>,
    pub iat_c: Option<i32>,
    pub vazao_lh: Option<f32>,
}

impl Readings {
    pub fn apply(&mut self, pid: Pid, valor: f32) {
        match pid {
            Pid::Rpm => self.rpm = Some(valor.max(0.0).round() as u32),
            Pid::Speed => self.speed_kmh = Some(valor.max(0.0).round() as u32),
            Pid::Coolant => self.coolant_c = Some(valor.round() as i32),
            Pid::Fuel => self.fuel_pct = Some(valor.clamp(0.0, 100.0).round() as u8),
            Pid::Voltage => self.voltage = Some(uma_casa(valor)),
            // Duas casas: em marcha lenta o MAF fica entre 2 e 5 g/s, e arredondar
            // para inteiro aí jogaria fora uns 20% da leitura.
            Pid::Maf => self.maf_gs = Some((valor.max(0.0) * 100.0).round() / 100.0),
            Pid::Carga => self.carga_pct = Some(valor.clamp(0.0, 100.0).round() as u8),
            Pid::Map => self.map_kpa = Some(valor.max(0.0).round() as u16),
            Pid::Iat => self.iat_c = Some(valor.round() as i32),
            Pid::VazaoComb => self.vazao_lh = Some(uma_casa(valor.max(0.0))),
        }
    }
}

fn uma_casa(valor: f32) -> f32 {
    (valor * 10.0).round() / 10.0
}
