mod command;
mod error;
mod executor;
mod line;
#[cfg(test)]
mod tests;

use crate::settings::Settings;
use core::fmt::Write;
use error::Error;
use executor::Executor;
use line::{LineBuffer, LineTooLong};

const ETX: u8 = 0x03;
const BEL: u8 = 0x07;
const BS: u8 = 0x08;
const SUB: u8 = 0x1A;
const DEL: u8 = 0x7F;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Bootsel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Command,
    Keyboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueFull;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveError(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxStatus {
    pub ptt: bool,
    pub queued: usize,
    pub free: usize,
}

impl TxStatus {
    fn transmitting(&self) -> bool {
        self.ptt || self.queued > 0
    }
}

pub trait Keyer {
    fn apply(&mut self, settings: &Settings);
    fn enqueue(&mut self, words: &[u8], resync: Option<u8>) -> Result<(), QueueFull>;
    fn ptt_on(&mut self);
    fn ptt_off(&mut self);
    fn abort(&mut self);
    fn status(&self) -> TxStatus;
}

pub trait Storage {
    fn save(&mut self, settings: &Settings) -> Result<(), SaveError>;
}

pub struct Controller<const LINE: usize, const WORDS: usize> {
    line: LineBuffer<LINE>,
    after_cr: bool,
    executor: Executor<WORDS>,
}

impl<const LINE: usize, const WORDS: usize> Controller<LINE, WORDS> {
    pub const fn new(settings: Settings, version: &'static str) -> Self {
        Self {
            line: LineBuffer::new(),
            after_cr: false,
            executor: Executor::new(settings, version),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.executor.settings
    }

    pub fn mode(&self) -> Mode {
        self.executor.mode
    }

    pub fn feed(
        &mut self,
        byte: u8,
        keyer: &mut impl Keyer,
        storage: &mut impl Storage,
        out: &mut impl Write,
    ) -> Option<Action> {
        let after_cr = core::mem::replace(&mut self.after_cr, byte == b'\r');
        let result = match (byte, self.executor.mode) {
            (ETX, _) => {
                self.line.clear();
                self.executor.abort(keyer);
                ok(out);
                Ok(None)
            }
            (b'\n', _) if after_cr => Ok(None),
            (_, Mode::Keyboard) => self.executor.keyboard(byte, keyer, out).map(|()| None),
            (b'\r' | b'\n', Mode::Command) => {
                let result = match self.line.contents() {
                    Ok([]) => Ok(None),
                    Ok(line) => self.executor.run(line, keyer, storage, out),
                    Err(LineTooLong) => Err(Error::LineTooLong),
                };
                self.line.clear();
                result
            }
            (BS | DEL, Mode::Command) => {
                self.line.backspace();
                Ok(None)
            }
            (_, Mode::Command) => {
                self.line.push(byte);
                Ok(None)
            }
        };
        result.unwrap_or_else(|error| {
            let _ = write!(out, "ERR {error}\r\n");
            None
        })
    }
}

fn ok(out: &mut impl Write) {
    let _ = out.write_str("OK\r\n");
}
