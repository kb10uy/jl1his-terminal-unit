use crate::settings::{Settings, StopBits};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framing {
    half_bit_ticks: u32,
    bits: u8,
    stop_half_bits: u8,
    invert: bool,
}

impl Framing {
    pub fn new(settings: &Settings, clock_hz: u32) -> Self {
        let millibaud = u64::from(settings.baud.millibaud());
        let half_bit_ticks = (u64::from(clock_hz) * 500 + millibaud / 2) / millibaud;
        Self {
            half_bit_ticks: half_bit_ticks as u32,
            bits: settings.bits,
            stop_half_bits: match settings.stop {
                StopBits::One => 2,
                StopBits::OneAndHalf => 3,
                StopBits::Two => 4,
            },
            invert: settings.invert,
        }
    }

    pub const fn half_bit_ticks(&self) -> u32 {
        self.half_bit_ticks
    }

    pub const fn half_bits(&self) -> u8 {
        2 + 2 * self.bits + self.stop_half_bits
    }

    pub const fn ticks(&self) -> u64 {
        self.half_bit_ticks as u64 * self.half_bits() as u64
    }

    pub const fn ticks_before_stop(&self) -> u64 {
        self.half_bit_ticks as u64 * (2 + 2 * self.bits as u64)
    }

    pub const fn frame(&self, word: u8) -> u32 {
        let data_half_bits = 2 * self.bits as u32;
        let mut marks = 0;
        let mut bit = 0;
        while bit < self.bits {
            if word >> bit & 1 != 0 {
                marks |= 0b11 << (2 + 2 * bit);
            }
            bit += 1;
        }
        marks |= ((1 << self.stop_half_bits) - 1) << (2 + data_half_bits);
        if self.invert {
            marks
        } else {
            !marks & ((1 << self.half_bits()) - 1)
        }
    }
}

pub const fn drive(mark: bool, invert: bool) -> bool {
    mark == invert
}

#[cfg(test)]
mod tests {
    use super::{Framing, drive};
    use crate::{
        baudot::LTRS,
        settings::{Setting, Settings},
    };
    use rstest::rstest;

    const CLOCK_HZ: u32 = 150_000_000;

    fn settings(changes: &[(Setting, &str)]) -> Settings {
        let mut settings = Settings::default();
        for &(setting, value) in changes {
            settings.set(setting, value.as_bytes()).unwrap();
        }
        settings
    }

    fn levels(framing: &Framing, word: u8) -> String {
        let frame = framing.frame(word);
        assert_eq!(frame >> framing.half_bits(), 0);
        (0..framing.half_bits())
            .map(|half_bit| if frame >> half_bit & 1 != 0 { '1' } else { '0' })
            .collect()
    }

    #[rstest]
    #[case(true, false, false)]
    #[case(false, false, true)]
    #[case(true, true, true)]
    #[case(false, true, false)]
    fn maps_mark_to_the_released_drive_unless_inverted(
        #[case] mark: bool,
        #[case] invert: bool,
        #[case] expected: bool,
    ) {
        assert_eq!(drive(mark, invert), expected);
    }

    #[rstest]
    #[case("10", 7_500_000)]
    #[case("45.45", 1_650_165)]
    #[case("50", 1_500_000)]
    #[case("75", 1_000_000)]
    #[case("1200", 62_500)]
    fn rounds_the_half_bit_to_the_nearest_tick(#[case] baud: &str, #[case] ticks: u32) {
        let framing = Framing::new(&settings(&[(Setting::Baud, baud)]), CLOCK_HZ);
        assert_eq!(framing.half_bit_ticks(), ticks);
    }

    #[rstest]
    fn keeps_the_bit_within_one_tick(
        #[values("10", "45.45", "45.454", "110.001", "999.999", "1200")] baud: &str,
        #[values(150_000_000, 125_000_000, 12_000_000)] clock_hz: u32,
    ) {
        let settings = settings(&[(Setting::Baud, baud)]);
        let framing = Framing::new(&settings, clock_hz);
        let millibaud = u64::from(settings.baud.millibaud());
        let bit = 2 * u64::from(framing.half_bit_ticks()) * millibaud;
        assert!(bit.abs_diff(u64::from(clock_hz) * 1_000) <= millibaud);
    }

    #[rstest]
    #[case("1", "11000000000000")]
    #[case("1.5", "110000000000000")]
    #[case("2", "1100000000000000")]
    fn frames_ltrs_with_each_stop_length(#[case] stop: &str, #[case] expected: &str) {
        let framing = Framing::new(&settings(&[(Setting::Stop, stop)]), CLOCK_HZ);
        assert_eq!(levels(&framing, LTRS), expected);
    }

    #[rstest]
    #[case("OFF", "111100110011000")]
    #[case("ON", "000011001100111")]
    fn sends_data_bits_lsb_first(#[case] invert: &str, #[case] expected: &str) {
        let framing = Framing::new(&settings(&[(Setting::Invert, invert)]), CLOCK_HZ);
        assert_eq!(levels(&framing, 0x0A), expected);
    }

    #[rstest]
    #[case("5", 0xFF, "110000000000000")]
    #[case("6", 0x00, "11111111111111000")]
    #[case("7", 0x55, "1100110011001100000")]
    #[case("8", 0xA5, "110011001111001100000")]
    fn frames_every_word_length(#[case] bits: &str, #[case] word: u8, #[case] expected: &str) {
        let framing = Framing::new(&settings(&[(Setting::Bits, bits)]), CLOCK_HZ);
        assert_eq!(levels(&framing, word), expected);
    }

    #[rstest]
    #[case("5", "1", 14, 12)]
    #[case("5", "1.5", 15, 12)]
    #[case("8", "2", 22, 18)]
    fn measures_the_frame_in_half_bits(
        #[case] bits: &str,
        #[case] stop: &str,
        #[case] half_bits: u8,
        #[case] before_stop: u64,
    ) {
        let settings = settings(&[(Setting::Bits, bits), (Setting::Stop, stop)]);
        let framing = Framing::new(&settings, CLOCK_HZ);
        assert_eq!(framing.half_bits(), half_bits);
        assert_eq!(
            framing.ticks(),
            u64::from(half_bits) * u64::from(framing.half_bit_ticks())
        );
        assert_eq!(
            framing.ticks_before_stop(),
            before_stop * u64::from(framing.half_bit_ticks())
        );
    }
}
