# RTTY Terminal Unit Design

Decisions from the design discussion. The firmware presents the UAC
microphone and streams a test tone through it; no other part of the stages
below is implemented. Items marked "unverified" come from
memory and have not been checked against source code, datasheets or manuals.
Once an item is verified, remove the mark and cite the source.

## 1. Purpose

Build an RTTY terminal unit around the RP2350. The PC sees it as a composite
USB device with:

- USB Audio input (microphone direction): captures the rig's receive AF and
  sends it to the PC. Demodulation runs on the PC for now (MMTTY and the
  like).
- A serial port: takes text commands from the PC that key FSK, control PTT
  and change settings.

On-board demodulation on the RP2350 is a later goal.

## 2. Decided

### Hardware

- Board: Pimoroni Tiny 2350
- Chip marking: RP2350A0A2 (stepping A2, affected by erratum RP2350-E9)
- Debug: the board does not expose SWD (checked on the board). Firmware is
  flashed over USB in BOOTSEL mode with `picotool`, and there is no RTT log.
  Logs and status go out through the control port once it exists.
- On-board parts: an RGB LED on GPIO 18 (R), 19 (G) and 20 (B), and a user
  switch on GPIO 23 (pico-sdk `src/boards/include/boards/pimoroni_tiny2350.h`).
  The LED is active low: Pimoroni's `RGBLED` driver defaults to
  `Polarity::ACTIVE_LOW` (pimoroni-pico `drivers/rgbled/rgbled.hpp`) and their
  Tiny 2350 example uses that default.
- Audio input: the internal ADC, driven by an op-amp front end built on the
  MCP6022, behind an isolation transformer.
- Target rigs: the Yaesu FTDX10 first. The design also works with the ACC
  jack of Icom rigs.

### Firmware

- Language and framework: Rust + embassy (`embassy-rp`, `embassy-usb`)
- USB configuration: UAC (microphone, mono 48 kHz / 16 bit) + one CDC ACM
  port, the control port.
  - The control port takes line-based text commands. The text to transmit
    and PTT are commands like any other, in the same stream as the commands
    that change settings and query status, so their order is defined. Later
    the port also carries decoded text.
  - There is no port that takes raw Baudot at 45 baud / 5 bit with PTT on
    RTS / DTR. Its only use was to let MMTTY open the unit as a UART with
    nothing added on the PC, and it depended on Windows passing that line
    coding through unchanged, which was never checked.
  - MMTTY can still transmit through the unit with an `.fsk` add-on on the
    PC that translates to the command protocol, as JA7UDE's `tinyfsk.fsk`
    does for TinyFSK (https://www.qsl.net/ja7ude/tinyfsk/index.html). No
    such add-on is planned yet.

### License

The firmware and the rest of the software are licensed under
`MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). The license of the
hardware design files is undecided.

### USB VID / PID

- VID 0x1209 with a PID allocated by pid.codes. The PID has not been
  requested yet.
- Until it is allocated, the firmware uses the pid.codes test PIDs
  1209:0001–0010 (https://pid.codes/1209/), starting at 0001. Move to the
  next one whenever the descriptor configuration changes, because Windows
  caches the old configuration.
- To request a PID (https://pid.codes/howto/), the repository has to be
  public and carry a LICENSE file with a recognised open source license. If
  it holds both hardware and software, both need one. The request is a pull
  request to the pid.codes repository adding `org/<organisation>/index.md`
  and `1209/<PID>/index.md` for an unused PID outside the reserved 0xxx and
  1xxx ranges.

## 3. Firmware Design

### Tasks (draft)

Four tasks: the USB device, FSK transmit, control commands and audio,
connected with `embassy-sync` Channel / Signal.

### FSK Transmit

- A PIO state machine generates 5 data bits + 1.5 stop bits.
- The baud rate is set by a control command, so 45.45 baud is given exactly.
  The firmware ignores the line coding of the control port.
- A transmit command carries text. The firmware converts it to Baudot and
  inserts the LTRS / FIGS shifts. The conversion lives in `core` and is
  tested on the host.
- The FSK transmit task takes Baudot codes and PTT requests from a queue and
  does not know what filled it. The command parser is the only source now; a
  raw port could be added later as a second one without changing the task.
- PTT is keyed and released by commands. A release takes effect only after
  the queue is empty and the last character has been sent in full.
- The control port never stops reading because the transmit queue is full.
  A transmit command that does not fit is answered with an error, so that an
  abort never waits behind queued text.
- The FSK polarity (which of mark / space maps to open / closed) can be
  inverted by a control command.
- The scaffold halts on panic with `panic-halt`. Once the FSK / PTT outputs
  exist, replace it with a handler that releases both before anything else.

### Control Commands (candidates; the command set is not designed yet)

- Transmit text
- PTT on / off
- Abort the transmission and discard the queue
- Baud rate
- Invert FSK polarity
- PTT delay
- Status query
- Digital gain

Toward the PC the port carries command replies, logs, an echo of the
characters as they are sent and, later, decoded text. Each line is tagged
with its kind. The format is part of the command set.

### Audio

- The ADC is free-running, paced by its internal clock divider and read
  through DMA. No software timer triggers conversions.
- The ADC clock is 48 MHz. Both 48 kHz (÷1000) and 192 kHz (÷250) are
  integer divisions.
- Oversample at 192 kHz or more and decimate to 48 kHz. A first-order RC is
  expected to be enough for anti-aliasing.
- The UAC capture endpoint is asynchronous and sends samples on the device's
  clock.
- Samples pass through an interface boundary inside the firmware, so that an
  external I2S ADC (such as the PCM1808) can replace the internal ADC later.

### UAC Microphone

`embassy-usb` has no UAC microphone class, so `firmware/src/microphone.rs`
writes the UAC 1.0 descriptors itself.

- Audio control interface: Input Terminal (Microphone, 0x0201) → Output
  Terminal (USB Streaming, 0x0101). There is no Feature Unit, so the host
  offers no volume or mute control and the level stays as the analog front
  end sets it. No class-specific requests are handled.
- Audio streaming interface: alternate setting 0 has no endpoint, alternate
  setting 1 carries Format Type I PCM, mono, 16 bit, one discrete rate of
  48 kHz.
- Isochronous IN endpoint, asynchronous, `bInterval` 1. `wMaxPacketSize` is
  98 bytes (49 samples), so that the device clock can send an extra sample
  in some frames once the ADC paces the stream. The endpoint descriptor has
  no sampling frequency control.
- Until the ADC path exists, the stream carries a 1 kHz sine at −6 dBFS from
  `terminal_unit_core::tone::Sine`, 48 samples per packet. Each write waits
  until the host has taken the previous packet, so the host's frame clock
  paces it.

### Stages

1. The control port + PIO-based FSK / PTT (enough to transmit by typing
   commands into a serial terminal)
2. Add the UAC microphone
3. On-board demodulation

The UAC microphone was brought up ahead of stage 1 with a test tone in place
of the ADC.

## 4. Analog Circuits

The schematic is `hardware/terminal-unit.kicad_sch`, drawn with KiCad 10. Its
component values are starting points for the prototype, not decisions. The
Tiny 2350 is not on the sheet yet, so the GPIO and ADC connections end in
global labels.

### Receive AF Input Stage

1. Isolation transformer (10 kΩ : 10 kΩ)
2. AC coupling
3. Attenuator / gain adjustment
4. Non-inverting amplifier (3.3 V single supply, rail-to-rail, 1.65 V bias)
5. RC anti-aliasing LPF (a few kHz to 10 kHz)
6. Series resistor + small capacitor → ADC pin

- The op-amp is the MCP6022, a dual with rail-to-rail input and output. It
  runs from 2.5 V to 5.5 V, has a gain bandwidth product of 10 MHz and an
  input noise density of 8.7 nV/√Hz at 10 kHz, swings to within 15 mV of
  ground and 20 mV of the supply into 10 kΩ, draws 1 mA per amplifier and has
  an input bias current of 1 pA. It comes in 8-lead PDIP, SOIC and TSSOP
  (Microchip DS20001685E, front page and section 1.0).
- Input impedance around 100 kΩ, so the difference in rig output impedance
  (4.7 k to 10 kΩ) can be ignored.
- A fixed gain of 3 covers both the FTDX10 and Icom rigs. With the FTDX10
  this only reaches about −11 dBFS, so a switch to 8–10× makes better use of
  the ADC's effective resolution.
- Overvoltage protection: series resistor + clamp to the supply rails
  (BAT54S or similar).
- A transformer isolates the AF input from the rig. With the optocouplers on
  FSK and PTT, no conductor then joins the rig's ground to the USB ground, so
  no ground loop forms and RF common-mode current on transmit has no path
  through the USB cable. It is a 10 kΩ type: driving a 600 Ω type from a
  10 kΩ output loses the low end.
- The transformer is the Sansui ST-78 made by Hashimoto Denki: 10 kΩ to
  10 kΩ with a centre-tapped secondary, DC resistance 330 Ω and 420 Ω ±20 %,
  insulation resistance 100 MΩ or more at 100 V DC, case CB-19, 13 g. The
  catalogue lists the case with leads and with PCB pins and gives the
  frequency response as a graph from 0.2 kHz to 10 kHz (Hashimoto Denki,
  "トランジスタ用小型トランス" ST series catalogue,
  `akizukidenshi.com/goodsaffix/st.pdf`). Which of the two cases to use is not
  chosen yet.

### Rig-Side Assumptions (all unverified)

| Rig | Connector | Output impedance | Level |
|---|---|---|---|
| FTDX10 | DATA OUT on RTTY/DATA (6-pin mini-DIN) | 10 kΩ | up to about 300 mVp-p (fixed) |
| Icom | AF output on ACC | 4.7 kΩ | 100–300 mV rms |

### FSK / PTT Outputs

- Isolated by optocoupler open-collector outputs. Both rigs are assumed to
  take an input pulled up on the rig side that activates when pulled to GND
  (unverified).
- Each output is a Toshiba TLP785 of rank GB, a phototransistor coupler in
  DIP4. Rank GB has a current transfer ratio of 100 % to 600 % at
  IF = 5 mA and VCE = 5 V. The saturation voltage is at most 0.4 V at
  IC = 2.4 mA and IF = 8 mA, the collector-emitter rating is 80 V, the LED
  forward voltage is 1.0 V to 1.3 V at 10 mA and the isolation voltage is
  5000 Vrms. Saturated with a 1.9 kΩ load, it turns on in 1.5 µs and off in
  50 µs, typically (Toshiba TLP785 datasheet, 2012-02-20). A coupler with a
  logic output would be faster but needs a supply on the rig side, which the
  rig's jack does not provide.
- So that nothing transmits right after power-up or while pins are
  undefined, the optocoupler drive circuit is off whenever its pin is
  undefined.

### Connector

- The unit has one 6-pin mini-DIN jack, mounted on a pitch-conversion board.
  Its pinout mirrors the RTTY/DATA jack of the FTDX10, so a straight cable
  with all six conductors joins the two. Other rigs take an adapter cable.

| Pin | Unit | FTDX10 RTTY/DATA |
|---|---|---|
| 1 | AF output, spare in case AFSK transmit is added later | DATA IN |
| 2 | Rig ground | GND |
| 3 | PTT | DATA PTT |
| 4 | FSK | SHIFT |
| 5 | AF input | DATA OUT |
| 6 | Not connected | SQL OUT |

- The FTDX10 column comes from the page "RTTY (FSK) Operation" of the FTDX10
  Operation Manual as transcribed by ManualsLib. Confirm it against the
  manual itself under item 8 of "To Check Before Implementing".
- Pin 2 is the return for the AF and for the keying current alike.
- The shell of the jack and the shield of the cable connect to the rig's
  ground only. Joining them to the USB ground or the USB shield, directly or
  through a metal enclosure, defeats the isolation.

## 5. Working Around RP2350-E9 (Stepping A2)

- Do not rely on internal pull-downs. Inputs use pull-ups and are pulled to
  GND. Where a fixed Low is needed, use an external resistor of 8.2 kΩ or
  less.
- Disable the digital input on pins used by the ADC (required for noise as
  well).
- FSK / PTT outputs and I2S inputs are not affected.

## 6. Expected Accuracy of the Internal ADC

- Recalled as a little over 9 effective bits (SNR about 56 dB) against the
  nominal 12 bits (unverified).
- Enough for RTTY, given the in-band dynamic range of the rig's AF,
  oversampling and the processing gain of narrow demodulation filters.
- What remains is distortion from nonlinearity, and spurs from the 1 kHz USB
  frame period, core supply switching and coupling from digital circuits.
  These may show as lines in the PC's waterfall.
- Evaluation: once USB Audio works, record on the PC and look at an FFT.
  Measure the noise floor and spurs with no signal, and SINAD with a sine
  wave near full scale.

## 7. To Check Before Implementing

When an item is checked, tick it and write the result and its source (crate
version and source location, datasheet section, manual page) under it.

- [x] 1. Whether `embassy-usb` has a UAC class for the microphone direction
  (device → PC). As recalled, only the speaker direction exists. If not,
  write the descriptors and the isochronous IN endpoint ourselves.
  - No. `embassy-usb` 0.6.0 `src/class/uac1/mod.rs` declares only
    `pub mod speaker`, and its `class_codes` and `terminal_type` modules are
    private. The descriptors are written in `firmware/src/microphone.rs` with
    the public `InterfaceAltBuilder::descriptor` and
    `endpoint_isochronous_in` (`src/builder.rs`).
- [x] 2. Whether the `embassy-rp` USB driver supports isochronous transfers.
  - Yes. `embassy-rp` 0.10.0 `src/usb.rs` `alloc_endpoint` accepts
    `EndpointType::Isochronous` with a packet size up to 1023 bytes and sets
    the endpoint type to `ISOCHRONOUS`. Endpoints are single-buffered, and
    `EndpointIn::write` waits until the previous packet has been taken.
  - `write` flips the data PID on every packet, also for isochronous
    endpoints, where full-speed USB uses DATA0 only. Check on hardware that
    the host accepts the stream.
- [ ] 3. Whether the `embassy-rp` ADC pin initialization disables the digital
  input.
- [ ] 4. Whether the `embassy-rp` ADC can capture continuously without gaps
  through DMA.
- [ ] 5. How `CdcAcmClass` in `embassy-usb` reports DTR changes and
  SEND_BREAK, to tell when the PC has closed the port and to offer an abort
  that does not travel through the bulk pipe.
- [ ] 6. The Tiny 2350 schematic: exposed GPIOs, and filtering of the ADC's
  analog supply and reference.
- [ ] 7. The effective resolution of the ADC in the RP2350 datasheet.
- [ ] 8. The manuals of the FTDX10 and each Icom model: AF output level and
  impedance, FSK / PTT input specification and polarity, pinout.

## 8. Undecided

- The pin assignment. The Tiny 2350 exposes few GPIOs. Decide whether to keep
  3–4 pins free for I2S in addition to FSK, PTT and the ADC input.
- How to switch the gain (jumper or trimmer).
- The load on the transformer secondary. 10 kΩ matches the transformer but
  halves the level from a 10 kΩ rig output; around 100 kΩ keeps the level but
  may peak at the high end. Measure both on the prototype.
- The control command set, including whether it follows the TinyFSK protocol
  so that the existing `tinyfsk.fsk` can drive the unit from MMTTY. The
  TinyFSK protocol has not been checked against its documentation.
- What happens to a transmission in progress when the PC closes the control
  port, and whether a DTR drop or SEND_BREAK aborts it.
- The license of the hardware design files. pid.codes requires one once
  they are in the repository (see "USB VID / PID").
