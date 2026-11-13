#![no_std]
#![no_main]

mod capture;
mod control;
mod fault;
mod keyer;
mod microphone;
mod status;

use capture::{Capture, Ring};
use defmt::{info, warn};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::join::join5;
use embassy_rp::{
    binary_info::{
        EntryAddr, rp_cargo_version, rp_program_build_attribute, rp_program_description,
        rp_program_name,
    },
    bind_interrupts,
    gpio::{Level, Output},
    peripherals::{PIO0, USB},
    pio::{self, Pio},
    usb,
};
use embassy_usb::{
    Builder, Config,
    class::cdc_acm::{CdcAcmClass, State},
};
use keyer::FskKeyer;
use microphone::Microphone;
use status::{Configured, UsbState};
use terminal_unit_core::{
    audio::{MAX_FRAME_SAMPLES, packet_len},
    settings::Settings,
};

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

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("jl1his-terminal-unit started");
    #[cfg(not(feature = "tiny2350"))]
    let (fsk_pin, ptt_pin, status_pin, active_low) = (p.PIN_17, p.PIN_16, p.PIN_15, false);
    #[cfg(feature = "tiny2350")]
    let (fsk_pin, ptt_pin, status_pin, active_low) = (p.PIN_19, p.PIN_18, p.PIN_20, true);
    let mut status_led = Output::new(status_pin, Level::Low);
    status_led.set_inversion(active_low);
    let (fsk_output, ptt_output) =
        keyer::outputs(Pio::new(p.PIO0, Irqs), fsk_pin, ptt_pin, active_low);
    let ring = cortex_m::singleton!(: Ring = Ring::new()).unwrap();
    let mut capture = Capture::new(p.ADC, p.PIN_26, p.DMA_CH0, ring);

    let mut usb_config = Config::new(USB_VID, USB_PID);
    usb_config.product = Some("JL1HIS RTTY Terminal Unit");

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut cdc_state = State::new();
    let configured = Configured::new();
    let mut usb_state = UsbState(&configured);
    let mut builder = Builder::new(
        usb::Driver::new(p.USB, Irqs),
        usb_config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],
        &mut control_buf,
    );
    builder.handler(&mut usb_state);
    let mut microphone = Microphone::new(&mut builder);
    let control_port = CdcAcmClass::new(&mut builder, &mut cdc_state, control::MAX_PACKET_SIZE);
    let mut usb_device = builder.build();

    let audio = async {
        let mut samples = [0; MAX_FRAME_SAMPLES];
        loop {
            microphone.wait_enabled().await;
            capture.resync();
            loop {
                let Some(len) = packet_len(capture.pending()) else {
                    warn!("audio: capture overrun");
                    capture.resync();
                    continue;
                };
                capture.read(&mut samples[..len]);
                if microphone.write_samples(&samples[..len]).await.is_err() {
                    break;
                }
            }
        }
    };

    let settings = Settings::default();
    let keyer = FskKeyer::new(settings);

    join5(
        usb_device.run(),
        status::run(status_led, &configured),
        audio,
        control::run(control_port, settings, &keyer),
        keyer.run(fsk_output, ptt_output),
    )
    .await;
}
