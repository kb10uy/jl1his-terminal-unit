use crate::{
    baudot::LTRS,
    control::{Keyer, LastTx, QueueFull, TxStatus},
    fsk::mark_drive,
    settings::{DataBits, Diddle, Settings},
};
use heapless::Deque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Idle,
    Send(Settings, u8),
    Hold(bool, Wait),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    Millis(u64),
    Wake(Option<u64>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    PttOn,
    PttOff,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ptt {
    Released,
    Requested,
    Keyed(u64),
}

pub struct Sequencer<const N: usize> {
    settings: Settings,
    queue: Deque<u8, N>,
    ptt: Ptt,
    keep_keyed: bool,
    tail: bool,
    shift_lost: bool,
    starting: bool,
    sent: u32,
    last_tx: LastTx,
}

impl<const N: usize> Sequencer<N> {
    pub const fn new(settings: Settings) -> Self {
        Self {
            settings,
            queue: Deque::new(),
            ptt: Ptt::Released,
            keep_keyed: false,
            tail: false,
            shift_lost: false,
            starting: false,
            sent: 0,
            last_tx: LastTx::None,
        }
    }

    pub fn step(&mut self, now_ms: u64) -> (Step, Option<Event>) {
        let mark = mark_drive(self.settings.invert);
        let keyed_at_ms = match self.ptt {
            Ptt::Released => return (Step::Idle, None),
            Ptt::Requested => {
                self.ptt = Ptt::Keyed(now_ms);
                let delay = Wait::Millis(self.settings.tx_delay_ms.into());
                return (Step::Hold(mark, delay), Some(Event::PttOn));
            }
            Ptt::Keyed(at_ms) => at_ms,
        };
        let remaining_ms = match self.settings.tot_s {
            Some(tot) => {
                let limit_ms = u64::from(tot.get()) * 1_000;
                let elapsed_ms = now_ms.saturating_sub(keyed_at_ms);
                if elapsed_ms >= limit_ms {
                    self.unkey(LastTx::Tot);
                    return (Step::Idle, Some(Event::Timeout));
                }
                Some(limit_ms - elapsed_ms)
            }
            None => None,
        };
        if let Some(word) = self.queue.pop_front() {
            self.tail = false;
            self.starting = true;
            return (Step::Send(self.settings, word), None);
        }
        if self.keep_keyed {
            self.tail = false;
            if self.settings.diddle == Diddle::Ltrs && self.settings.bits == DataBits::BAUDOT {
                self.shift_lost = true;
                return (Step::Send(self.settings, LTRS), None);
            }
            return (Step::Hold(mark, Wait::Wake(remaining_ms)), None);
        }
        if !self.tail {
            self.tail = true;
            let tail = Wait::Millis(self.settings.tx_tail_ms.into());
            return (Step::Hold(mark, tail), None);
        }
        self.tail = false;
        self.ptt = Ptt::Released;
        self.shift_lost = true;
        self.last_tx = LastTx::Done;
        (Step::Idle, Some(Event::PttOff))
    }

    pub fn started(&mut self) {
        if core::mem::take(&mut self.starting) {
            self.sent = self.sent.wrapping_add(1);
        }
    }

    fn unkey(&mut self, ended: LastTx) {
        if core::mem::replace(&mut self.ptt, Ptt::Released) != Ptt::Released {
            self.last_tx = ended;
        }
        self.queue.clear();
        self.keep_keyed = false;
        self.tail = false;
        self.shift_lost = true;
        self.starting = false;
    }
}

impl<const N: usize> Keyer for Sequencer<N> {
    fn apply(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    fn enqueue(&mut self, words: &[u8], resync: Option<u8>) -> Result<(), QueueFull> {
        let resync = resync.filter(|_| self.shift_lost);
        let needed = words.len() + usize::from(resync.is_some());
        if needed > self.queue.capacity() - self.queue.len() {
            return Err(QueueFull);
        }
        for &word in resync.iter().chain(words) {
            let _ = self.queue.push_back(word);
        }
        self.shift_lost = false;
        Ok(())
    }

    fn ptt_on(&mut self) {
        if self.ptt == Ptt::Released {
            self.ptt = Ptt::Requested;
            self.sent = 0;
            self.last_tx = LastTx::None;
        }
        self.keep_keyed = true;
    }

    fn ptt_off(&mut self) {
        self.keep_keyed = false;
    }

    fn abort(&mut self) {
        self.unkey(LastTx::Abort);
    }

    fn status(&self) -> TxStatus {
        TxStatus {
            ptt: self.ptt != Ptt::Released,
            queued: self.queue.len(),
            free: self.queue.capacity() - self.queue.len(),
            sent: self.sent,
            last_tx: self.last_tx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Sequencer, Step, Wait};
    use crate::{
        baudot::{FIGS, LTRS},
        control::{Keyer, LastTx, QueueFull, TxStatus},
        settings::{Setting, Settings},
    };
    use rstest::rstest;

    const QUEUE: usize = 8;
    const MARK: bool = false;
    const TXDELAY_MS: u64 = 100;
    const TXTAIL_MS: u64 = 50;
    const TOT_MS: u64 = 600_000;

    fn settings(changes: &[(Setting, &str)]) -> Settings {
        let mut settings = Settings::default();
        for &(setting, value) in changes {
            settings.set(setting, value.as_bytes()).unwrap();
        }
        settings
    }

    fn keyed(changes: &[(Setting, &str)]) -> Sequencer<QUEUE> {
        let mut sequencer = Sequencer::new(settings(changes));
        sequencer.ptt_on();
        assert_eq!(sequencer.step(0).1, Some(Event::PttOn));
        sequencer
    }

    fn send(word: u8) -> (Step, Option<Event>) {
        (Step::Send(Settings::default(), word), None)
    }

    #[test]
    fn holds_words_until_ptt_is_requested() {
        let mut sequencer = Sequencer::<QUEUE>::new(Settings::default());
        sequencer.enqueue(&[0x03], None).unwrap();
        assert_eq!(sequencer.step(0), (Step::Idle, None));
        assert_eq!(
            sequencer.status(),
            TxStatus {
                ptt: false,
                queued: 1,
                free: QUEUE - 1,
                sent: 0,
                last_tx: LastTx::None,
            }
        );
    }

    #[rstest]
    #[case("OFF", false)]
    #[case("ON", true)]
    fn keys_ptt_and_holds_mark_for_txdelay(#[case] invert: &str, #[case] mark: bool) {
        let mut sequencer = Sequencer::<QUEUE>::new(settings(&[(Setting::Invert, invert)]));
        sequencer.ptt_on();
        assert_eq!(
            sequencer.step(0),
            (
                Step::Hold(mark, Wait::Millis(TXDELAY_MS)),
                Some(Event::PttOn)
            )
        );
        assert!(sequencer.status().ptt);
    }

    #[test]
    fn sends_queued_words_and_then_diddles() {
        let mut sequencer = keyed(&[]);
        sequencer.enqueue(&[0x03, 0x19], None).unwrap();
        assert_eq!(sequencer.step(100), send(0x03));
        assert_eq!(sequencer.step(200), send(0x19));
        assert_eq!(sequencer.step(300), send(LTRS));
        assert_eq!(sequencer.step(400), send(LTRS));
    }

    #[rstest]
    #[case(&[(Setting::Diddle, "OFF")], Some(TOT_MS - 100))]
    #[case(&[(Setting::Bits, "8")], Some(TOT_MS - 100))]
    #[case(&[(Setting::Diddle, "OFF"), (Setting::Tot, "OFF")], None)]
    fn holds_mark_until_woken_without_diddle(
        #[case] changes: &[(Setting, &str)],
        #[case] limit_ms: Option<u64>,
    ) {
        let mut sequencer = keyed(changes);
        assert_eq!(
            sequencer.step(100),
            (Step::Hold(MARK, Wait::Wake(limit_ms)), None)
        );
    }

    #[test]
    fn releases_ptt_after_the_queue_and_txtail() {
        let mut sequencer = keyed(&[]);
        sequencer.enqueue(&[0x03], None).unwrap();
        sequencer.ptt_off();
        assert_eq!(sequencer.step(100), send(0x03));
        assert_eq!(
            sequencer.step(200),
            (Step::Hold(MARK, Wait::Millis(TXTAIL_MS)), None)
        );
        assert!(sequencer.status().ptt);
        assert_eq!(sequencer.step(250), (Step::Idle, Some(Event::PttOff)));
        assert!(!sequencer.status().ptt);
        assert_eq!(sequencer.step(300), (Step::Idle, None));
    }

    #[rstest]
    #[case(&[])]
    #[case(&[0x03, 0x19])]
    fn keys_for_a_request_withdrawn_before_it_took_effect(#[case] words: &[u8]) {
        let mut sequencer = Sequencer::<QUEUE>::new(Settings::default());
        sequencer.enqueue(words, None).unwrap();
        sequencer.ptt_on();
        sequencer.ptt_off();
        assert!(sequencer.status().ptt);
        assert_eq!(
            sequencer.step(0),
            (
                Step::Hold(MARK, Wait::Millis(TXDELAY_MS)),
                Some(Event::PttOn)
            )
        );
        for &word in words {
            assert_eq!(sequencer.step(100), send(word));
        }
        assert_eq!(
            sequencer.step(100),
            (Step::Hold(MARK, Wait::Millis(TXTAIL_MS)), None)
        );
        assert_eq!(sequencer.step(150), (Step::Idle, Some(Event::PttOff)));
        let status = sequencer.status();
        assert_eq!((status.ptt, status.last_tx), (false, LastTx::Done));
    }

    #[test]
    fn abort_withdraws_a_request_that_has_not_taken_effect() {
        let mut sequencer = Sequencer::<QUEUE>::new(Settings::default());
        sequencer.enqueue(&[0x03], None).unwrap();
        sequencer.ptt_on();
        sequencer.abort();
        assert_eq!(
            sequencer.status(),
            TxStatus {
                ptt: false,
                queued: 0,
                free: QUEUE,
                sent: 0,
                last_tx: LastTx::Abort,
            }
        );
        assert_eq!(sequencer.step(0), (Step::Idle, None));
    }

    #[test]
    fn restarts_txtail_after_a_late_word() {
        let mut sequencer = keyed(&[]);
        sequencer.ptt_off();
        assert_eq!(
            sequencer.step(100),
            (Step::Hold(MARK, Wait::Millis(TXTAIL_MS)), None)
        );
        sequencer.enqueue(&[0x03], None).unwrap();
        assert_eq!(sequencer.step(150), send(0x03));
        assert_eq!(
            sequencer.step(250),
            (Step::Hold(MARK, Wait::Millis(TXTAIL_MS)), None)
        );
        assert_eq!(sequencer.step(300), (Step::Idle, Some(Event::PttOff)));
    }

    #[test]
    fn tot_aborts_the_transmission() {
        let mut sequencer = keyed(&[(Setting::Tot, "1")]);
        sequencer.enqueue(&[0x03, 0x19], None).unwrap();
        assert!(matches!(sequencer.step(999), (Step::Send(_, 0x03), None)));
        assert_eq!(sequencer.step(1_000), (Step::Idle, Some(Event::Timeout)));
        assert_eq!(
            sequencer.status(),
            TxStatus {
                ptt: false,
                queued: 0,
                free: QUEUE,
                sent: 0,
                last_tx: LastTx::Tot,
            }
        );
        assert_eq!(sequencer.step(2_000), (Step::Idle, None));
    }

    #[test]
    fn abort_discards_the_queue_and_releases_ptt() {
        let mut sequencer = keyed(&[]);
        sequencer.enqueue(&[0x03, 0x19], None).unwrap();
        sequencer.abort();
        assert_eq!(
            sequencer.status(),
            TxStatus {
                ptt: false,
                queued: 0,
                free: QUEUE,
                sent: 0,
                last_tx: LastTx::Abort,
            }
        );
        assert_eq!(sequencer.step(100), (Step::Idle, None));
    }

    #[test]
    fn counts_the_queued_words_that_have_started() {
        let mut sequencer = keyed(&[]);
        sequencer.enqueue(&[0x03, 0x19], None).unwrap();
        assert_eq!(sequencer.step(100), send(0x03));
        assert_eq!(sequencer.status().sent, 0);
        sequencer.started();
        sequencer.started();
        assert_eq!(sequencer.status().sent, 1);
        assert_eq!(sequencer.step(200), send(0x19));
        sequencer.started();
        assert_eq!(sequencer.step(300), send(LTRS));
        sequencer.started();
        assert_eq!(sequencer.status().sent, 2);
    }

    #[test]
    fn does_not_count_a_word_cut_off_before_it_started() {
        let mut sequencer = keyed(&[]);
        sequencer.enqueue(&[0x03], None).unwrap();
        assert_eq!(sequencer.step(100), send(0x03));
        sequencer.abort();
        sequencer.started();
        assert_eq!(sequencer.status().sent, 0);
    }

    #[test]
    fn reports_a_transmission_sent_in_full_until_the_next_one() {
        let mut sequencer = keyed(&[]);
        sequencer.enqueue(&[0x03], None).unwrap();
        sequencer.ptt_off();
        assert_eq!(sequencer.step(100), send(0x03));
        sequencer.started();
        sequencer.step(200);
        assert_eq!(sequencer.status().last_tx, LastTx::None);
        sequencer.ptt_on();
        assert_eq!(sequencer.status().sent, 1);
        sequencer.ptt_off();
        assert_eq!(sequencer.step(250), (Step::Idle, Some(Event::PttOff)));
        sequencer.abort();
        let status = sequencer.status();
        assert_eq!((status.sent, status.last_tx), (1, LastTx::Done));
        sequencer.ptt_on();
        let status = sequencer.status();
        assert_eq!((status.sent, status.last_tx), (0, LastTx::None));
    }

    #[test]
    fn enqueues_all_or_nothing() {
        let mut sequencer = Sequencer::<QUEUE>::new(Settings::default());
        assert_eq!(sequencer.enqueue(&[0; QUEUE + 1], None), Err(QueueFull));
        assert_eq!(sequencer.status().queued, 0);
        sequencer.enqueue(&[0; QUEUE - 1], None).unwrap();
        assert_eq!(sequencer.enqueue(&[0; 2], None), Err(QueueFull));
        assert_eq!(sequencer.status().queued, QUEUE - 1);
    }

    #[test]
    fn counts_the_resync_word_against_the_queue() {
        let mut sequencer = Sequencer::<QUEUE>::new(Settings::default());
        sequencer.abort();
        assert_eq!(sequencer.enqueue(&[0; QUEUE], Some(LTRS)), Err(QueueFull));
        assert_eq!(sequencer.status().queued, 0);
    }

    #[test]
    fn drops_the_resync_while_the_shift_is_known() {
        let mut sequencer = Sequencer::<QUEUE>::new(Settings::default());
        sequencer.enqueue(&[0x19], Some(LTRS)).unwrap();
        assert_eq!(sequencer.status().queued, 1);
    }

    #[test]
    fn resends_the_shift_after_a_diddle() {
        let mut sequencer = keyed(&[]);
        assert_eq!(sequencer.step(100), send(LTRS));
        sequencer.enqueue(&[0x17], Some(FIGS)).unwrap();
        sequencer.enqueue(&[0x13], Some(FIGS)).unwrap();
        assert_eq!(sequencer.step(200), send(FIGS));
        assert_eq!(sequencer.step(300), send(0x17));
        assert_eq!(sequencer.step(400), send(0x13));
    }

    #[rstest]
    #[case::abort(|sequencer: &mut Sequencer<QUEUE>| sequencer.abort())]
    #[case::release(|sequencer: &mut Sequencer<QUEUE>| {
        sequencer.ptt_off();
        sequencer.step(100);
        assert_eq!(sequencer.step(150), (Step::Idle, Some(Event::PttOff)));
    })]
    fn resends_the_shift_after_unkeying(#[case] unkey: fn(&mut Sequencer<QUEUE>)) {
        let mut sequencer = keyed(&[]);
        unkey(&mut sequencer);
        sequencer.enqueue(&[0x19], Some(LTRS)).unwrap();
        assert_eq!(sequencer.status().queued, 2);
    }

    #[test]
    fn applies_settings_between_characters() {
        let mut sequencer = keyed(&[]);
        sequencer.enqueue(&[0x03, 0x19], None).unwrap();
        assert_eq!(sequencer.step(100), send(0x03));
        let changed = settings(&[(Setting::Baud, "50")]);
        sequencer.apply(&changed);
        assert_eq!(sequencer.step(200), (Step::Send(changed, 0x19), None));
    }
}
