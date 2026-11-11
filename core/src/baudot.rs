pub const LTRS: u8 = 0x1F;
pub const FIGS: u8 = 0x1B;

const SPACE: u8 = 0x04;
const CR: u8 = 0x08;
const LF: u8 = 0x02;
const ASCII_BEL: u8 = 0x07;

const LETTERS: [u8; 26] = [
    0x03, 0x19, 0x0E, 0x09, 0x01, 0x0D, 0x1A, 0x14, 0x06, 0x0B, 0x0F, 0x12, 0x1C, 0x0C, 0x18, 0x16,
    0x17, 0x0A, 0x05, 0x10, 0x07, 0x1E, 0x13, 0x1D, 0x15, 0x11,
];
const DIGITS: [u8; 10] = [0x16, 0x17, 0x13, 0x01, 0x0A, 0x10, 0x15, 0x07, 0x06, 0x18];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Letters,
    Figures,
}

impl Shift {
    pub const fn word(self) -> u8 {
        match self {
            Shift::Letters => LTRS,
            Shift::Figures => FIGS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Ita2,
    Us,
}

fn figure(code: Code, ch: u8) -> Option<u8> {
    let word = match (ch, code) {
        (b'-', _) => 0x03,
        (b'?', _) => 0x19,
        (b':', _) => 0x0E,
        (b'(', _) => 0x0F,
        (b')', _) => 0x12,
        (b'.', _) => 0x1C,
        (b',', _) => 0x0C,
        (b'/', _) => 0x1D,
        (b'\'', Code::Ita2) | (ASCII_BEL, Code::Us) => 0x05,
        (ASCII_BEL, Code::Ita2) | (b'\'', Code::Us) => 0x0B,
        (b'+', Code::Ita2) | (b'"', Code::Us) => 0x11,
        (b'=', Code::Ita2) | (b';', Code::Us) => 0x1E,
        (b'$', Code::Us) => 0x09,
        (b'!', Code::Us) => 0x0D,
        (b'#', Code::Us) => 0x14,
        (b'&', Code::Us) => 0x1A,
        _ => return None,
    };
    Some(word)
}

fn lookup(code: Code, ch: u8) -> Option<(u8, Option<Shift>)> {
    let entry = match ch {
        b'A'..=b'Z' => (LETTERS[usize::from(ch - b'A')], Some(Shift::Letters)),
        b'0'..=b'9' => (DIGITS[usize::from(ch - b'0')], Some(Shift::Figures)),
        b' ' => (SPACE, None),
        b'\r' => (CR, None),
        b'\n' => (LF, None),
        _ => (figure(code, ch)?, Some(Shift::Figures)),
    };
    Some(entry)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    Unencodable,
    Full,
}

pub struct Words<'a> {
    buf: &'a mut [u8],
    len: usize,
    resync: Option<Option<Shift>>,
}

impl<'a> Words<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            resync: None,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    pub fn resync(&self) -> Option<u8> {
        self.resync.flatten().map(Shift::word)
    }

    fn push(&mut self, word: u8) -> Result<(), EncodeError> {
        let slot = self.buf.get_mut(self.len).ok_or(EncodeError::Full)?;
        *slot = word;
        self.len += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Encoder {
    shift: Option<Shift>,
}

impl Encoder {
    pub const fn new() -> Self {
        Self { shift: None }
    }

    pub fn reset(&mut self) {
        self.shift = None;
    }

    pub fn encode(
        &mut self,
        code: Code,
        usos: bool,
        ch: u8,
        words: &mut Words,
    ) -> Result<(), EncodeError> {
        let (word, needed) =
            lookup(code, ch.to_ascii_uppercase()).ok_or(EncodeError::Unencodable)?;
        if let Some(needed) = needed {
            let shifting = self.shift != Some(needed);
            if shifting {
                words.push(needed.word())?;
            }
            words.resync.get_or_insert((!shifting).then_some(needed));
            self.shift = Some(needed);
        }
        words.push(word)?;
        if usos && word == SPACE {
            self.shift = Some(Shift::Letters);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Code, EncodeError, Encoder, FIGS, LTRS, Shift, Words};
    use rstest::rstest;

    fn encode(encoder: &mut Encoder, code: Code, usos: bool, text: &[u8]) -> (Vec<u8>, Option<u8>) {
        let mut buf = [0; 64];
        let mut words = Words::new(&mut buf);
        for &ch in text {
            encoder.encode(code, usos, ch, &mut words).unwrap();
        }
        (words.as_slice().to_vec(), words.resync())
    }

    #[rstest]
    #[case(b"RYRY", vec![LTRS, 0x0A, 0x15, 0x0A, 0x15])]
    #[case(b"cq de", vec![LTRS, 0x0E, 0x17, 0x04, 0x09, 0x01])]
    #[case(b"599", vec![FIGS, 0x10, 0x18, 0x18])]
    #[case(b"A1B", vec![LTRS, 0x03, FIGS, 0x17, LTRS, 0x19])]
    #[case(b"\r\n", vec![0x08, 0x02])]
    #[case(b"-?:().,/", vec![FIGS, 0x03, 0x19, 0x0E, 0x0F, 0x12, 0x1C, 0x0C, 0x1D])]
    fn encodes_text_from_unknown_shift(#[case] text: &[u8], #[case] expected: Vec<u8>) {
        let (words, resync) = encode(&mut Encoder::default(), Code::Ita2, false, text);
        assert_eq!(words, expected);
        assert_eq!(resync, None);
    }

    #[rstest]
    #[case(Code::Ita2, b'\'', 0x05)]
    #[case(Code::Ita2, 0x07, 0x0B)]
    #[case(Code::Ita2, b'+', 0x11)]
    #[case(Code::Ita2, b'=', 0x1E)]
    #[case(Code::Us, 0x07, 0x05)]
    #[case(Code::Us, b'$', 0x09)]
    #[case(Code::Us, b'\'', 0x0B)]
    #[case(Code::Us, b'!', 0x0D)]
    #[case(Code::Us, b'"', 0x11)]
    #[case(Code::Us, b'#', 0x14)]
    #[case(Code::Us, b'&', 0x1A)]
    #[case(Code::Us, b';', 0x1E)]
    fn encodes_figures_of_each_code(#[case] code: Code, #[case] ch: u8, #[case] word: u8) {
        let (words, _) = encode(&mut Encoder::default(), code, false, &[ch]);
        assert_eq!(words, vec![FIGS, word]);
    }

    #[rstest]
    #[case(Code::Ita2, b'$')]
    #[case(Code::Ita2, b'!')]
    #[case(Code::Us, b'+')]
    #[case(Code::Us, b'=')]
    #[case(Code::Ita2, b'\\')]
    #[case(Code::Ita2, b'\t')]
    #[case(Code::Ita2, 0xE3)]
    fn rejects_characters_outside_the_code(#[case] code: Code, #[case] ch: u8) {
        let mut buf = [0; 4];
        let mut words = Words::new(&mut buf);
        assert_eq!(
            Encoder::default().encode(code, false, ch, &mut words),
            Err(EncodeError::Unencodable)
        );
    }

    #[rstest]
    #[case(false, vec![FIGS, 0x17, 0x04, 0x13])]
    #[case(true, vec![FIGS, 0x17, 0x04, FIGS, 0x13])]
    fn unshift_on_space_repeats_figs(#[case] usos: bool, #[case] expected: Vec<u8>) {
        let (words, _) = encode(&mut Encoder::default(), Code::Ita2, usos, b"1 2");
        assert_eq!(words, expected);
    }

    #[test]
    fn unshift_on_space_skips_ltrs_after_space() {
        let (words, _) = encode(&mut Encoder::default(), Code::Ita2, true, b"1 A");
        assert_eq!(words, vec![FIGS, 0x17, 0x04, 0x03]);
    }

    #[test]
    fn keeps_shift_across_calls() {
        let mut encoder = Encoder::default();
        encode(&mut encoder, Code::Ita2, false, b"A");
        assert_eq!(
            encode(&mut encoder, Code::Ita2, false, b" B"),
            (vec![0x04, 0x19], Some(LTRS))
        );
        assert_eq!(
            encode(&mut encoder, Code::Ita2, false, b"2"),
            (vec![FIGS, 0x13], None)
        );
    }

    #[rstest]
    #[case(Shift::Letters, b"\r\nAB", Some(LTRS))]
    #[case(Shift::Figures, b"12", Some(FIGS))]
    #[case(Shift::Letters, b"1", None)]
    #[case(Shift::Letters, b"\r\n ", None)]
    fn reports_the_shift_to_resend(
        #[case] shift: Shift,
        #[case] text: &[u8],
        #[case] resync: Option<u8>,
    ) {
        let mut encoder = Encoder { shift: Some(shift) };
        assert_eq!(encode(&mut encoder, Code::Ita2, false, text).1, resync);
    }

    #[test]
    fn reset_forgets_the_shift() {
        let mut encoder = Encoder::default();
        encode(&mut encoder, Code::Ita2, false, b"A");
        encoder.reset();
        assert_eq!(
            encode(&mut encoder, Code::Ita2, false, b"B"),
            (vec![LTRS, 0x19], None)
        );
    }

    #[test]
    fn reports_a_full_buffer() {
        let mut buf = [0; 1];
        let mut words = Words::new(&mut buf);
        assert_eq!(
            Encoder::default().encode(Code::Ita2, false, b'A', &mut words),
            Err(EncodeError::Full)
        );
    }
}
