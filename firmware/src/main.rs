#![no_std]
#![no_main]

mod microphone;

use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_rp::{
    binary_info::{
        EntryAddr, rp_cargo_version, rp_program_build_attribute, rp_program_description,
        rp_program_name,
    },
    bind_interrupts,
    gpio::{Level, Output},
    peripherals::USB,
    usb,
};
use embassy_time::Timer;
use embassy_usb::{Builder, Config};
use microphone::{Microphone, SAMPLE_RATE_HZ, SAMPLES_PER_FRAME};
use terminal_unit_core::tone::Sine;
use {defmt_rtt as _, panic_probe as _};

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [EntryAddr; 4] = [
    rp_program_name!(c"jl1his-terminal-unit"),
    rp_program_description!(c"RTTY terminal unit"),
    rp_cargo_version!(),
    rp_program_build_attribute!(),
];

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

const USB_VID: u16 = 0x1209;
const USB_PID: u16 = 0x0001;

const TEST_TONE_HZ: u32 = 1_000;
const TEST_TONE_AMPLITUDE: f32 = 0.5;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("jl1his-terminal-unit started");
    let mut led = Output::new(p.PIN_25, Level::Low);

    let mut usb_config = Config::new(USB_VID, USB_PID);
    usb_config.product = Some("RTTY Terminal Unit");

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut builder = Builder::new(
        usb::Driver::new(p.USB, Irqs),
        usb_config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],
        &mut control_buf,
    );
    let mut microphone = Microphone::new(&mut builder);
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

    let blink = async {
        loop {
            led.toggle();
            Timer::after_millis(500).await;
        }
    };

    join3(usb_device.run(), audio, blink).await;
}
