mod output;

pub use output::{FskOutput, release_on_fault};

use core::{
    cell::RefCell,
    future::{Future, pending},
};
use defmt::{debug, info, warn};
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::{NoopMutex, raw::NoopRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Instant, Timer};
use heapless::Deque;
use terminal_unit_core::{
    baudot::LTRS,
    control::{Keyer, QueueFull, TxStatus},
    fsk::drive,
    settings::{Diddle, Settings},
};

pub const QUEUE_WORDS: usize = 8192;

pub struct FskKeyer {
    state: NoopMutex<RefCell<State>>,
    wake: Signal<NoopRawMutex, ()>,
    aborted: Signal<NoopRawMutex, ()>,
}

struct State {
    settings: Settings,
    queue: Deque<u8, QUEUE_WORDS>,
    ptt_requested: bool,
    keyed_at: Option<Instant>,
    tail: bool,
    shift_lost: bool,
}

enum Step {
    Idle,
    Send(Settings, u8),
    Hold(bool, Wait),
}

enum Wait {
    For(Duration),
    Wake(Option<Duration>),
}

impl State {
    fn abort(&mut self) {
        self.queue.clear();
        self.ptt_requested = false;
        self.tail = false;
        self.shift_lost = true;
        if self.keyed_at.take().is_some() {
            info!("keyer: PTT off (abort)");
        }
    }

    fn step(&mut self, now: Instant) -> Step {
        let mark = drive(true, self.settings.invert);
        let Some(keyed_at) = self.keyed_at else {
            if !self.ptt_requested {
                return Step::Idle;
            }
            self.keyed_at = Some(now);
            info!("keyer: PTT on");
            let delay = Duration::from_millis(self.settings.tx_delay_ms.into());
            return Step::Hold(mark, Wait::For(delay));
        };
        let remaining = match self.settings.tot_s {
            Some(tot) => {
                let limit = Duration::from_secs(tot.get().into());
                let elapsed = now - keyed_at;
                if elapsed >= limit {
                    warn!("keyer: TOT expired");
                    self.abort();
                    return Step::Idle;
                }
                Some(limit - elapsed)
            }
            None => None,
        };
        if let Some(word) = self.queue.pop_front() {
            self.tail = false;
            debug!("keyer: FSK {=u8:#x}", word);
            return Step::Send(self.settings, word);
        }
        if self.ptt_requested {
            self.tail = false;
            if self.settings.diddle == Diddle::Ltrs && self.settings.bits == 5 {
                self.shift_lost = true;
                return Step::Send(self.settings, LTRS);
            }
            return Step::Hold(mark, Wait::Wake(remaining));
        }
        if !self.tail {
            self.tail = true;
            let tail = Duration::from_millis(self.settings.tx_tail_ms.into());
            return Step::Hold(mark, Wait::For(tail));
        }
        self.tail = false;
        self.keyed_at = None;
        self.shift_lost = true;
        info!("keyer: PTT off");
        Step::Idle
    }
}

impl FskKeyer {
    pub const fn new(settings: Settings) -> Self {
        Self {
            state: NoopMutex::new(RefCell::new(State {
                settings,
                queue: Deque::new(),
                ptt_requested: false,
                keyed_at: None,
                tail: false,
                shift_lost: false,
            })),
            wake: Signal::new(),
            aborted: Signal::new(),
        }
    }

    pub async fn run(&self, mut output: FskOutput<'_>) -> ! {
        loop {
            let completed = match self.with(|state| state.step(Instant::now())) {
                Step::Idle => {
                    output.release();
                    self.aborted.reset();
                    self.wake.wait().await;
                    continue;
                }
                Step::Send(settings, word) => {
                    self.unless_aborted(output.send(&settings, word)).await
                }
                Step::Hold(drive, wait) => {
                    let hold = async {
                        output.hold(drive).await;
                        match wait {
                            Wait::For(duration) => Timer::after(duration).await,
                            Wait::Wake(limit) => {
                                let timeout = async {
                                    match limit {
                                        Some(duration) => Timer::after(duration).await,
                                        None => pending().await,
                                    }
                                };
                                select(self.wake.wait(), timeout).await;
                            }
                        }
                    };
                    self.unless_aborted(hold).await
                }
            };
            if completed.is_none() {
                output.release();
            }
        }
    }

    async fn unless_aborted<F: Future>(&self, future: F) -> Option<F::Output> {
        match select(future, self.aborted.wait()).await {
            Either::First(output) => Some(output),
            Either::Second(()) => None,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        self.state.lock(|state| f(&mut state.borrow_mut()))
    }
}

impl Keyer for &FskKeyer {
    fn apply(&mut self, settings: &Settings) {
        self.with(|state| state.settings = *settings);
        self.wake.signal(());
    }

    fn enqueue(&mut self, words: &[u8], resync: Option<u8>) -> Result<(), QueueFull> {
        self.with(|state| {
            let resync = resync.filter(|_| state.shift_lost);
            let needed = words.len() + usize::from(resync.is_some());
            if needed > state.queue.capacity() - state.queue.len() {
                return Err(QueueFull);
            }
            for &word in resync.iter().chain(words) {
                let _ = state.queue.push_back(word);
            }
            state.shift_lost = false;
            Ok(())
        })?;
        self.wake.signal(());
        Ok(())
    }

    fn ptt_on(&mut self) {
        self.with(|state| state.ptt_requested = true);
        self.wake.signal(());
    }

    fn ptt_off(&mut self) {
        self.with(|state| state.ptt_requested = false);
        self.wake.signal(());
    }

    fn abort(&mut self) {
        self.with(State::abort);
        self.aborted.signal(());
        self.wake.signal(());
    }

    fn status(&self) -> TxStatus {
        self.with(|state| TxStatus {
            ptt: state.keyed_at.is_some(),
            queued: state.queue.len(),
            free: state.queue.capacity() - state.queue.len(),
        })
    }
}
