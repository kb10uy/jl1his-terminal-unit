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

- Board: Raspberry Pi Pico 2, an RP2350A with 4 MB of flash. Unless another
  source is given, the board facts in this document come from the Pico 2
  datasheet (RP-008299-DS-3, release of 2026-07-03),
  `pip-assets.raspberrypi.com/categories/1005-raspberry-pi-pico-2/documents/RP-008299-DS-3-pico-2-datasheet.pdf`.
- The firmware so far was brought up on a Pimoroni Tiny 2350 and is not
  ported yet. It still blinks that board's green LED on GPIO 19, which is
  active low; the Tiny 2350 has an RGB LED on GPIO 18, 19 and 20 (pico-sdk
  `src/boards/include/boards/pimoroni_tiny2350.h`, pimoroni-pico
  `drivers/rgbled/rgbled.hpp`).
- Chip stepping: not known until the Pico 2 is at hand. The Tiny 2350 is
  marked RP2350A0A2, stepping A2, which erratum RP2350-E9 affects. The design
  keeps the E9 workarounds either way.
- Debug: the Pico 2 brings SWD out on a 3-pin debug header (section 5.8), so
  a debug probe and an RTT log become possible. None is set up yet: firmware
  is flashed over USB in BOOTSEL mode with `picotool`. Logs and status go
  out through the control port once it exists.
- Pins: the headers expose 26 GPIOs, three of them ADC inputs. GPIO 23
  drives the regulator's power-save pin, GPIO 24 senses VBUS, GPIO 25
  drives the user LED and GPIO 29 measures VSYS / 3 (section 3.1).
- Pin assignment: the receive AF goes to GPIO 26 (ADC0, pin 31), FSK comes
  from GPIO 17 (pin 22) and PTT from GPIO 16 (pin 21). GPIO 26 sits beside
  AGND, and GPIO 16 and 17 sit at the end of the board next to a GND pin,
  which keeps the wiring of the perfboard layout short.
- Power: 3V3 comes from an RT6150 buck-boost switching regulator. The
  datasheet recommends keeping external loads on the 3V3 pin under 300 mA.
  With GPIO 23 low, the default, the regulator runs in PFM mode; driving it
  high forces PWM mode, which the datasheet says greatly reduces the ripple
  at light load (sections 5.3 and 5.4).
- ADC supply and reference: ADC_AVDD is the 3V3 rail through 201 Ω into
  2.2 µF and is brought out as ADC_VREF. AGND is the ground reference of
  GPIO 26–29. The datasheet names the weak points: the accuracy of the
  regulator, the limited filtering, and an offset of about 30 mV from the
  ADC's own current. An external 3.0 V shunt reference on ADC_VREF is the
  suggested improvement (section 5.3).
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

- A PIO state machine generates each character with the data bits set by
  `SET BITS` (5 to 8) and the stop bits set by `SET STOP` (1, 1.5 or 2).
- The baud rate is set by a control command, so 45.45 baud is given exactly.
  The firmware ignores the line coding of the control port.
- A transmit command carries text. The firmware converts it to Baudot and
  inserts the LTRS / FIGS shifts. The conversion lives in `core` and is
  tested on the host.
- The FSK transmit task takes code words and PTT requests from a queue and
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

### Control Port

The protocol of the control port, its commands, settings and limits, is in
`control-port.md`.

### Audio

- The ADC is free-running, paced by its internal clock divider and read
  through DMA. No software timer triggers conversions.
- The ADC clock is 48 MHz. Both 48 kHz (÷1000) and 192 kHz (÷250) are
  integer divisions.
- Oversample at 192 kHz or more and low-pass filter before decimating to
  48 kHz. The analog RC alone does not provide enough rejection for direct
  48 kHz capture. See `circuit-review.md` for the calculated response.
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
Pico 2 is on the sheet as A1, drawn with KiCad's `RaspberryPi_Pico` symbol,
which also covers the Pico 2, and global labels join it to the circuits.

The USB side has two ground nets. GNDA carries the grounds of the receive AF
stage and of the op-amp supply and returns to AGND on pin 33. GND carries
the grounds of the FSK / PTT drivers and returns to the GND pins. The two
join only inside the Pico 2. The 3V3 pin, the GND pins and AGND are power
outputs in the symbol, so +3V3, GND and GNDA need no PWR_FLAG.

### Receive AF Input Stage

1. AC coupling on the rig side (C1)
2. Isolation transformer (10 kΩ : 10 kΩ), with one secondary end at VBIAS
3. Non-inverting amplifier with trimmer-set gain (3.3 V single supply,
   rail-to-rail, 1.65 V bias)
4. RC anti-aliasing LPF (a few kHz to 10 kHz)
5. Series resistor + small capacitor → ADC pin

- The op-amp is the MCP6022, a dual with rail-to-rail input and output. It
  runs from 2.5 V to 5.5 V, has a gain bandwidth product of 10 MHz and an
  input noise density of 8.7 nV/√Hz at 10 kHz, swings to within 15 mV of
  ground and 20 mV of the supply into 10 kΩ, draws 1 mA per amplifier and has
  an input bias current of 1 pA. It comes in 8-lead PDIP, SOIC and TSSOP
  (Microchip DS20001685E, front page and section 1.0).
- The schematic currently loads the transformer secondary with R4 = 10 kΩ.
  This is a provisional value; the choice between 10 kΩ and around 100 kΩ
  remains undecided. At 10 kΩ, the rig's output impedance cannot be ignored.
- A trimmer in the feedback path sets the gain between 3 and 23, so the level
  is adjusted once per rig without changing parts. This is the midband AC
  gain; R11 and C7 make the DC gain unity. Using the unverified FTDX10
  assumptions below and R4 = 10 kΩ, a gain around 16–17 gives about
  2.4 Vp-p at the ADC, allowing for the transformer's winding resistance
  but not its magnetizing inductance or leakage. Measure the actual level
  before choosing the gain. See `circuit-review.md`.
- The trimmer RV1 is a Bourns 3296W. The layout uses KiCad's footprint
  `Potentiometer_Bourns_3296W_Vertical`, three pins in a row 2.54 mm apart.
- Overvoltage protection: a series resistor, then two BAT43 Schottky diodes
  in DO-35 that clamp the op-amp input to the supply rails. The BAT43
  withstands 30 V, leaks at most 0.5 µA at 25 V, drops 0.26 V to 0.33 V at
  2 mA and has 7 pF at 1 V (BAT42 / BAT43 datasheet,
  `akizukidenshi.com/goodsaffix/bat43.pdf`).
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
  `akizukidenshi.com/goodsaffix/st.pdf`). The unit uses the case with leads,
  which reach any hole of a universal board.

### Rig-Side Specifications and Assumptions

| Rig | Connector | Output impedance | Level |
|---|---|---|---|
| FTDX10 | DATA OUT on RTTY/DATA (6-pin mini-DIN) | 10 kΩ (unverified) | up to about 300 mVp-p (unverified) |
| Icom IC-7300 | AF/IF output on ACC, pin 12, selected as AF | 4.7 kΩ | 100–300 mV rms, adjustable |

- The FTDX10 Operation Manual (2307A-LS-1), p. 64, confirms the connector
  mapping. Its p. 91 documents DATA OUT LEVEL adjustment, so "fixed level"
  must not be interpreted as independent of menu settings. The numerical AF
  specifications and FSK / PTT electrical limits above remain unverified.
  Source: [Yaesu manual](https://yaesu.com/Files/4CB893D7-1018-01AF-FA97E9E9AD48B50C/FTDX10_OM_ENG_EH076H202_2307A-LS-1.pdf).
- The IC-7300 Basic Manual, pp. 12-1–12-2, specifies the AF values above
  and approximately 200 mV rms at the default 50% output-level setting.
  These specifications are for the IC-7300, not all Icom rigs.
  Source: [Icom manual](https://www.icomeurope.com/wp-content/uploads/2019/07/IC-7300_ENG_Basic_6.pdf).

### FSK / PTT Outputs

- Isolated by optocoupler open-collector outputs. Both rigs are assumed to
  take an input pulled up on the rig side that activates when pulled to GND.
  This is confirmed for the IC-7300 SEND input; its FSKK input specifies
  Low below 0.6 V and current below 2 mA. SEND lists a TX input range of
  −0.5 V to +0.8 V and maximum current flow of 20 mA. Check actual sink
  current for each rig rather than using the FSK figure for PTT. The FTDX10
  electrical specifications remain unverified (Icom manual, p. 12-1).
- Each output is a Toshiba TLP785 of rank GB, a phototransistor coupler in
  DIP4. Rank GB has a current transfer ratio of 100 % to 600 % at
  IF = 5 mA and VCE = 5 V. The saturation voltage is at most 0.4 V at
  IC = 2.4 mA and IF = 8 mA, the collector-emitter rating is 80 V, the LED
  forward voltage is 1.0 V to 1.3 V at 10 mA and the isolation voltage is
  5000 Vrms. Saturated with a 1.9 kΩ load, it turns on in 1.5 µs and off in
  50 µs, typically (Toshiba TLP785 datasheet, 2012-02-20). A coupler with a
  logic output would be faster but needs a supply on the rig side, which the
  rig's jack does not provide.
- Each LED runs from 3V3 through 150 Ω (R6 / R7), and a 2SC1815 (Q1 / Q2)
  switches its cathode to ground. The LED current is about 14 mA nominally.
  With the supply 5 % low, VF = 1.3 V, VCE(sat) = 0.25 V and the resistor
  1 % high it is about 10.5 mA, above the 8 mA at which the TLP785 is
  specified to hold 0.4 V at 2.4 mA. This is an estimate rather than a
  guarantee: the VF limit is specified at 10 mA and the 2SC1815 saturation
  limit at IC = 100 mA and IB = 10 mA. A rig input that sources more than
  2.4 mA needs its own budget, so the sink current of each rig is still to
  be measured. The 2SC1815 is rated 50 V and 150 mA, with hFE from 70 to 700
  at 2 mA and VCE(sat) at most 0.25 V (2SC1815 datasheet, Lumimax,
  2024-04-26 Rev 1.3).
- The GPIO drives the base through 4.7 kΩ (R12 / R13). The base current is
  about 0.55 mA at 3.3 V and 0.41 mA at the GPIO's 2.62 V floor, assuming
  VBE = 0.7 V. The hFE minimum of 70 is specified at VCE = 6 V and
  IC = 2 mA and does not establish saturation at the present load. A
  proposal is to reduce R12 / R13 to 1 kΩ and configure the GPIO drive
  strength to 8 mA, giving a forced gain around 5–10 for the estimated
  LED load. This still needs verification with the purchased transistor.
  With the present 4.7 kΩ resistors and pull-downs each GPIO supplies about
  2 mA. The two LED branches together draw about 27 mA from 3V3; GPIO
  currents add to this supply load.
- So that nothing transmits right after power-up or while pins are
  undefined, R8 / R9 pull each GPIO to ground through 2.2 kΩ, which holds
  the transistor and so the LED off. 2.2 kΩ is well under the 8.2 kΩ limit
  of the E9 workaround.
- KiCad's 2SC1815 symbol numbers the pins emitter, collector, base. Check
  the pin order of the purchased part against its own datasheet.

### Connector

- The unit has one 6-pin mini-DIN jack, mounted on a pitch-conversion board
  of about 27 mm by 16.5 mm with eight pins in a row about 2.54 mm from one
  long side. With that side on the left, the pins are marked G, 3, 5, 1, 2,
  6, 4, G from top to bottom (read from the board in hand). What the two G
  pins connect to has not been checked; the layout joins them to GND_RIG.
  The pinout of the jack mirrors the RTTY/DATA jack of the FTDX10, so a
  straight cable with all six conductors joins the two. Other rigs take an
  adapter cable.

| Pin | Unit | FTDX10 RTTY/DATA |
|---|---|---|
| 1 | AF output, spare in case AFSK transmit is added later | DATA IN |
| 2 | Rig ground | GND |
| 3 | PTT | DATA PTT |
| 4 | FSK | SHIFT |
| 5 | AF input | DATA OUT |
| 6 | Not connected | SQL OUT |

- The FTDX10 column is confirmed against the official FTDX10 Operation
  Manual (2307A-LS-1), p. 64; see the link under rig-side specifications.
- Pin 2 is the return for the AF and for the keying current alike.
- The shell of the jack and the shield of the cable connect to the rig's
  ground only. Joining them to the USB ground or the USB shield, directly or
  through a metal enclosure, defeats the isolation.

### Perfboard Layout

`hardware/terminal-unit.kicad_pcb` is not a board to be fabricated. It places
the parts on a universal board with a 2.54 mm grid, 37 by 28 holes, with the
Pico 2 in sockets on the same board. With 3.5 mm beyond the outer holes that
is about 98 mm by 76 mm. It is a draft, and the board to build on has not
been chosen.

- Hole (1, 1) is at (50.8 mm, 50.8 mm), so a 2.54 mm grid in the PCB editor
  falls on the holes. The `Dwgs.User` layer shows the holes and numbers
  their columns and rows.
- Tracks on `B.Cu` are bare links on the solder side and run from hole to
  adjacent hole. Tracks on `F.Cu` stand for insulated wires on the solder
  side; only their ends matter, not the path drawn.
- Columns 27 and 28 stay empty as the isolation gap, and a rule area keeps
  tracks and pads out of them. The USB side is columns 1 to 26 and the rig
  side columns 29 to 37. Only T1, U2 and U3 straddle the gap.
- GNDA returns to AGND on pin 33 and the GND of the drivers to pin 18. The
  footprint of the Pico 2 on the board lists its seven GND pads as a jumper
  pad group, which the library footprint does not, so that DRC treats them
  as joined inside the module. The LED supply is a wire of its own from the
  3V3 pin.
- The ground of the drivers goes round to pin 18 because the footprint
  shared by the Pico boards keeps copper out of the area under the antenna
  of the W variants, at the end of the board away from USB.
- The FSK / PTT drivers fill columns 21 to 26 beyond the end of the Pico 2,
  FSK in rows 9 to 15 and PTT in rows 16 to 22. All their resistors lie
  flat, one row each, and one ground rail in column 23 runs under them.
- Where pads sit in adjacent holes DRC reports overlapping courtyards. T1
  reaches to within about 1 mm of the top edge drawn for the board.
- The receive AF stage keeps at least one empty hole between neighbouring
  parts where the nets allow, and leaves rows 1 to 3 and row 19, next to
  the Pico 2, free. The 3V3, VBIAS and GNDA rails run along rows 4, 5 and 7
  above U1, the three signal nets of U1A along rows 9 to 11 to its right,
  and R5 runs straight down to GP26.
- The breakout board of J1 lies flat over columns 30 to 36 with its pins in
  column 31, rows 13 to 20, and the jack facing the edge of the board. C1
  sits above it in row 10. The rig side needs two insulated wires: FSK from
  U2 to pin 4, and a link between the GND_RIG of U2 and that of U3.
- Two footprints rest on assumptions. `ST-78_Leads_Perfboard` takes the body
  of T1 as 16.5 mm by 20 mm, read from the dimension table of the ST
  catalogue without its drawing, and puts the lead holes where the layout
  wants them; the leads have to reach up to 16 mm from the body, and their
  length has not been checked. `MiniDIN6_Breakout_SIP8` assumes that the
  breakout board extends equally beyond both end pins and that the jack
  faces away from the pin row (unverified). Its two G pads carry pad number
  2, so that they take the net of the jack's ground pin.

## 5. Working Around RP2350-E9 (Stepping A2)

- Do not rely on internal pull-downs. Inputs use pull-ups and are pulled to
  GND. Where a fixed Low is needed, use an external resistor of 8.2 kΩ or
  less.
- Disable the digital input on pins used by the ADC (required for noise as
  well).
- FSK / PTT outputs and I2S inputs are not affected.

## 6. Expected Accuracy of the Internal ADC

- The RP2350 datasheet, Table 1438, specifies 9 bits minimum and 9.5 bits
  typical ENOB against nominal 12-bit resolution. The overview in section
  12.4 instead says 9.2 ENOB; the table is used here. These are chip
  specifications, not a measurement of this assembly.
  Source: [RP2350 datasheet](https://pip-assets.raspberrypi.com/categories/1214-rp2350/documents/RP-008373-DS-2-rp2350-datasheet.pdf).
- Enough for RTTY, given the in-band dynamic range of the rig's AF,
  oversampling and the processing gain of narrow demodulation filters.
- What remains is distortion from nonlinearity, and spurs from the 1 kHz USB
  frame period, core supply switching and coupling from digital circuits.
  These may show as lines in the PC's waterfall.
- On the Pico 2 the 3V3 rail, and with it the ADC reference and the op-amp
  supply, carries the ripple of a switching regulator. Measure with GPIO 23
  low and high, and decide from the result whether to hold it high, filter
  the op-amp supply or fit an external reference.
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
- [x] 5. How `CdcAcmClass` in `embassy-usb` reports DTR changes and
  SEND_BREAK, to tell when the PC has closed the port and to offer an abort
  that does not travel through the bulk pipe.
  - DTR and RTS: `embassy-usb` 0.6.0 `src/class/cdc_acm.rs` stores both on
    SET_CONTROL_LINE_STATE in `control_out`. `split_with_control` returns a
    `ControlChanged`, whose `control_changed` waits for a change and whose
    `dtr` reads the state. SET_LINE_CODING sets the same change flag, so a
    change of DTR is found by comparing it with its previous state.
  - SEND_BREAK: not handled. `control_out` rejects requests it does not
    know, and the ACM functional descriptor gives `bmCapabilities` 0x02,
    which does not announce Send_Break. An abort through SEND_BREAK needs a
    CDC ACM class of our own.
  - DTR and RTS reach the firmware only as these flags, so opening the port
    resets nothing.
- [x] 6. The board's exposed GPIOs, and filtering of the ADC's analog supply
  and reference.
  - Pico 2: 26 GPIOs on the headers, three of them ADC inputs; ADC_AVDD is
    3V3 through 201 Ω into 2.2 µF, brought out as ADC_VREF, with AGND as a
    separate pin. See "Hardware" in section 2 for the details and source.
  - Tiny 2350, the board used before: the PIM721 schematic dated 2024-08-29
    exposes GP0–GP7 and GP26–GP29 / ADC0–ADC3 on the headers, and GP12 /
    GP13 on Qw/ST. ADC_IOVDD is connected directly to the common 3V3 rail
    with 100 nF decoupling (C12); there is no separate ADC supply filter or
    exposed reference input. The R12 / C32 filter serves VREG_AVDD, not the
    ADC.
    Source: [Pimoroni schematic](https://cdn.shopify.com/s/files/1/0174/1800/files/Pimoroni_Tiny_2350_Schematic.pdf?v=1724926880).
- [x] 7. The effective resolution of the ADC in the RP2350 datasheet.
  - Table 1438: 9 bits minimum, 9.5 bits typical. See section 6 above for
    the source and the different value in the datasheet overview.
- [ ] 8. The manuals of the FTDX10 and each Icom model: AF output level and
  impedance, FSK / PTT input specification and polarity, pinout.
  - Partially checked: official FTDX10 connector mapping and DATA OUT LEVEL
    adjustment, and IC-7300 ACC specifications, are recorded in section 4.
    FTDX10 numerical AF and keying specifications and other Icom models
    still need checking.
- [ ] 9. How `embassy-rp` writes the flash, whether XIP stops while a sector
  is erased, and what that does to the USB audio stream and the FSK output.
- [ ] 10. How the firmware reboots the RP2350 into BOOTSEL mode with
  `embassy-rp`.
- [ ] 11. Whether the host asserts DTR when it opens the control port and
  drops it when it closes the port, also when a program exits without
  closing it: Windows `usbser.sys`, Linux `cdc_acm`, Tera Term and pyserial.
  Whether ModemManager probes the port, whose interface protocol is 0x00
  (`cdc_acm.rs`, `CDC_PROTOCOL_NONE`).

## 8. Undecided

- The pins kept for an external I2S ADC. GPIO 18 to 21 are four consecutive
  free pins and are the proposal.
- Whether to hold GPIO 23 high for PWM mode, filter the op-amp supply or fit
  an external ADC reference. Decide after measuring the ADC noise.
- The load on the transformer secondary. 10 kΩ matches the transformer but
  halves the level from a 10 kΩ rig output; around 100 kΩ keeps the level but
  may peak at the high end. Measure both on the prototype.
- The tags of the lines the control port sends toward the PC
  (`control-port.md`).
- Whether `TX` while PTT is released only queues the text until `PTT ON`
  and `TXDELAY` have passed. That is the proposal, so that text can be
  queued before keying.
- What `TX` does with `BITS` other than 5. The proposal is to send ASCII
  codes as they are, without upper-casing, with 7 or 8 bits, and to answer
  `ERR` with 6.
- Whether a `PARITY` setting is needed for 7- and 8-bit transfers.
- The default of every setting, in particular `TOT`, and the scale of `GAIN`.
- Whether the unit also follows the TinyFSK protocol, so that the existing
  `tinyfsk.fsk` can drive it from MMTTY. The TinyFSK protocol has not been
  checked against its documentation, nor whether it fits the framing in
  `control-port.md`.
- What the firmware does when the PC opens and closes the control port.
  SEND_BREAK is not available with the `CdcAcmClass` of `embassy-usb`
  (check 5), so DTR is what tells. The proposals, which rest on check 11:
  - When DTR rises, discard the line being assembled, so that what a
    previous session left half typed does not join the first command. An
    empty line would not help: it runs what was left.
  - Let Ctrl-U (0x15) discard that line too, for a program to send first.
  - Discard output while DTR is low, so that opening the port does not
    deliver stale logs. A program that never asserts DTR then gets no output.
  - When DTR falls, abort the transmission and release PTT, so that a
    program that dies does not leave PTT keyed.
- The license of the hardware design files. pid.codes requires one once
  they are in the repository (see "USB VID / PID").
