mod output;

pub use output::{FskOutput, PttOutput, outputs, release_on_fault};

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
use embassy_time::{Instant, Timer};
use terminal_unit_core::{
    control::{Keyer, QueueFull, TxStatus},
    keyer::{Event, Sequencer, Step, Wait},
    settings::Settings,
};

pub const QUEUE_WORDS: usize = 8192;

pub struct FskKeyer {
    sequencer: NoopMutex<RefCell<Sequencer<QUEUE_WORDS>>>,
    wake: Signal<NoopRawMutex, ()>,
    aborted: Signal<NoopRawMutex, ()>,
}

impl FskKeyer {
    pub const fn new(settings: Settings) -> Self {
        Self {
            sequencer: NoopMutex::new(RefCell::new(Sequencer::new(settings))),
            wake: Signal::new(),
            aborted: Signal::new(),
        }
    }

    pub async fn run(&self, mut output: FskOutput<'_>, mut ptt: PttOutput<'_>) -> ! {
        loop {
            self.aborted.reset();
            let (step, event, keyed) = self.with(|sequencer| {
                let (step, event) = sequencer.step(Instant::now().as_millis());
                (step, event, sequencer.status().ptt)
            });
            match event {
                Some(Event::PttOn) => info!("keyer: PTT on"),
                Some(Event::PttOff) => info!("keyer: PTT off"),
                Some(Event::Timeout) => warn!("keyer: TOT expired"),
                None => {}
            }
            ptt.set(keyed);
            let completed = match step {
                Step::Idle => {
                    output.release();
                    self.wake.wait().await;
                    continue;
                }
                Step::Send(settings, word) => {
                    debug!("keyer: FSK {=u8:#x}", word);
                    let started = || self.with(|sequencer| sequencer.started());
                    self.unless_aborted(output.send(&settings, word, started))
                        .await
                }
                Step::Hold(drive, wait) => {
                    let hold = async {
                        output.hold(drive).await;
                        match wait {
                            Wait::Millis(ms) => Timer::after_millis(ms).await,
                            Wait::Wake(limit_ms) => {
                                let timeout = async {
                                    match limit_ms {
                                        Some(ms) => Timer::after_millis(ms).await,
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

    fn with<R>(&self, f: impl FnOnce(&mut Sequencer<QUEUE_WORDS>) -> R) -> R {
        self.sequencer
            .lock(|sequencer| f(&mut sequencer.borrow_mut()))
    }
}

impl Keyer for &FskKeyer {
    fn apply(&mut self, settings: &Settings) {
        self.with(|sequencer| sequencer.apply(settings));
        self.wake.signal(());
    }

    fn enqueue(&mut self, words: &[u8], resync: Option<u8>) -> Result<(), QueueFull> {
        self.with(|sequencer| sequencer.enqueue(words, resync))?;
        self.wake.signal(());
        Ok(())
    }

    fn ptt_on(&mut self) {
        self.with(|sequencer| sequencer.ptt_on());
        self.wake.signal(());
    }

    fn ptt_off(&mut self) {
        self.with(|sequencer| sequencer.ptt_off());
        self.wake.signal(());
    }

    fn abort(&mut self) {
        let keyed = self.with(|sequencer| {
            let keyed = sequencer.status().ptt;
            sequencer.abort();
            keyed
        });
        if keyed {
            info!("keyer: PTT off (abort)");
        }
        self.aborted.signal(());
        self.wake.signal(());
    }

    fn status(&self) -> TxStatus {
        self.with(|sequencer| sequencer.status())
    }
}
