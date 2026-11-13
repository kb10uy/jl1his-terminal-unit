use embassy_futures::select::{Either, select};
use embassy_rp::gpio::Output;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use embassy_usb::Handler;

const BLINK: Duration = Duration::from_millis(250);

pub type Configured = Signal<NoopRawMutex, bool>;

pub struct UsbState<'a>(pub &'a Configured);

impl Handler for UsbState<'_> {
    fn enabled(&mut self, enabled: bool) {
        if !enabled {
            self.0.signal(false);
        }
    }

    fn reset(&mut self) {
        self.0.signal(false);
    }

    fn configured(&mut self, configured: bool) {
        self.0.signal(configured);
    }
}

pub async fn run(mut led: Output<'_>, configured: &Configured) -> ! {
    let mut is_configured = false;
    loop {
        if is_configured {
            led.set_high();
            is_configured = configured.wait().await;
        } else {
            led.toggle();
            if let Either::Second(state) = select(Timer::after(BLINK), configured.wait()).await {
                is_configured = state;
            }
        }
    }
}
