use super::{
    Action, BEL, BS, DEL, Keyer, Mode, SUB, SaveError, Storage,
    command::{self, Command},
    error::Error,
    ok,
};
use crate::{
    baudot::{Encoder, Words},
    settings::{Setting, Settings},
};
use core::{fmt::Write, iter, slice};

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

fn hex_digit(digit: u8) -> Result<u8, Error> {
    char::from(digit)
        .to_digit(16)
        .map(|value| value as u8)
        .ok_or(Error::BadArgument)
}

fn switch(on: bool) -> &'static str {
    if on { "ON" } else { "OFF" }
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
        out: &mut impl Write,
    ) -> Result<(), Error> {
        match byte {
            SUB => {
                self.mode = Mode::Command;
                keyer.ptt_off();
                ok(out);
                Ok(())
            }
            BS | DEL => Ok(()),
            b'\r' | b'\n' => self.queue_text(keyer, b"\r\n".iter().copied().map(Ok)),
            _ => self.queue_text(keyer, iter::once(Ok(byte))),
        }
    }

    pub(super) fn run(
        &mut self,
        line: &[u8],
        keyer: &mut impl Keyer,
        storage: &mut impl Storage,
        out: &mut impl Write,
    ) -> Result<Option<Action>, Error> {
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
                self.settings
                    .set(setting, value)
                    .map_err(|_| Error::BadValue)?;
                if setting == Setting::Bits {
                    self.encoder.reset();
                }
                keyer.apply(&self.settings);
            }
            Command::Get(Some(setting)) => {
                let _ = write!(out, "OK {}\r\n", self.settings.value(setting));
                return Ok(None);
            }
            Command::Get(None) => {
                let _ = out.write_str("OK");
                for setting in Setting::ALL {
                    let _ = write!(out, " {}={}", setting.name(), self.settings.value(setting));
                }
                let _ = out.write_str("\r\n");
                return Ok(None);
            }
            Command::Status => {
                let status = keyer.status();
                let mode = match self.mode {
                    Mode::Command => "COMMAND",
                    Mode::Keyboard => "KEYBOARD",
                };
                let _ = write!(
                    out,
                    "OK PTT={} MODE={mode} QUEUED={} FREE={}\r\n",
                    switch(status.ptt),
                    status.queued,
                    status.free
                );
                return Ok(None);
            }
            Command::Save => {
                if keyer.status().transmitting() {
                    return Err(Error::Transmitting);
                }
                storage
                    .save(&self.settings)
                    .map_err(|SaveError(reason)| Error::Save(reason))?;
            }
            Command::Defaults => {
                self.settings = Settings::default();
                self.encoder.reset();
                keyer.apply(&self.settings);
            }
            Command::Ver => {
                let _ = write!(out, "OK {}\r\n", self.version);
                return Ok(None);
            }
            Command::Bootsel => {
                keyer.abort();
                ok(out);
                return Ok(Some(Action::Bootsel));
            }
            Command::Help => {
                let _ = out.write_str("OK");
                for name in command::NAMES {
                    let _ = write!(out, " {name}");
                }
                let _ = out.write_str("\r\n");
                return Ok(None);
            }
        }
        ok(out);
        Ok(None)
    }

    fn require_baudot(&self) -> Result<(), Error> {
        if self.settings.bits == 5 {
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
            if u32::from(*word) >> self.settings.bits != 0 {
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
