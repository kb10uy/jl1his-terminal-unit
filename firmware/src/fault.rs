use crate::keyer::release_on_fault;
use core::{
    panic::PanicInfo,
    sync::atomic::{AtomicBool, Ordering},
};
use cortex_m::{asm, interrupt, peripheral::SCB};
use cortex_m_rt::{ExceptionFrame, exception};
use defmt::{Display2Format, error};

const SHCSR_USGFAULTENA: u32 = 1 << 18;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    static PANICKED: AtomicBool = AtomicBool::new(false);

    interrupt::disable();
    release_on_fault();
    if !PANICKED.swap(true, Ordering::Relaxed) {
        error!("{}", Display2Format(info));
    }
    unsafe { (*SCB::PTR).shcsr.modify(|shcsr| shcsr & !SHCSR_USGFAULTENA) };
    asm::udf();
}

#[exception]
unsafe fn HardFault(_frame: &ExceptionFrame) -> ! {
    release_on_fault();
    loop {
        asm::nop();
    }
}
