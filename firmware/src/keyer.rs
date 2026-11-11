use core::cell::RefCell;
use defmt::{debug, info, warn};
use embassy_sync::{
    blocking_mutex::{NoopMutex, raw::NoopRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Instant, Timer};
use heapless::Deque;
use terminal_unit_core::{
    control::{Keyer, QueueFull, TxStatus},
    settings::{Diddle, Settings, StopBits},
};

pub const QUEUE_WORDS: usize = 8192;

pub struct StubKeyer {
    state: NoopMutex<RefCell<State>>,
    wake: Signal<NoopRawMutex, ()>,
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
    Wait(Duration),
}

impl State {
    fn char_time(&self) -> Duration {
        let stop_half_bits = match self.settings.stop {
            StopBits::One => 2,
            StopBits::OneAndHalf => 3,
            StopBits::Two => 4,
        };
        let half_bits = 2 + 2 * u64::from(self.settings.bits) + stop_half_bits;
        Duration::from_micros(half_bits * 500_000_000 / u64::from(self.settings.baud.millibaud()))
    }

    fn abort(&mut self) {
        self.queue.clear();
        self.ptt_requested = false;
        self.tail = false;
        self.shift_lost = true;
        if self.keyed_at.take().is_some() {
            info!("stub keyer: PTT off (abort)");
        }
    }

    fn step(&mut self, now: Instant) -> Step {
        let Some(keyed_at) = self.keyed_at else {
            if !self.ptt_requested {
                return Step::Idle;
            }
            self.keyed_at = Some(now);
            info!("stub keyer: PTT on");
            return Step::Wait(Duration::from_millis(self.settings.tx_delay_ms.into()));
        };
        if let Some(tot) = self.settings.tot_s
            && now - keyed_at >= Duration::from_secs(tot.get().into())
        {
            warn!("stub keyer: TOT expired");
            self.abort();
            return Step::Idle;
        }
        if let Some(word) = self.queue.pop_front() {
            self.tail = false;
            debug!("stub keyer: FSK {=u8:#x}", word);
            return Step::Wait(self.char_time());
        }
        if self.ptt_requested {
            self.tail = false;
            if self.settings.diddle == Diddle::Ltrs {
                self.shift_lost = true;
            }
            return Step::Wait(self.char_time());
        }
        if !self.tail {
            self.tail = true;
            return Step::Wait(Duration::from_millis(self.settings.tx_tail_ms.into()));
        }
        self.tail = false;
        self.keyed_at = None;
        self.shift_lost = true;
        info!("stub keyer: PTT off");
        Step::Idle
    }
}

impl StubKeyer {
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
        }
    }

    pub async fn run(&self) -> ! {
        loop {
            match self.with(|state| state.step(Instant::now())) {
                Step::Idle => self.wake.wait().await,
                Step::Wait(duration) => Timer::after(duration).await,
            }
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        self.state.lock(|state| f(&mut state.borrow_mut()))
    }
}

impl Keyer for &StubKeyer {
    fn apply(&mut self, settings: &Settings) {
        self.with(|state| state.settings = *settings);
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
