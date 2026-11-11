use embassy_futures::select::select;
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
use embassy_time::{Duration, Timer};
use terminal_unit_core::{fsk::Framing, settings::Settings};

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
    pub fn new(pio: Pio<'d, PIO0>, pin: Peri<'d, impl PioPin>, active_low: bool) -> Self {
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
        let mut pin = common.make_pio_pin(pin);
        pin.set_output_inversion(active_low);
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

    pub(super) fn release(&mut self) {
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

    pub(super) async fn hold(&mut self, drive: bool) {
        self.settle().await;
        self.while_stopped(|output| output.sm.set_pins(Level::from(drive), &[&output.pin]));
    }

    pub(super) async fn send(&mut self, settings: &Settings, word: u8) {
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
