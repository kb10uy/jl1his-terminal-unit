use super::{
    BEL, BS, DEL, Keyer, Mode, SUB, Storage,
    command::{self, Command},
    error::Error,
    reply::Reply,
};
use crate::{
    baudot::{Encoder, Words},
    settings::{DataBits, Setting, Settings},
};
use core::{iter, slice};

struct Unescape<'a>(slice::Iter<'a, u8>);

impl Iterator for Unescape<'_> {
    type Item = Result<u8, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let &byte = self.0.next()?;
        if byte != b'\\' {
            return Some(Ok(byte));
        }
        Some(match self.0.next() {
            Some(b'r') => Ok(b'\r'),
            Some(b'n') => Ok(b'\n'),
            Some(b'a') => Ok(BEL),
            _ => Err(Error::BadEscape),
        })
    }
}

pub(super) struct Executor<const WORDS: usize> {
    pub(super) settings: Settings,
    version: &'static str,
    pub(super) mode: Mode,
    encoder: Encoder,
    words: [u8; WORDS],
}

impl<const WORDS: usize> Executor<WORDS> {
    pub(super) const fn new(settings: Settings, version: &'static str) -> Self {
        Self {
            settings,
            version,
            mode: Mode::Command,
            encoder: Encoder::new(),
            words: [0; WORDS],
        }
    }

    pub(super) fn abort(&mut self, keyer: &mut impl Keyer) {
        keyer.abort();
        self.mode = Mode::Command;
    }

    pub(super) fn keyboard(
        &mut self,
        byte: u8,
        keyer: &mut impl Keyer,
    ) -> Result<Option<Reply<'static>>, Error> {
        match byte {
            SUB => {
                self.mode = Mode::Command;
                keyer.ptt_off();
                return Ok(Some(Reply::Ok));
            }
            BS | DEL => {}
            b'\r' | b'\n' => self.queue_text(keyer, b"\r\n".iter().copied().map(Ok))?,
            _ => self.queue_text(keyer, iter::once(Ok(byte)))?,
        }
        Ok(None)
    }

    pub(super) fn run(
        &mut self,
        line: &[u8],
        keyer: &mut impl Keyer,
        storage: &mut impl Storage,
    ) -> Result<Reply<'_>, Error> {
        match command::parse(line)? {
            Command::Tx(text) => self.queue_text(keyer, Unescape(text.iter()))?,
            Command::Send(hex) => self.send(hex, keyer)?,
            Command::Ptt(true) => keyer.ptt_on(),
            Command::Ptt(false) => keyer.ptt_off(),
            Command::Key => {
                self.require_baudot()?;
                self.mode = Mode::Keyboard;
                keyer.ptt_on();
            }
            Command::Abort => self.abort(keyer),
            Command::Set(setting, value) => {
                self.settings.set(setting, value)?;
                if setting == Setting::Bits {
                    self.encoder.reset();
                }
                keyer.apply(&self.settings);
            }
            Command::Get(Some(setting)) => return Ok(Reply::Value(self.settings.value(setting))),
            Command::Get(None) => return Ok(Reply::Settings(&self.settings)),
            Command::Status => return Ok(Reply::Status(keyer.status(), self.mode)),
            Command::Save => {
                if keyer.status().transmitting() {
                    return Err(Error::Transmitting);
                }
                storage.save(&self.settings)?;
            }
            Command::Defaults => {
                self.settings = Settings::default();
                self.encoder.reset();
                keyer.apply(&self.settings);
            }
            Command::Ver => return Ok(Reply::Version(self.version)),
            Command::Bootsel => {
                keyer.abort();
                return Ok(Reply::Bootsel);
            }
            Command::Help => return Ok(Reply::Help),
        }
        Ok(Reply::Ok)
    }

    fn require_baudot(&self) -> Result<(), Error> {
        if self.settings.bits == DataBits::BAUDOT {
            Ok(())
        } else {
            Err(Error::NeedsBaudot)
        }
    }

    fn queue_text(
        &mut self,
        keyer: &mut impl Keyer,
        chars: impl IntoIterator<Item = Result<u8, Error>>,
    ) -> Result<(), Error> {
        self.require_baudot()?;
        let mut encoder = self.encoder;
        let mut words = Words::new(&mut self.words);
        for ch in chars {
            encoder.encode(self.settings.code, self.settings.usos, ch?, &mut words)?;
        }
        if !words.as_slice().is_empty() {
            keyer.enqueue(words.as_slice(), words.resync())?;
        }
        self.encoder = encoder;
        Ok(())
    }

    fn send(&mut self, hex: &[u8], keyer: &mut impl Keyer) -> Result<(), Error> {
        if !hex.len().is_multiple_of(2) {
            return Err(Error::OddDigits);
        }
        let words = self
            .words
            .get_mut(..hex.len() / 2)
            .ok_or(Error::QueueFull)?;
        for (word, &[high, low]) in words.iter_mut().zip(hex.as_chunks::<2>().0) {
            *word = (hex_digit(high)? << 4) | hex_digit(low)?;
            if !self.settings.bits.fits(*word) {
                return Err(Error::WordTooWide);
            }
        }
        if !words.is_empty() {
            keyer.enqueue(words, None)?;
            self.encoder.reset();
        }
        Ok(())
    }
}

fn hex_digit(digit: u8) -> Result<u8, Error> {
    char::from(digit)
        .to_digit(16)
        .map(|value| value as u8)
        .ok_or(Error::BadArgument)
}
