#![no_std]
#![no_main]

mod control;
mod fault;
mod keyer;
mod microphone;

use defmt::info;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::join::join4;
use embassy_rp::{
    binary_info::{
        EntryAddr, rp_cargo_version, rp_program_build_attribute, rp_program_description,
        rp_program_name,
    },
    bind_interrupts,
    peripherals::{PIO0, USB},
    pio::{self, Pio},
    usb,
};
use embassy_usb::{
    Builder, Config,
    class::cdc_acm::{CdcAcmClass, State},
};
use keyer::{FskKeyer, FskOutput};
use microphone::{Microphone, SAMPLE_RATE_HZ, SAMPLES_PER_FRAME};
use terminal_unit_core::{settings::Settings, tone::Sine};

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [EntryAddr; 4] = [
    rp_program_name!(c"jl1his-terminal-unit"),
    rp_program_description!(c"RTTY terminal unit"),
    rp_cargo_version!(),
    rp_program_build_attribute!(),
];

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

const USB_VID: u16 = 0x1209;
const USB_PID: u16 = 0x0002;

const TEST_TONE_HZ: u32 = 1_000;
const TEST_TONE_AMPLITUDE: f32 = 0.5;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("jl1his-terminal-unit started");
    let fsk_output = FskOutput::new(Pio::new(p.PIO0, Irqs), p.PIN_25);

    let mut usb_config = Config::new(USB_VID, USB_PID);
    usb_config.product = Some("RTTY Terminal Unit");

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut cdc_state = State::new();
    let mut builder = Builder::new(
        usb::Driver::new(p.USB, Irqs),
        usb_config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],
        &mut control_buf,
    );
    let mut microphone = Microphone::new(&mut builder);
    let control_port = CdcAcmClass::new(&mut builder, &mut cdc_state, control::MAX_PACKET_SIZE);
    let mut usb_device = builder.build();

    let audio = async {
        let mut tone = Sine::new(TEST_TONE_HZ, SAMPLE_RATE_HZ, TEST_TONE_AMPLITUDE);
        let mut samples = [0; SAMPLES_PER_FRAME];
        loop {
            microphone.wait_enabled().await;
            loop {
                samples.fill_with(|| tone.next_sample());
                if microphone.write_samples(&samples).await.is_err() {
                    break;
                }
            }
        }
    };

    let settings = Settings::default();
    let keyer = FskKeyer::new(settings);

    join4(
        usb_device.run(),
        audio,
        control::run(control_port, settings, &keyer),
        keyer.run(fsk_output),
    )
    .await;
}
