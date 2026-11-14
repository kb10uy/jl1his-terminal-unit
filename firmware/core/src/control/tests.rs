use super::{Action, Controller, Keyer, LastTx, Mode, QueueFull, SaveError, Storage, TxStatus};
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
    sent: u32,
    last_tx: LastTx,
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
            sent: self.sent,
            last_tx: self.last_tx,
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
            let action = self
                .controller
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
            sent: 0,
            last_tx: LastTx::None,
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
#[case(b"SENDM 1001\r", "OK\r\n", vec![(vec![0x01, 0x10], None)])]
#[case(b"SENDM 1b\r", "OK\r\n", vec![(vec![0x1B], None)])]
#[case(b"SENDM 20\r", "ERR word does not fit BITS\r\n", vec![])]
#[case(b"SET BITS 7\rSENDM 01\r", "OK\r\nOK\r\n", vec![(vec![0x40], None)])]
#[case(b"SET BITS 8\rSENDM 01c5\r", "OK\r\nOK\r\n", vec![(vec![0x80, 0xA3], None)])]
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
#[case(b"SENDM 18\r")]
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
        "OK PTT=OFF MODE=COMMAND QUEUED=0 FREE=64 SENT=0 LAST_TX=NONE\r\n"
    );
    unit.feed(b"PTT ON\rTX AB\r");
    unit.keyer.sent = 7;
    assert_eq!(
        unit.feed(b"STATUS\r"),
        "OK PTT=ON MODE=COMMAND QUEUED=3 FREE=61 SENT=7 LAST_TX=NONE\r\n"
    );
}

#[rstest]
#[case(LastTx::Done, "DONE")]
#[case(LastTx::Abort, "ABORT")]
#[case(LastTx::Tot, "TOT")]
fn reports_how_the_last_transmission_ended(
    mut unit: Unit,
    #[case] last_tx: LastTx,
    #[case] name: &str,
) {
    unit.keyer.last_tx = last_tx;
    assert_eq!(
        unit.feed(b"STATUS\r"),
        format!("OK PTT=OFF MODE=COMMAND QUEUED=0 FREE=64 SENT=0 LAST_TX={name}\r\n")
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
        "OK TX SEND SENDM PTT KEY ABORT SET GET STATUS SAVE DEFAULTS VER BOOTSEL HELP\r\n"
    );
}
