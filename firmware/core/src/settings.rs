use crate::baudot::Code;
use core::{fmt, num::NonZeroU16};

const STOP_BITS: [(&str, StopBits); 3] = [
    ("1", StopBits::One),
    ("1.5", StopBits::OneAndHalf),
    ("2", StopBits::Two),
];
const DIDDLES: [(&str, Diddle); 2] = [("OFF", Diddle::Off), ("LTRS", Diddle::Ltrs)];
const CODES: [(&str, Code); 2] = [("ITA2", Code::Ita2), ("US", Code::Us)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Baud(u32);

impl Baud {
    pub const MIN: Baud = Baud(10_000);
    pub const MAX: Baud = Baud(1_200_000);

    pub const fn millibaud(self) -> u32 {
        self.0
    }

    fn parse(text: &[u8]) -> Option<Self> {
        let (whole, fraction) = match text.iter().position(|&b| b == b'.') {
            Some(dot) => (&text[..dot], &text[dot + 1..]),
            None => (text, &b"0"[..]),
        };
        if fraction.is_empty() || fraction.len() > 3 {
            return None;
        }
        let mut millibaud = u32::from(parse_u16(whole)?) * 1_000;
        let mut scale = 100;
        for &digit in fraction {
            if !digit.is_ascii_digit() {
                return None;
            }
            millibaud += u32::from(digit - b'0') * scale;
            scale /= 10;
        }
        (Self::MIN.0..=Self::MAX.0)
            .contains(&millibaud)
            .then_some(Baud(millibaud))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataBits(u8);

impl DataBits {
    pub const BAUDOT: DataBits = DataBits(5);

    pub const fn get(self) -> u8 {
        self.0
    }

    pub const fn fits(self, word: u8) -> bool {
        word as u32 >> self.0 == 0
    }

    fn parse(text: &[u8]) -> Option<Self> {
        parse_u16(text)
            .filter(|bits| (5..=8).contains(bits))
            .map(|bits| DataBits(bits as u8))
    }
}

impl fmt::Display for DataBits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for Baud {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, mut fraction) = (self.0 / 1_000, self.0 % 1_000);
        write!(f, "{whole}")?;
        if fraction == 0 {
            return Ok(());
        }
        let mut digits = 3;
        while fraction % 10 == 0 {
            fraction /= 10;
            digits -= 1;
        }
        write!(f, ".{fraction:0digits$}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopBits {
    One,
    OneAndHalf,
    Two,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diddle {
    Off,
    Ltrs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub baud: Baud,
    pub bits: DataBits,
    pub stop: StopBits,
    pub invert: bool,
    pub tx_delay_ms: u16,
    pub tx_tail_ms: u16,
    pub diddle: Diddle,
    pub usos: bool,
    pub code: Code,
    pub tot_s: Option<NonZeroU16>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            baud: Baud(45_450),
            bits: DataBits::BAUDOT,
            stop: StopBits::OneAndHalf,
            invert: false,
            tx_delay_ms: 100,
            tx_tail_ms: 50,
            diddle: Diddle::Ltrs,
            usos: true,
            code: Code::Ita2,
            tot_s: NonZeroU16::new(600),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Baud,
    Bits,
    Stop,
    Invert,
    TxDelay,
    TxTail,
    Diddle,
    Usos,
    Code,
    Tot,
}

impl Setting {
    pub const ALL: [Setting; 10] = [
        Setting::Baud,
        Setting::Bits,
        Setting::Stop,
        Setting::Invert,
        Setting::TxDelay,
        Setting::TxTail,
        Setting::Diddle,
        Setting::Usos,
        Setting::Code,
        Setting::Tot,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Setting::Baud => "BAUD",
            Setting::Bits => "BITS",
            Setting::Stop => "STOP",
            Setting::Invert => "INVERT",
            Setting::TxDelay => "TXDELAY",
            Setting::TxTail => "TXTAIL",
            Setting::Diddle => "DIDDLE",
            Setting::Usos => "USOS",
            Setting::Code => "CODE",
            Setting::Tot => "TOT",
        }
    }

    pub fn from_name(name: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|setting| name.eq_ignore_ascii_case(setting.name().as_bytes()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidValue;

fn parse_u16(text: &[u8]) -> Option<u16> {
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return None;
    }
    core::str::from_utf8(text).ok()?.parse().ok()
}

fn parse_switch(text: &[u8]) -> Option<bool> {
    if text.eq_ignore_ascii_case(b"ON") {
        Some(true)
    } else if text.eq_ignore_ascii_case(b"OFF") {
        Some(false)
    } else {
        None
    }
}

fn parse_tot(text: &[u8]) -> Option<Option<NonZeroU16>> {
    if text.eq_ignore_ascii_case(b"OFF") {
        Some(None)
    } else {
        parse_u16(text).and_then(NonZeroU16::new).map(Some)
    }
}

pub(crate) fn parse_keyword<T: Copy>(text: &[u8], keywords: &[(&str, T)]) -> Option<T> {
    keywords
        .iter()
        .find(|(keyword, _)| text.eq_ignore_ascii_case(keyword.as_bytes()))
        .map(|&(_, value)| value)
}

fn keyword<T: PartialEq>(value: T, keywords: &[(&'static str, T)]) -> &'static str {
    keywords
        .iter()
        .find(|(_, candidate)| *candidate == value)
        .map_or("", |&(keyword, _)| keyword)
}

fn switch(value: bool) -> &'static str {
    if value { "ON" } else { "OFF" }
}

impl Settings {
    pub fn set(&mut self, setting: Setting, value: &[u8]) -> Result<(), InvalidValue> {
        let applied = match setting {
            Setting::Baud => Baud::parse(value).map(|baud| self.baud = baud),
            Setting::Bits => DataBits::parse(value).map(|bits| self.bits = bits),
            Setting::Stop => parse_keyword(value, &STOP_BITS).map(|stop| self.stop = stop),
            Setting::Invert => parse_switch(value).map(|invert| self.invert = invert),
            Setting::TxDelay => parse_u16(value).map(|ms| self.tx_delay_ms = ms),
            Setting::TxTail => parse_u16(value).map(|ms| self.tx_tail_ms = ms),
            Setting::Diddle => parse_keyword(value, &DIDDLES).map(|diddle| self.diddle = diddle),
            Setting::Usos => parse_switch(value).map(|usos| self.usos = usos),
            Setting::Code => parse_keyword(value, &CODES).map(|code| self.code = code),
            Setting::Tot => parse_tot(value).map(|tot| self.tot_s = tot),
        };
        applied.ok_or(InvalidValue)
    }

    pub fn value(&self, setting: Setting) -> Value<'_> {
        Value {
            settings: self,
            setting,
        }
    }
}

pub struct Value<'a> {
    settings: &'a Settings,
    setting: Setting,
}

impl fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.settings;
        match self.setting {
            Setting::Baud => write!(f, "{}", s.baud),
            Setting::Bits => write!(f, "{}", s.bits),
            Setting::Stop => f.write_str(keyword(s.stop, &STOP_BITS)),
            Setting::Invert => f.write_str(switch(s.invert)),
            Setting::TxDelay => write!(f, "{}", s.tx_delay_ms),
            Setting::TxTail => write!(f, "{}", s.tx_tail_ms),
            Setting::Diddle => f.write_str(keyword(s.diddle, &DIDDLES)),
            Setting::Usos => f.write_str(switch(s.usos)),
            Setting::Code => f.write_str(keyword(s.code, &CODES)),
            Setting::Tot => match s.tot_s {
                Some(seconds) => write!(f, "{seconds}"),
                None => f.write_str("OFF"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DataBits, InvalidValue, Setting, Settings};
    use rstest::rstest;

    #[rstest]
    #[case("5", 0x1F, true)]
    #[case("5", 0x20, false)]
    #[case("7", 0x7F, true)]
    #[case("7", 0x80, false)]
    #[case("8", 0xFF, true)]
    fn checks_that_a_word_fits(#[case] bits: &str, #[case] word: u8, #[case] fits: bool) {
        assert_eq!(DataBits::parse(bits.as_bytes()).unwrap().fits(word), fits);
    }

    #[rstest]
    #[case(Setting::Baud, "45.45", "45.45")]
    #[case(Setting::Baud, "50", "50")]
    #[case(Setting::Baud, "75.0", "75")]
    #[case(Setting::Baud, "45.454", "45.454")]
    #[case(Setting::Baud, "100.10", "100.1")]
    #[case(Setting::Baud, "10", "10")]
    #[case(Setting::Baud, "1200", "1200")]
    #[case(Setting::Bits, "8", "8")]
    #[case(Setting::Stop, "1", "1")]
    #[case(Setting::Stop, "2", "2")]
    #[case(Setting::Invert, "on", "ON")]
    #[case(Setting::TxDelay, "0", "0")]
    #[case(Setting::TxTail, "65535", "65535")]
    #[case(Setting::Diddle, "off", "OFF")]
    #[case(Setting::Usos, "OFF", "OFF")]
    #[case(Setting::Code, "us", "US")]
    #[case(Setting::Tot, "Off", "OFF")]
    #[case(Setting::Tot, "30", "30")]
    fn sets_and_formats(#[case] setting: Setting, #[case] value: &str, #[case] shown: &str) {
        let mut settings = Settings::default();
        settings.set(setting, value.as_bytes()).unwrap();
        assert_eq!(settings.value(setting).to_string(), shown);
    }

    #[rstest]
    #[case(Setting::Baud, "")]
    #[case(Setting::Baud, "45.")]
    #[case(Setting::Baud, ".5")]
    #[case(Setting::Baud, "45.4545")]
    #[case(Setting::Baud, "9.999")]
    #[case(Setting::Baud, "1200.001")]
    #[case(Setting::Baud, "+50")]
    #[case(Setting::Baud, "45,45")]
    #[case(Setting::Bits, "4")]
    #[case(Setting::Bits, "9")]
    #[case(Setting::Stop, "1.25")]
    #[case(Setting::Invert, "1")]
    #[case(Setting::TxDelay, "65536")]
    #[case(Setting::TxDelay, "-1")]
    #[case(Setting::Diddle, "RY")]
    #[case(Setting::Code, "ITA")]
    #[case(Setting::Tot, "0")]
    fn rejects_invalid_values(#[case] setting: Setting, #[case] value: &str) {
        let mut settings = Settings::default();
        assert_eq!(settings.set(setting, value.as_bytes()), Err(InvalidValue));
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn formats_defaults() {
        let settings = Settings::default();
        let shown: Vec<String> = Setting::ALL
            .iter()
            .map(|&setting| format!("{}={}", setting.name(), settings.value(setting)))
            .collect();
        assert_eq!(
            shown.join(" "),
            "BAUD=45.45 BITS=5 STOP=1.5 INVERT=OFF TXDELAY=100 TXTAIL=50 \
             DIDDLE=LTRS USOS=ON CODE=ITA2 TOT=600"
        );
    }

    #[rstest]
    #[case("baud", Some(Setting::Baud))]
    #[case("TxDelay", Some(Setting::TxDelay))]
    #[case("GAIN", None)]
    fn finds_settings_by_name(#[case] name: &str, #[case] setting: Option<Setting>) {
        assert_eq!(Setting::from_name(name.as_bytes()), setting);
    }
}
