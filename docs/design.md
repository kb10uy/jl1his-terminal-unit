# RTTY Terminal Unit Design

Decisions from the design discussion. Only the workspace scaffold exists;
none of the stages below is implemented. Items marked "unverified" come from
memory and have not been checked against source code, datasheets or manuals.
Once an item is verified, remove the mark and cite the source.

## 1. Purpose

Build an RTTY terminal unit around the RP2350. The PC sees it as a composite
USB device with:

- USB Audio input (microphone direction): captures the rig's receive AF and
  sends it to the PC. Demodulation runs on the PC for now (MMTTY and the
  like).
- Serial ports: key FSK and control PTT from data sent by the PC.

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
- Audio input: the internal ADC, driven by an op-amp front end.
- Target rigs: the Yaesu FTDX10 first. The design also works with the ACC
  jack of Icom rigs.

### Firmware

- Language and framework: Rust + embassy (`embassy-rp`, `embassy-usb`)
- USB configuration: UAC (microphone, mono 48 kHz / 16 bit) + two CDC ACM
  - CDC #1 (FSK port): MMTTY opens it as a COM port for FSK. Received bytes
    are keyed as Baudot. RTS / DTR drive PTT.
  - CDC #2 (control port): line-based text commands. Changes settings and
    reports status independently of what is being sent. Later it also
    carries decoded text.

## 3. Firmware Design

### Tasks (draft)

Four tasks: the USB device, FSK transmit, control commands and audio,
connected with `embassy-sync` Channel / Signal.

### FSK Transmit

- A PIO state machine generates 5 data bits + 1.5 stop bits.
- The baud rate follows the line coding of CDC #1. Line coding carries the
  baud rate as an integer only, so 45 is read as 45.45 baud.
- USB delivers bytes in bursts that queue in a firmware buffer. When the PC
  drops RTS / DTR, PTT stays on until the buffer is empty and the last
  character has been sent in full.
- The FSK polarity (which of mark / space maps to open / closed) can be
  inverted by a control command.
- The scaffold halts on panic with `panic-halt`. Once the FSK / PTT outputs
  exist, replace it with a handler that releases both before anything else.

### Control Commands (candidates; the command set is not designed yet)

- Invert FSK polarity
- PTT delay
- Status query
- Digital gain

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

### Stages

1. Two CDC ports + PIO-based FSK / PTT (enough to transmit from MMTTY)
2. Add the UAC microphone
3. On-board demodulation

## 4. Analog Circuits

### Receive AF Input Stage

1. AC coupling
2. Attenuator / gain adjustment
3. Non-inverting amplifier (3.3 V single supply, rail-to-rail, 1.65 V bias)
4. RC anti-aliasing LPF (a few kHz to 10 kHz)
5. Series resistor + small capacitor → ADC pin

- Input impedance around 100 kΩ, so the difference in rig output impedance
  (4.7 k to 10 kΩ) can be ignored.
- A fixed gain of 3 covers both the FTDX10 and Icom rigs. With the FTDX10
  this only reaches about −11 dBFS, so a switch to 8–10× makes better use of
  the ADC's effective resolution.
- Overvoltage protection: series resistor + clamp to the supply rails
  (BAT54S or similar).
- If a transformer provides isolation, choose a 10 kΩ type. Driving a 600 Ω
  type from a 10 kΩ output loses the low end. Whether to use a transformer is
  undecided.

### Rig-Side Assumptions (all unverified)

| Rig | Connector | Output impedance | Level |
|---|---|---|---|
| FTDX10 | DATA OUT on RTTY/DATA (6-pin mini-DIN) | 10 kΩ | up to about 300 mVp-p (fixed) |
| Icom | AF output on ACC | 4.7 kΩ | 100–300 mV rms |

### FSK / PTT Outputs

- Isolated by optocoupler open-collector outputs. Both rigs are assumed to
  take an input pulled up on the rig side that activates when pulled to GND
  (unverified).
- So that nothing transmits right after power-up or while pins are
  undefined, the optocoupler drive circuit is off whenever its pin is
  undefined.

### Connector

- One general-purpose connector on the unit, with an adapter cable per rig.
- Signals: AF input, FSK, PTT and GND, plus one spare for AF output in case
  AFSK transmit is added later.

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

- [ ] 1. Whether `embassy-usb` has a UAC class for the microphone direction
  (device → PC). As recalled, only the speaker direction exists. If not,
  write the descriptors and the isochronous IN endpoint ourselves.
- [ ] 2. Whether the `embassy-rp` USB driver supports isochronous transfers.
- [ ] 3. Whether the `embassy-rp` ADC pin initialization disables the digital
  input.
- [ ] 4. Whether the `embassy-rp` ADC can capture continuously without gaps
  through DMA.
- [ ] 5. How to get the line coding (baud rate, data bits, stop bits) and
  DTR / RTS changes from `CdcAcmClass` in `embassy-usb`.
- [ ] 6. Whether Windows usbser passes 5 bit / 1.5 stop / 45 baud settings to
  the device unchanged (check on hardware with MMTTY).
- [ ] 7. The Tiny 2350 schematic: exposed GPIOs, and filtering of the ADC's
  analog supply and reference.
- [ ] 8. The effective resolution of the ADC in the RP2350 datasheet.
- [ ] 9. The manuals of the FTDX10 and each Icom model: AF output level and
  impedance, FSK / PTT input specification and polarity, pinout.

## 8. Undecided

- The pin assignment. The Tiny 2350 exposes few GPIOs. Decide whether to keep
  3–4 pins free for I2S in addition to FSK, PTT and the ADC input.
- Whether to isolate with a transformer.
- How to switch the gain (jumper or trimmer).
- The control command set.
- The USB VID / PID. During development, change the PID whenever the
  descriptor configuration changes, because Windows caches the old
  configuration.
