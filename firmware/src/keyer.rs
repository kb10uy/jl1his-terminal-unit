use core::{
    cell::RefCell,
    future::{Future, pending},
};
use defmt::{debug, info, warn};
use embassy_futures::select::{Either, select};
use embassy_rp::{
    Peri,
    clocks::clk_sys_freq,
    gpio::Level,
    pac,
    peripherals::PIO0,
    pio::{
        Config, Direction, Irq, IrqFlags, Pin, Pio, PioPin, ShiftDirection, StateMachine,
        program::{
            InstructionOperands, MovDestination, MovOperation, MovSource, SetDestination, pio_asm,
        },
    },
};
use embassy_sync::{
    blocking_mutex::{NoopMutex, raw::NoopRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Instant, Timer};
use heapless::Deque;
use terminal_unit_core::{
    baudot::LTRS,
    control::{Keyer, QueueFull, TxStatus},
    fsk::{Framing, drive},
    settings::{Diddle, Settings},
};

pub const QUEUE_WORDS: usize = 8192;

const HALF_BIT_OVERHEAD_TICKS: u32 = 4;
const SET_PINS_LOW: u16 = InstructionOperands::SET {
    destination: SetDestination::PINS,
    data: 0,
}
.encode();
const PULL: u16 = InstructionOperands::PULL {
    if_empty: false,
    block: false,
}
.encode();
const MOV_Y_OSR: u16 = InstructionOperands::MOV {
    destination: MovDestination::Y,
    op: MovOperation::None,
    source: MovSource::OSR,
}
.encode();

pub fn release_on_fault() {
    pac::PIO0.ctrl().write(|w| w.set_sm_enable(0));
    pac::PIO0.sm(0).instr().write(|w| w.set_instr(SET_PINS_LOW));
}

pub struct FskOutput<'d> {
    sm: StateMachine<'d, PIO0, 0>,
    started: Irq<'d, PIO0, 0>,
    finished: Irq<'d, PIO0, 1>,
    irq_flags: IrqFlags<'d, PIO0>,
    pin: Pin<'d, PIO0>,
    origin: u8,
    clock_hz: u32,
    framing: Option<Framing>,
    busy: Option<Duration>,
}

impl<'d> FskOutput<'d> {
    pub fn new(pio: Pio<'d, PIO0>, pin: Peri<'d, impl PioPin>) -> Self {
        let Pio {
            mut common,
            irq_flags,
            irq0,
            irq1,
            mut sm0,
            ..
        } = pio;
        let program = pio_asm!(
            ".wrap_target",
            "    pull block",
            "    irq nowait 0",
            "half_bit:",
            "    out pins, 1",
            "    mov x, y",
            "delay:",
            "    jmp x-- delay",
            "    jmp !osre half_bit",
            "    irq nowait 1",
            ".wrap",
        );
        let program = common.load_program(&program.program);
        let pin = common.make_pio_pin(pin);
        sm0.set_pins(Level::Low, &[&pin]);
        sm0.set_pin_dirs(Direction::Out, &[&pin]);

        let mut config = Config::default();
        config.use_program(&program, &[]);
        config.set_out_pins(&[&pin]);
        config.set_set_pins(&[&pin]);
        config.shift_out.direction = ShiftDirection::Right;
        sm0.set_config(&config);
        sm0.set_enable(true);

        Self {
            sm: sm0,
            started: irq0,
            finished: irq1,
            irq_flags,
            pin,
            origin: program.origin,
            clock_hz: clk_sys_freq(),
            framing: None,
            busy: None,
        }
    }

    fn release(&mut self) {
        self.sm.set_enable(false);
        self.sm.set_pins(Level::Low, &[&self.pin]);
        self.sm.clear_fifos();
        self.sm.restart();
        unsafe { self.sm.exec_jmp(self.origin) };
        self.irq_flags.clear_all(0b11);
        self.sm.set_enable(true);
        self.framing = None;
        self.busy = None;
    }

    async fn settle(&mut self) {
        if let Some(frame) = self.busy.take() {
            select(self.finished.wait(), Timer::after(frame)).await;
        }
    }

    fn while_stopped(&mut self, f: impl FnOnce(&mut Self)) {
        self.sm.set_enable(false);
        f(self);
        unsafe { self.sm.exec_jmp(self.origin) };
        self.sm.set_enable(true);
    }

    fn duration(&self, ticks: u64) -> Duration {
        Duration::from_micros(ticks * 1_000_000 / u64::from(self.clock_hz))
    }

    async fn hold(&mut self, drive: bool) {
        self.settle().await;
        self.while_stopped(|output| output.sm.set_pins(Level::from(drive), &[&output.pin]));
    }

    async fn send(&mut self, settings: &Settings, word: u8) {
        let framing = Framing::new(settings, self.clock_hz);
        if self.framing != Some(framing) {
            self.settle().await;
            self.while_stopped(|output| {
                output.sm.set_tx_threshold(framing.half_bits());
                output
                    .sm
                    .tx()
                    .push(framing.half_bit_ticks() - HALF_BIT_OVERHEAD_TICKS);
                unsafe {
                    output.sm.exec_instr(PULL);
                    output.sm.exec_instr(MOV_Y_OSR);
                }
            });
            self.framing = Some(framing);
        }
        self.sm.tx().wait_push(framing.frame(word)).await;
        self.busy = Some(self.duration(framing.ticks()));
        self.started.wait().await;
        self.irq_flags.clear(1);
        Timer::after(self.duration(framing.ticks_before_stop())).await;
    }
}

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
