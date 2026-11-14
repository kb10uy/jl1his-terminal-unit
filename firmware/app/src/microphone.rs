use embassy_usb::{
    Builder,
    descriptor::{SynchronizationType, UsageType},
    driver::{Driver, Endpoint, EndpointError, EndpointIn},
};
use terminal_unit_core::audio::{MAX_FRAME_SAMPLES, SAMPLE_RATE_HZ};

const BYTES_PER_SAMPLE: usize = 2;
const MAX_PACKET_SIZE: usize = MAX_FRAME_SAMPLES * BYTES_PER_SAMPLE;

const AUDIO: u8 = 0x01;
const AUDIOCONTROL: u8 = 0x01;
const AUDIOSTREAMING: u8 = 0x02;
const PROTOCOL_NONE: u8 = 0x00;

const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;

const HEADER: u8 = 0x01;
const INPUT_TERMINAL: u8 = 0x02;
const OUTPUT_TERMINAL: u8 = 0x03;
const AS_GENERAL: u8 = 0x01;
const FORMAT_TYPE: u8 = 0x02;
const EP_GENERAL: u8 = 0x01;

const BCD_ADC: u16 = 0x0100;
const MICROPHONE: u16 = 0x0201;
const USB_STREAMING: u16 = 0x0101;
const FORMAT_TYPE_I: u8 = 0x01;
const PCM: u16 = 0x0001;

const INPUT_TERMINAL_ID: u8 = 1;
const OUTPUT_TERMINAL_ID: u8 = 2;
const NO_TERMINAL: u8 = 0;
const NO_STRING: u8 = 0;
const STREAMING_INTERFACES: u8 = 1;
const CHANNELS: u8 = 1;
const CHANNEL_CONFIG: u16 = 0x0000;
const DELAY_FRAMES: u8 = 1;
const SAMPLE_RATES: u8 = 1;
const REFRESH: u8 = 0;
const SYNCH_ADDRESS: u8 = 0;
const NO_CONTROLS: u8 = 0;
const LOCK_DELAY_UNITS: u8 = 0;
const LOCK_DELAY: u16 = 0;

pub struct Microphone<'d, D: Driver<'d>> {
    endpoint: D::EndpointIn,
}

impl<'d, D: Driver<'d>> Microphone<'d, D> {
    pub fn new(builder: &mut Builder<'d, D>) -> Self {
        let mut function = builder.function(AUDIO, AUDIOCONTROL, PROTOCOL_NONE);

        {
            let mut interface = function.interface();
            let streaming_interface = u8::from(interface.interface_number()) + 1;
            let mut alt = interface.alt_setting(AUDIO, AUDIOCONTROL, PROTOCOL_NONE, None);

            let input_terminal = [
                INPUT_TERMINAL,
                INPUT_TERMINAL_ID,
                MICROPHONE as u8,
                (MICROPHONE >> 8) as u8,
                NO_TERMINAL,
                CHANNELS,
                CHANNEL_CONFIG as u8,
                (CHANNEL_CONFIG >> 8) as u8,
                NO_STRING,
                NO_STRING,
            ];
            let output_terminal = [
                OUTPUT_TERMINAL,
                OUTPUT_TERMINAL_ID,
                USB_STREAMING as u8,
                (USB_STREAMING >> 8) as u8,
                NO_TERMINAL,
                INPUT_TERMINAL_ID,
                NO_STRING,
            ];
            const HEADER_LENGTH: usize = 9;
            let total_length = HEADER_LENGTH + input_terminal.len() + 2 + output_terminal.len() + 2;

            alt.descriptor(
                CS_INTERFACE,
                &[
                    HEADER,
                    BCD_ADC as u8,
                    (BCD_ADC >> 8) as u8,
                    total_length as u8,
                    (total_length >> 8) as u8,
                    STREAMING_INTERFACES,
                    streaming_interface,
                ],
            );
            alt.descriptor(CS_INTERFACE, &input_terminal);
            alt.descriptor(CS_INTERFACE, &output_terminal);
        }

        let mut interface = function.interface();
        interface.alt_setting(AUDIO, AUDIOSTREAMING, PROTOCOL_NONE, None);
        let mut alt = interface.alt_setting(AUDIO, AUDIOSTREAMING, PROTOCOL_NONE, None);

        alt.descriptor(
            CS_INTERFACE,
            &[
                AS_GENERAL,
                OUTPUT_TERMINAL_ID,
                DELAY_FRAMES,
                PCM as u8,
                (PCM >> 8) as u8,
            ],
        );
        alt.descriptor(
            CS_INTERFACE,
            &[
                FORMAT_TYPE,
                FORMAT_TYPE_I,
                CHANNELS,
                BYTES_PER_SAMPLE as u8,
                (BYTES_PER_SAMPLE * 8) as u8,
                SAMPLE_RATES,
                SAMPLE_RATE_HZ as u8,
                (SAMPLE_RATE_HZ >> 8) as u8,
                (SAMPLE_RATE_HZ >> 16) as u8,
            ],
        );
        let endpoint = alt.endpoint_isochronous_in(
            None,
            MAX_PACKET_SIZE as u16,
            1,
            SynchronizationType::Asynchronous,
            UsageType::DataEndpoint,
            &[REFRESH, SYNCH_ADDRESS],
        );
        alt.descriptor(
            CS_ENDPOINT,
            &[
                EP_GENERAL,
                NO_CONTROLS,
                LOCK_DELAY_UNITS,
                LOCK_DELAY as u8,
                (LOCK_DELAY >> 8) as u8,
            ],
        );

        Self { endpoint }
    }

    pub async fn wait_enabled(&mut self) {
        self.endpoint.wait_enabled().await;
    }

    pub async fn write_samples(&mut self, samples: &[i16]) -> Result<(), EndpointError> {
        let length = samples.len() * BYTES_PER_SAMPLE;
        if length > MAX_PACKET_SIZE {
            return Err(EndpointError::BufferOverflow);
        }

        let mut packet = [0; MAX_PACKET_SIZE];
        for (bytes, sample) in packet
            .as_chunks_mut::<BYTES_PER_SAMPLE>()
            .0
            .iter_mut()
            .zip(samples)
        {
            bytes.copy_from_slice(&sample.to_le_bytes());
        }
        self.endpoint.write(&packet[..length]).await
    }
}
