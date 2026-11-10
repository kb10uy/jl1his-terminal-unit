use core::f32::consts::TAU;

const PHASE_STEPS_PER_CYCLE: f32 = 4_294_967_296.0;

pub struct Sine {
    phase: u32,
    step: u32,
    peak: f32,
}

impl Sine {
    pub fn new(frequency_hz: u32, sample_rate_hz: u32, amplitude: f32) -> Self {
        let step = ((u64::from(frequency_hz) << 32) + u64::from(sample_rate_hz) / 2)
            / u64::from(sample_rate_hz);
        Self {
            phase: 0,
            step: step as u32,
            peak: amplitude.clamp(0.0, 1.0) * f32::from(i16::MAX),
        }
    }

    pub fn next_sample(&mut self) -> i16 {
        let angle = self.phase as f32 * (TAU / PHASE_STEPS_PER_CYCLE);
        self.phase = self.phase.wrapping_add(self.step);
        libm::roundf(libm::sinf(angle) * self.peak) as i16
    }
}

#[cfg(test)]
mod tests {
    use super::Sine;
    use rstest::rstest;

    #[rstest]
    #[case(1.0, 32767)]
    #[case(0.5, 16384)]
    #[case(2.0, 32767)]
    #[case(-1.0, 0)]
    fn quarter_cycles_reach_peak(#[case] amplitude: f32, #[case] peak: i16) {
        let mut sine = Sine::new(12_000, 48_000, amplitude);
        let samples: [i16; 8] = core::array::from_fn(|_| sine.next_sample());
        assert_eq!(samples, [0, peak, 0, -peak, 0, peak, 0, -peak]);
    }

    #[rstest]
    #[case(1_000, 48_000, 48)]
    #[case(2_125, 48_000, 384)]
    #[case(441, 44_100, 100)]
    fn repeats_after_whole_cycles(
        #[case] frequency_hz: u32,
        #[case] sample_rate_hz: u32,
        #[case] period: usize,
    ) {
        let mut sine = Sine::new(frequency_hz, sample_rate_hz, 1.0);
        let first: Vec<i16> = (0..period).map(|_| sine.next_sample()).collect();
        let second: Vec<i16> = (0..period).map(|_| sine.next_sample()).collect();
        for (a, b) in first.iter().zip(&second) {
            assert!((a - b).abs() <= 1, "{a} != {b}");
        }
    }
}
