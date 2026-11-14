use crate::keyer::{FskKeyer, QUEUE_WORDS};
use core::{cell::RefCell, fmt};
use defmt::info;
use embassy_futures::join::join3;
use embassy_rp::rom_data::reset_to_usb_boot;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, pipe::Pipe};
use embassy_time::{Duration, Timer, with_timeout};
use embassy_usb::{class::cdc_acm::CdcAcmClass, driver::Driver};
use terminal_unit_core::{
    control::{Action, Controller, SaveError, Storage},
    settings::Settings,
};

pub const MAX_PACKET_SIZE: u16 = 64;

const LINE_BYTES: usize = 16 * 1024;
const OUTPUT_BYTES: usize = 1024;
const REPLY_BYTES: usize = 256;

type Output = Pipe<NoopRawMutex, OUTPUT_BYTES>;

struct NoStorage;

impl Storage for NoStorage {
    fn save(&mut self, _: &Settings) -> Result<(), SaveError> {
        Err(SaveError("SAVE is not implemented yet"))
    }
}

pub async fn run<'d, D: Driver<'d>>(
    class: CdcAcmClass<'d, D>,
    settings: Settings,
    keyer: &FskKeyer,
) {
    let (mut sender, mut receiver, lines) = class.split_with_control();
    let output = Output::new();
    let controller = RefCell::new(Controller::<LINE_BYTES, QUEUE_WORDS>::new(
        settings,
        env!("CARGO_PKG_VERSION"),
    ));

    let commands = async {
        let mut keyer = keyer;
        let mut packet = [0; MAX_PACKET_SIZE as usize];
        let mut reply = Reply::new();
        loop {
            receiver.wait_connection().await;
            info!("control port connected");
            while let Ok(len) = receiver.read_packet(&mut packet).await {
                for &byte in &packet[..len] {
                    reply.clear();
                    let action =
                        controller
                            .borrow_mut()
                            .feed(byte, &mut keyer, &mut NoStorage, &mut reply);
                    if let Some(line) = reply.line()
                        && line.len() <= output.free_capacity()
                    {
                        let _ = output.try_write(line);
                    }
                    if action == Some(Action::Bootsel) {
                        reboot_to_bootsel(&output).await;
                    }
                }
            }
        }
    };

    let replies = async {
        let mut packet = [0; MAX_PACKET_SIZE as usize];
        loop {
            sender.wait_connection().await;
            loop {
                let len = output.read(&mut packet).await;
                if sender.write_packet(&packet[..len]).await.is_err() {
                    break;
                }
            }
        }
    };

    let hang_up = async {
        let mut keyer = keyer;
        let mut dtr = false;
        loop {
            lines.control_changed().await;
            let was = core::mem::replace(&mut dtr, lines.dtr());
            if was && !dtr {
                info!("control port: DTR off, aborting");
                controller.borrow_mut().abort(&mut keyer);
            }
        }
    };

    join3(commands, replies, hang_up).await;
}

async fn reboot_to_bootsel(output: &Output) {
    info!("rebooting into BOOTSEL");
    let drained = async {
        while !output.is_empty() {
            Timer::after_millis(10).await;
        }
    };
    let _ = with_timeout(Duration::from_millis(500), drained).await;
    Timer::after_millis(50).await;
    reset_to_usb_boot(0, 0);
}

struct Reply {
    buf: [u8; REPLY_BYTES],
    len: usize,
    overflowed: bool,
}

impl Reply {
    const fn new() -> Self {
        Self {
            buf: [0; REPLY_BYTES],
            len: 0,
            overflowed: false,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.overflowed = false;
    }

    fn line(&self) -> Option<&[u8]> {
        (!self.overflowed && self.len > 0).then(|| &self.buf[..self.len])
    }
}

impl fmt::Write for Reply {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        let Some(dest) = self.buf.get_mut(self.len..end) else {
            self.overflowed = true;
            return Err(fmt::Error);
        };
        dest.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}
