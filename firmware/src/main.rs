#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::binary_info::{
    EntryAddr, rp_cargo_version, rp_program_build_attribute, rp_program_description,
    rp_program_name,
};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

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
    let _peripherals = embassy_rp::init(Default::default());
    info!("jl1his-terminal-unit started");

    loop {
        Timer::after_secs(10).await;
        info!("alive");
    }
}
