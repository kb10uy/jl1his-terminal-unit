use crate::{
    baudot::{EncodeError, Encoder, Words},
    command::{self, Command, ParseError},
    settings::{Setting, Settings},
};
use core::{
    fmt::{self, Write},
    iter, slice,
};

const ETX: u8 = 0x03;
const BEL: u8 = 0x07;
const BS: u8 = 0x08;
const SUB: u8 = 0x1A;
const DEL: u8 = 0x7F;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueFull;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveError(pub &'static str);

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
enum Error {
    UnknownCommand,
    BadArgument,
    UnknownSetting,
    BadValue,
    LineTooLong,
    QueueFull,
    BadEscape,
    Unencodable,
    OddDigits,
    WordTooWide,
    NeedsBaudot,
    Transmitting,
    Save(&'static str),
}

impl From<ParseError> for Error {
    fn from(error: ParseError) -> Self {
        match error {
            ParseError::UnknownCommand => Error::UnknownCommand,
            ParseError::BadArgument => Error::BadArgument,
            ParseError::UnknownSetting => Error::UnknownSetting,
        }
    }
}

impl From<EncodeError> for Error {
    fn from(error: EncodeError) -> Self {
        match error {
            EncodeError::Unencodable => Error::Unencodable,
            EncodeError::Full => Error::QueueFull,
        }
    }
}

impl From<QueueFull> for Error {
    fn from(_: QueueFull) -> Self {
        Error::QueueFull
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::UnknownCommand => "unknown command",
            Error::BadArgument => "bad argument",
            Error::UnknownSetting => "unknown setting",
            Error::BadValue => "bad value",
            Error::LineTooLong => "line too long",
            Error::QueueFull => "queue full",
            Error::BadEscape => "bad escape",
            Error::Unencodable => "character not in code",
            Error::OddDigits => "odd number of digits",
            Error::WordTooWide => "word does not fit BITS",
            Error::NeedsBaudot => "TX needs BITS 5",
            Error::Transmitting => "transmitting",
            Error::Save(reason) => reason,
        })
    }
}

struct LineTooLong;

struct LineBuffer<const N: usize> {
    buf: [u8; N],
    len: usize,
    overflowed: bool,
}

impl<const N: usize> LineBuffer<N> {
    const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            overflowed: false,
        }
    }

    fn push(&mut self, byte: u8) {
        match self.buf.get_mut(self.len) {
            Some(slot) if !self.overflowed => {
                *slot = byte;
                self.len += 1;
            }
            _ => self.overflowed = true,
        }
    }

    fn backspace(&mut self) {
        if !self.overflowed {
            self.len = self.len.saturating_sub(1);
        }
    }

    fn contents(&self) -> Result<&[u8], LineTooLong> {
        if self.overflowed {
            Err(LineTooLong)
        } else {
            Ok(&self.buf[..self.len])
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.overflowed = false;
    }
}

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

fn ok(out: &mut impl Write) {
    let _ = out.write_str("OK\r\n");
}

fn switch(on: bool) -> &'static str {
    if on { "ON" } else { "OFF" }
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
            executor: Executor {
                settings,
                version,
                mode: Mode::Command,
                encoder: Encoder::new(),
                words: [0; WORDS],
            },
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

struct Executor<const WORDS: usize> {
    settings: Settings,
    version: &'static str,
    mode: Mode,
    encoder: Encoder,
    words: [u8; WORDS],
}

impl<const WORDS: usize> Executor<WORDS> {
    fn abort(&mut self, keyer: &mut impl Keyer) {
        keyer.abort();
        self.mode = Mode::Command;
    }

    fn keyboard(
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

    fn run(
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

#[cfg(test)]
mod tests {
    use super::{Action, Controller, Keyer, Mode, QueueFull, SaveError, Storage, TxStatus};
    use crate::{
        baudot::{FIGS, LTRS},
        settings::{Baud, Settings},
    };
    use rstest::{fixture, rstest};

    #[derive(Clone, Debug, PartialEq)]
    enum Event {
        Apply(Settings),
        Enqueue(Vec<u8>, Option<u8>),
        PttOn,
        PttOff,
        Abort,
    }

    struct MockKeyer {
        events: Vec<Event>,
        queued: usize,
        capacity: usize,
        ptt: bool,
    }

    impl Keyer for MockKeyer {
        fn apply(&mut self, settings: &Settings) {
            self.events.push(Event::Apply(*settings));
        }

        fn enqueue(&mut self, words: &[u8], resync: Option<u8>) -> Result<(), QueueFull> {
            let needed = words.len() + usize::from(resync.is_some());
            if self.queued + needed > self.capacity {
                return Err(QueueFull);
            }
            self.queued += needed;
            self.events.push(Event::Enqueue(words.to_vec(), resync));
            Ok(())
        }

        fn ptt_on(&mut self) {
            self.ptt = true;
            self.events.push(Event::PttOn);
        }

        fn ptt_off(&mut self) {
            self.events.push(Event::PttOff);
        }

        fn abort(&mut self) {
            self.ptt = false;
            self.queued = 0;
            self.events.push(Event::Abort);
        }

        fn status(&self) -> TxStatus {
            TxStatus {
                ptt: self.ptt,
                queued: self.queued,
                free: self.capacity - self.queued,
            }
        }
    }

    struct MockStorage {
        result: Result<(), SaveError>,
        saved: Option<Settings>,
    }

    impl Storage for MockStorage {
        fn save(&mut self, settings: &Settings) -> Result<(), SaveError> {
            self.result?;
            self.saved = Some(*settings);
            Ok(())
        }
    }

    struct Unit {
        controller: Controller<32, 16>,
        keyer: MockKeyer,
        storage: MockStorage,
        actions: Vec<Action>,
    }

    impl Unit {
        fn feed(&mut self, input: &[u8]) -> String {
            let mut out = String::new();
            for &byte in input {
                let action =
                    self.controller
                        .feed(byte, &mut self.keyer, &mut self.storage, &mut out);
                self.actions.extend(action);
            }
            out
        }

        fn take_events(&mut self) -> Vec<Event> {
            core::mem::take(&mut self.keyer.events)
        }

        fn enqueued(&mut self) -> Vec<(Vec<u8>, Option<u8>)> {
            self.take_events()
                .into_iter()
                .filter_map(|event| match event {
                    Event::Enqueue(words, resync) => Some((words, resync)),
                    _ => None,
                })
                .collect()
        }
    }

    #[fixture]
    fn unit() -> Unit {
        Unit {
            controller: Controller::new(Settings::default(), "0.1.0"),
            keyer: MockKeyer {
                events: Vec::new(),
                queued: 0,
                capacity: 64,
                ptt: false,
            },
            storage: MockStorage {
                result: Ok(()),
                saved: None,
            },
            actions: Vec::new(),
        }
    }

    const VER: &str = "OK 0.1.0\r\n";

    #[rstest]
    #[case(b"VER\r", 1)]
    #[case(b"VER\n", 1)]
    #[case(b"VER\r\n", 1)]
    #[case(b"VER\rVER\nVER\r\n", 3)]
    #[case(b"VER\n\rVER\r\r\n", 2)]
    #[case(b"\r\n\n\r\r", 0)]
    #[case(b"VER", 0)]
    #[case(b"VEX\x08R\r", 1)]
    #[case(b"VERR\x7f\r", 1)]
    #[case(b"\x08\x08VER\r", 1)]
    fn assembles_lines(mut unit: Unit, #[case] input: &[u8], #[case] replies: usize) {
        assert_eq!(unit.feed(input), VER.repeat(replies));
    }

    #[rstest]
    fn accepts_a_full_line_and_rejects_a_longer_one(mut unit: Unit) {
        assert_eq!(unit.feed(b"SET BAUD 00000000000000000045.45\r"), "OK\r\n");
        assert_eq!(
            unit.feed(b"SET BAUD 000000000000000000045.45\x08\x08\x08\r"),
            "ERR line too long\r\n"
        );
        assert_eq!(unit.feed(b"VER\r"), VER);
    }

    #[rstest]
    fn ctrl_c_aborts_at_once_and_discards_the_line(mut unit: Unit) {
        assert_eq!(unit.feed(b"TX AB"), "");
        assert_eq!(unit.feed(b"\x03"), "OK\r\n");
        assert_eq!(unit.take_events(), vec![Event::Abort]);
        assert_eq!(unit.feed(b"VER\r"), VER);
    }

    #[rstest]
    fn queues_text_with_shifts_and_resync(mut unit: Unit) {
        assert_eq!(unit.feed(b"TX cq\rTX  de\rTX 599\r"), "OK\r\n".repeat(3));
        assert_eq!(
            unit.enqueued(),
            vec![
                (vec![LTRS, 0x0E, 0x17], None),
                (vec![0x04, 0x09, 0x01], Some(LTRS)),
                (vec![FIGS, 0x10, 0x18, 0x18], None),
            ]
        );
    }

    #[rstest]
    #[case(b"TX 5\\r\\n\\a\r", vec![FIGS, 0x10, 0x08, 0x02, 0x0B])]
    #[case(b"TX \\n\r", vec![0x02])]
    fn decodes_escapes(mut unit: Unit, #[case] input: &[u8], #[case] words: Vec<u8>) {
        assert_eq!(unit.feed(input), "OK\r\n");
        assert_eq!(unit.enqueued(), vec![(words, None)]);
    }

    #[rstest]
    fn empty_tx_queues_nothing(mut unit: Unit) {
        assert_eq!(unit.feed(b"TX\rTX \r"), "OK\r\n".repeat(2));
        assert_eq!(unit.enqueued(), vec![]);
    }

    #[rstest]
    #[case(b"TX A\\x\r", "ERR bad escape\r\n")]
    #[case(b"TX A\\\r", "ERR bad escape\r\n")]
    #[case(b"TX A\\R\r", "ERR bad escape\r\n")]
    #[case(b"TX A~\r", "ERR character not in code\r\n")]
    #[case(b"TX A$\r", "ERR character not in code\r\n")]
    #[case(b"TX ABCDEFGHIJKLMNOPQ\r", "ERR queue full\r\n")]
    fn rejects_tx_without_queueing_or_shifting(
        mut unit: Unit,
        #[case] input: &[u8],
        #[case] reply: &str,
    ) {
        assert_eq!(unit.feed(input), reply);
        assert_eq!(unit.feed(b"TX B\r"), "OK\r\n");
        assert_eq!(unit.enqueued(), vec![(vec![LTRS, 0x19], None)]);
    }

    #[rstest]
    fn rejects_tx_that_does_not_fit_the_queue(mut unit: Unit) {
        unit.keyer.capacity = 4;
        assert_eq!(unit.feed(b"TX ABCD\r"), "ERR queue full\r\n");
        assert_eq!(unit.feed(b"TX A\r"), "OK\r\n");
        assert_eq!(unit.enqueued(), vec![(vec![LTRS, 0x03], None)]);
    }

    #[rstest]
    fn follows_the_code_and_usos_settings(mut unit: Unit) {
        assert_eq!(
            unit.feed(b"SET CODE US\rSET USOS OFF\rTX $ 1\r"),
            "OK\r\n".repeat(3)
        );
        assert_eq!(unit.enqueued(), vec![(vec![FIGS, 0x09, 0x04, 0x17], None)]);
    }

    #[rstest]
    #[case(b"SEND 1f04\r", "OK\r\n", vec![(vec![0x1F, 0x04], None)])]
    #[case(b"SEND\r", "OK\r\n", vec![])]
    #[case(b"SEND 1f0\r", "ERR odd number of digits\r\n", vec![])]
    #[case(b"SEND 1g\r", "ERR bad argument\r\n", vec![])]
    #[case(b"SEND 0120\r", "ERR word does not fit BITS\r\n", vec![])]
    #[case(b"SET BITS 8\rSEND ff\r", "OK\r\nOK\r\n", vec![(vec![0xFF], None)])]
    fn sends_code_words(
        mut unit: Unit,
        #[case] input: &[u8],
        #[case] reply: &str,
        #[case] queued: Vec<(Vec<u8>, Option<u8>)>,
    ) {
        assert_eq!(unit.feed(input), reply);
        assert_eq!(unit.enqueued(), queued);
    }

    #[rstest]
    fn rejects_send_that_does_not_fit_the_queue(mut unit: Unit) {
        unit.keyer.capacity = 2;
        assert_eq!(unit.feed(b"SEND 010203\r"), "ERR queue full\r\n");
        assert_eq!(unit.enqueued(), vec![]);
    }

    #[rstest]
    #[case(b"SEND 03\r")]
    #[case(b"SET BITS 5\r")]
    #[case(b"DEFAULTS\r")]
    fn forgets_the_shift(mut unit: Unit, #[case] input: &[u8]) {
        unit.feed(b"TX A\r");
        unit.feed(input);
        unit.take_events();
        assert_eq!(unit.feed(b"TX B\r"), "OK\r\n");
        assert_eq!(unit.enqueued(), vec![(vec![LTRS, 0x19], None)]);
    }

    #[rstest]
    #[case(b"TX A\r")]
    #[case(b"KEY\r")]
    fn text_needs_five_bits(mut unit: Unit, #[case] input: &[u8]) {
        unit.feed(b"SET BITS 7\r");
        unit.take_events();
        assert_eq!(unit.feed(input), "ERR TX needs BITS 5\r\n");
        assert_eq!(unit.take_events(), vec![]);
        assert_eq!(unit.controller.mode(), Mode::Command);
    }

    #[rstest]
    #[case(b"PTT ON\r", Event::PttOn)]
    #[case(b"PTT OFF\r", Event::PttOff)]
    #[case(b"ABORT\r", Event::Abort)]
    fn controls_the_keyer(mut unit: Unit, #[case] input: &[u8], #[case] event: Event) {
        assert_eq!(unit.feed(input), "OK\r\n");
        assert_eq!(unit.take_events(), vec![event]);
    }

    #[rstest]
    fn keyboard_mode_queues_each_character(mut unit: Unit) {
        assert_eq!(unit.feed(b"KEY\r\n"), "OK\r\n");
        assert_eq!(unit.controller.mode(), Mode::Keyboard);
        assert_eq!(unit.feed(b"ab\r\n1\n2\x08\x1a"), "OK\r\n");
        assert_eq!(unit.controller.mode(), Mode::Command);
        assert_eq!(
            unit.take_events(),
            vec![
                Event::PttOn,
                Event::Enqueue(vec![LTRS, 0x03], None),
                Event::Enqueue(vec![0x19], Some(LTRS)),
                Event::Enqueue(vec![0x08, 0x02], None),
                Event::Enqueue(vec![FIGS, 0x17], None),
                Event::Enqueue(vec![0x08, 0x02], None),
                Event::Enqueue(vec![0x13], Some(FIGS)),
                Event::PttOff,
            ]
        );
        assert_eq!(unit.feed(b"VER\r"), VER);
    }

    #[rstest]
    fn keyboard_mode_rejects_characters_it_cannot_queue(mut unit: Unit) {
        unit.feed(b"KEY\r");
        unit.keyer.capacity = 2;
        assert_eq!(unit.feed(b"~"), "ERR character not in code\r\n");
        assert_eq!(unit.feed(b"A"), "");
        assert_eq!(unit.feed(b"B"), "ERR queue full\r\n");
        assert_eq!(unit.controller.mode(), Mode::Keyboard);
    }

    #[rstest]
    fn ctrl_c_leaves_keyboard_mode(mut unit: Unit) {
        unit.feed(b"KEY\rA");
        unit.take_events();
        assert_eq!(unit.feed(b"\x03"), "OK\r\n");
        assert_eq!(unit.take_events(), vec![Event::Abort]);
        assert_eq!(unit.controller.mode(), Mode::Command);
    }

    #[rstest]
    fn sets_and_gets_settings(mut unit: Unit) {
        let expected = Settings {
            baud: Baud::MIN,
            ..Settings::default()
        };
        assert_eq!(unit.feed(b"set baud 10\r"), "OK\r\n");
        assert_eq!(unit.take_events().last(), Some(&Event::Apply(expected)));
        assert_eq!(unit.feed(b"GET baud\r"), "OK 10\r\n");
        assert_eq!(
            unit.feed(b"GET\r"),
            "OK BAUD=10 BITS=5 STOP=1.5 INVERT=OFF TXDELAY=100 TXTAIL=50 \
             DIDDLE=LTRS USOS=ON CODE=ITA2 TOT=600\r\n"
        );
        assert_eq!(unit.feed(b"DEFAULTS\r"), "OK\r\n");
        assert_eq!(unit.take_events(), vec![Event::Apply(Settings::default())]);
        assert_eq!(unit.controller.settings(), &Settings::default());
    }

    #[rstest]
    #[case(b"SET BAUD fast\r", "ERR bad value\r\n")]
    #[case(b"SET GAIN 1\r", "ERR unknown setting\r\n")]
    #[case(b"GET GAIN\r", "ERR unknown setting\r\n")]
    #[case(b"FOO\r", "ERR unknown command\r\n")]
    #[case(b"PTT\r", "ERR bad argument\r\n")]
    fn reports_errors(mut unit: Unit, #[case] input: &[u8], #[case] reply: &str) {
        assert_eq!(unit.feed(input), reply);
        assert_eq!(unit.take_events(), vec![]);
    }

    #[rstest]
    fn saves_only_while_idle(mut unit: Unit) {
        unit.feed(b"TX A\r");
        assert_eq!(unit.feed(b"SAVE\r"), "ERR transmitting\r\n");
        unit.feed(b"ABORT\rPTT ON\r");
        assert_eq!(unit.feed(b"SAVE\r"), "ERR transmitting\r\n");
        unit.feed(b"ABORT\r");
        assert_eq!(unit.storage.saved, None);
        assert_eq!(unit.feed(b"SAVE\r"), "OK\r\n");
        assert_eq!(unit.storage.saved, Some(Settings::default()));
    }

    #[rstest]
    fn reports_a_failed_save(mut unit: Unit) {
        unit.storage.result = Err(SaveError("flash busy"));
        assert_eq!(unit.feed(b"SAVE\r"), "ERR flash busy\r\n");
    }

    #[rstest]
    fn reports_status(mut unit: Unit) {
        assert_eq!(
            unit.feed(b"STATUS\r"),
            "OK PTT=OFF MODE=COMMAND QUEUED=0 FREE=64\r\n"
        );
        unit.feed(b"PTT ON\rTX AB\r");
        assert_eq!(
            unit.feed(b"STATUS\r"),
            "OK PTT=ON MODE=COMMAND QUEUED=3 FREE=61\r\n"
        );
    }

    #[rstest]
    fn bootsel_aborts_and_requests_a_reboot(mut unit: Unit) {
        assert_eq!(unit.feed(b"BOOTSEL\r"), "OK\r\n");
        assert_eq!(unit.take_events(), vec![Event::Abort]);
        assert_eq!(unit.actions, vec![Action::Bootsel]);
    }

    #[rstest]
    fn lists_commands(mut unit: Unit) {
        assert_eq!(
            unit.feed(b"HELP\r"),
            "OK TX SEND PTT KEY ABORT SET GET STATUS SAVE DEFAULTS VER BOOTSEL HELP\r\n"
        );
    }
}
