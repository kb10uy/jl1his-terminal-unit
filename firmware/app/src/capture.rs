use core::{marker::PhantomData, mem::align_of, ptr};
use embassy_rp::{
    Peri,
    adc::{self, Adc, Blocking, Channel},
    gpio::Pull,
    pac::{
        self,
        dma::vals::{DataSize, TransCountMode, TreqSel},
    },
    peripherals::{ADC, DMA_CH0, PIN_26},
};
use terminal_unit_core::audio::{CAPTURE_RATE_HZ, DECIMATION, Decimator, TARGET_BACKLOG};

const ADC_CLOCK_HZ: u32 = 48_000_000;
const ADC_DIV_INT: u16 = (ADC_CLOCK_HZ / CAPTURE_RATE_HZ - 1) as u16;
const DMA_CHANNEL: u8 = 0;
const RING_BITS: u8 = 12;
const RING_SAMPLES: usize = (1 << RING_BITS) / size_of::<u16>();

const _: () = assert!(ADC_CLOCK_HZ.is_multiple_of(CAPTURE_RATE_HZ));
const _: () = assert!(align_of::<Ring>() == 1 << RING_BITS);

#[repr(C, align(4096))]
pub struct Ring([u16; RING_SAMPLES]);

impl Ring {
    pub const fn new() -> Self {
        Self([0; RING_SAMPLES])
    }
}

pub struct Capture<'d> {
    _adc: Adc<'d, Blocking>,
    _channel: Channel<'d>,
    _dma: Peri<'d, DMA_CH0>,
    ring: *const u16,
    _ring: PhantomData<&'d mut Ring>,
    read: usize,
    decimator: Decimator,
}

impl<'d> Capture<'d> {
    pub fn new(
        adc: Peri<'d, ADC>,
        pin: Peri<'d, PIN_26>,
        dma: Peri<'d, DMA_CH0>,
        ring: &'d mut Ring,
    ) -> Self {
        let adc = Adc::new_blocking(adc, adc::Config::default());
        let channel = Channel::new_pin(pin, Pull::None);
        let ring = ring.0.as_mut_ptr();

        let ch = pac::DMA.ch(DMA_CHANNEL.into());
        ch.read_addr().write_value(pac::ADC.fifo().as_ptr() as u32);
        ch.write_addr().write_value(ring as u32);
        ch.trans_count().write(|w| {
            w.set_mode(TransCountMode::ENDLESS);
            w.set_count(1);
        });
        ch.ctrl_trig().write(|w| {
            w.set_data_size(DataSize::SIZE_HALFWORD);
            w.set_incr_write(true);
            w.set_ring_size(RING_BITS);
            w.set_ring_sel(true);
            w.set_chain_to(DMA_CHANNEL);
            w.set_treq_sel(TreqSel::ADC);
            w.set_irq_quiet(true);
            w.set_en(true);
        });

        pac::ADC.cs().modify(|w| {
            w.set_ainsel(0);
            w.set_rrobin(0);
        });
        pac::ADC.fcs().write(|w| {
            w.set_en(true);
            w.set_dreq_en(true);
            w.set_thresh(1);
        });
        pac::ADC.div().write(|w| w.set_int(ADC_DIV_INT));
        pac::ADC.cs().modify(|w| w.set_start_many(true));

        Self {
            _adc: adc,
            _channel: channel,
            _dma: dma,
            ring,
            _ring: PhantomData,
            read: 0,
            decimator: Decimator::new(),
        }
    }

    fn written(&self) -> usize {
        let address = pac::DMA.ch(DMA_CHANNEL.into()).write_addr().read() as usize;
        address.wrapping_sub(self.ring as usize) / size_of::<u16>() % RING_SAMPLES
    }

    pub fn pending(&self) -> usize {
        (self.written() + RING_SAMPLES - self.read) % RING_SAMPLES / DECIMATION
    }

    pub fn resync(&mut self) {
        self.read = (self.written() + RING_SAMPLES - TARGET_BACKLOG * DECIMATION) % RING_SAMPLES;
    }

    pub fn read(&mut self, samples: &mut [i16]) {
        for sample in samples {
            *sample = loop {
                let raw = unsafe { ptr::read_volatile(self.ring.add(self.read)) };
                self.read = (self.read + 1) % RING_SAMPLES;
                if let Some(output) = self.decimator.push(raw) {
                    break output;
                }
            };
        }
    }
}

impl Drop for Capture<'_> {
    fn drop(&mut self) {
        pac::ADC.cs().modify(|w| w.set_start_many(false));
        pac::DMA
            .chan_abort()
            .write(|w| w.set_chan_abort(1 << DMA_CHANNEL));
        while pac::DMA.ch(DMA_CHANNEL.into()).ctrl_trig().read().busy() {}
    }
}
