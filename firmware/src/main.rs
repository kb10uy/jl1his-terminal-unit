#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::{
    binary_info::{
        EntryAddr, rp_cargo_version, rp_program_build_attribute, rp_program_description,
        rp_program_name,
    },
    gpio::{Level, Output},
};
use embassy_time::Timer;
use panic_halt as _;

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [EntryAddr; 4] = [
    rp_program_name!(c"jl1his-terminal-unit"),
    rp_program_description!(c"RTTY terminal unit"),
    rp_cargo_version!(),
    rp_program_build_attribute!(),
];

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let mut led_green = Output::new(p.PIN_19, Level::High);

    loop {
        led_green.toggle();
        Timer::after_millis(500).await;
    }
}
