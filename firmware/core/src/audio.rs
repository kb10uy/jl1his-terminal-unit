pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const DECIMATION: usize = 4;
pub const CAPTURE_RATE_HZ: u32 = SAMPLE_RATE_HZ * DECIMATION as u32;
pub const FRAME_SAMPLES: usize = (SAMPLE_RATE_HZ / 1_000) as usize;
pub const MAX_FRAME_SAMPLES: usize = FRAME_SAMPLES + 1;
pub const TARGET_BACKLOG: usize = 2 * FRAME_SAMPLES;
pub const MAX_BACKLOG: usize = 4 * FRAME_SAMPLES;

const BACKLOG_HYSTERESIS: usize = FRAME_SAMPLES / 4;
const MIDSCALE: i32 = 2048;
const OUTPUT_SHIFT: u32 = 11;
const TAPS: usize = 61;
const COEFFICIENTS: [i16; TAPS] = [
    -2, -2, 1, 9, 19, 27, 27, 12, -21, -66, -108, -124, -94, -7, 126, 267, 363, 353, 198, -100,
    -482, -833, -1005, -858, -299, 673, 1956, 3353, 4612, 5488, 5802, 5488, 4612, 3353, 1956, 673,
    -299, -858, -1005, -833, -482, -100, 198, 353, 363, 267, 126, -7, -94, -124, -108, -66, -21,
    12, 27, 27, 19, 9, 1, -2, -2,
];

pub struct Decimator {
    history: [i32; 2 * TAPS],
    next: usize,
    phase: usize,
}

impl Decimator {
    pub const fn new() -> Self {
        Self {
            history: [0; 2 * TAPS],
            next: 0,
            phase: 0,
        }
    }

    pub fn push(&mut self, raw: u16) -> Option<i16> {
        let sample = i32::from(raw) - MIDSCALE;
        self.history[self.next] = sample;
        self.history[self.next + TAPS] = sample;
        self.next = (self.next + 1) % TAPS;
        self.phase = (self.phase + 1) % DECIMATION;
        if self.phase != 0 {
            return None;
        }
        let sum: i32 = self.history[self.next..self.next + TAPS]
            .iter()
            .zip(&COEFFICIENTS)
            .map(|(&sample, &coefficient)| sample * i32::from(coefficient))
            .sum();
        let scaled = (sum + (1 << (OUTPUT_SHIFT - 1))) >> OUTPUT_SHIFT;
        Some(scaled.clamp(i16::MIN.into(), i16::MAX.into()) as i16)
    }
}

impl Default for Decimator {
    fn default() -> Self {
        Self::new()
    }
}

pub fn packet_len(backlog: usize) -> Option<usize> {
    if backlog > MAX_BACKLOG {
        return None;
    }
    let len = if backlog > TARGET_BACKLOG + BACKLOG_HYSTERESIS {
        FRAME_SAMPLES + 1
    } else if backlog + BACKLOG_HYSTERESIS < TARGET_BACKLOG {
        FRAME_SAMPLES - 1
    } else {
        FRAME_SAMPLES
    };
    Some(len.min(backlog))
}

#[cfg(test)]
mod tests {
    use super::{
        CAPTURE_RATE_HZ, COEFFICIENTS, DECIMATION, Decimator, FRAME_SAMPLES, MAX_BACKLOG,
        TARGET_BACKLOG, packet_len,
    };
    use rstest::rstest;
    use std::f64::consts::TAU;

    fn decimate(input: impl IntoIterator<Item = u16>) -> Vec<i16> {
        let mut decimator = Decimator::new();
        input
            .into_iter()
            .filter_map(|raw| decimator.push(raw))
            .collect()
    }

    fn sine(frequency_hz: f64, amplitude: f64, len: usize) -> impl Iterator<Item = u16> {
        (0..len).map(move |n| {
            let phase = TAU * frequency_hz * n as f64 / f64::from(CAPTURE_RATE_HZ);
            (2048.0 + amplitude * phase.sin())
                .round()
                .clamp(0.0, 4095.0) as u16
        })
    }

    fn rms(samples: &[i16]) -> f64 {
        let sum: f64 = samples.iter().map(|&s| f64::from(s).powi(2)).sum();
        (sum / samples.len() as f64).sqrt()
    }

    #[test]
    fn coefficients_have_unity_gain() {
        let sum: i32 = COEFFICIENTS.iter().map(|&c| i32::from(c)).sum();
        assert_eq!(sum, 32768);
    }

    #[test]
    fn emits_one_sample_per_decimation() {
        let output = decimate(core::iter::repeat_n(2048, 400));
        assert_eq!(output.len(), 400 / DECIMATION);
    }

    #[rstest]
    #[case(2048, 0)]
    #[case(4095, 32752)]
    #[case(0, -32768)]
    fn maps_full_scale_to_full_scale(#[case] raw: u16, #[case] expected: i16) {
        let output = decimate(core::iter::repeat_n(raw, 400));
        assert_eq!(*output.last().unwrap(), expected);
    }

    #[rstest]
    #[case(1_275.0)]
    #[case(2_125.0)]
    #[case(2_295.0)]
    #[case(10_000.0)]
    fn passes_the_passband(#[case] frequency_hz: f64) {
        let output = decimate(sine(frequency_hz, 1000.0, 19_200));
        let expected = 1000.0 * 16.0 / 2.0_f64.sqrt();
        let gain_db = 20.0 * (rms(&output[100..]) / expected).log10();
        assert!(gain_db.abs() < 0.05, "{gain_db} dB at {frequency_hz} Hz");
    }

    #[rstest]
    #[case(24_000.0)]
    #[case(45_875.0)]
    #[case(50_125.0)]
    #[case(70_000.0)]
    fn rejects_what_would_alias(#[case] frequency_hz: f64) {
        let output = decimate(sine(frequency_hz, 2000.0, 19_200));
        let full = 2000.0 * 16.0 / 2.0_f64.sqrt();
        let gain_db = 20.0 * (rms(&output[100..]) / full).log10();
        assert!(gain_db < -60.0, "{gain_db} dB at {frequency_hz} Hz");
    }

    #[rstest]
    #[case(TARGET_BACKLOG, Some(FRAME_SAMPLES))]
    #[case(TARGET_BACKLOG + 12, Some(FRAME_SAMPLES))]
    #[case(TARGET_BACKLOG + 13, Some(FRAME_SAMPLES + 1))]
    #[case(TARGET_BACKLOG - 12, Some(FRAME_SAMPLES))]
    #[case(TARGET_BACKLOG - 13, Some(FRAME_SAMPLES - 1))]
    #[case(30, Some(30))]
    #[case(0, Some(0))]
    #[case(MAX_BACKLOG, Some(FRAME_SAMPLES + 1))]
    #[case(MAX_BACKLOG + 1, None)]
    fn paces_packets_by_backlog(#[case] backlog: usize, #[case] expected: Option<usize>) {
        assert_eq!(packet_len(backlog), expected);
    }
}
