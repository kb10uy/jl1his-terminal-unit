# RTTY Terminal Unit Design

Decisions from the design discussion. Items marked "unverified" come from
memory and have not been checked against source code, datasheets or manuals.

Implemented so far: the UAC microphone, which streams a test tone, the
control port, and FSK and PTT keying on the pins of the rig interface,
driven through PIO0, and the status LED. The receive AF from the ADC is
not implemented yet.

## 1. Purpose

Build an RTTY terminal unit around the RP2350. The PC sees it as a composite
USB device with:

- USB Audio input (microphone direction): the rig's receive AF. Demodulation
  runs on the PC for now (MMTTY and the like); on-board demodulation is a
  later goal.
- A serial port: text commands from the PC that key FSK, control PTT and
  change settings.

## 2. Decided

### Hardware

- Board: Raspberry Pi Pico 2, an RP2350A with 4 MB of flash. Unless another
  source is given, the board facts in this document come from the Pico 2
  datasheet (RP-008299-DS-3, release of 2026-07-03),
  `pip-assets.raspberrypi.com/categories/1005-raspberry-pi-pico-2/documents/RP-008299-DS-3-pico-2-datasheet.pdf`.
- On-board LED: GPIO 25, active high (pico-sdk
  `src/boards/include/boards/pico2.h` sets `PICO_DEFAULT_LED_PIN` to 25 and
  does not define `PICO_DEFAULT_LED_PIN_INVERTED`). The firmware does not
  use it.
- Tiny 2350: its headers do not expose GPIO 16 and 17, so the Cargo feature
  `tiny2350` moves the FSK output to the green element of that board's RGB
  LED on GPIO 19, PTT to the red element on GPIO 18 and the status LED to
  the blue element on GPIO 20. The LED is active low (Pimoroni product page, `shop.pimoroni.com/products/tiny-2350`); the
  pins are `TINY2350_LED_G_PIN`, `TINY2350_LED_R_PIN` and
  `TINY2350_LED_B_PIN` in pico-sdk
  `src/boards/include/boards/pimoroni_tiny2350.h`. `keyer::outputs` takes
  the polarity of the pins and inverts the pad outputs
  (`Pin::set_output_inversion`, `embassy-rp` 0.10.0 `src/pio/mod.rs`), so
  the state machines and the fault handlers stay the same. The status LED
  is inverted the same way with `Output::set_inversion`
  (`src/gpio.rs`).
- Chip stepping: not known until the Pico 2 is at hand. The Tiny 2350 is
  marked RP2350A0A2, stepping A2, which erratum RP2350-E9 affects. The design
  keeps the E9 workarounds either way.
- Debug: SWD is on the Pico 2's 3-pin debug header (section 5.8). Firmware
  is flashed through a debug probe with `probe-rs` and logs over defmt-rtt.
  The `BOOTSEL` command of the control port stays, so that a unit without a
  probe can still be reflashed over USB.
- Pins: the headers expose 26 GPIOs, three of them ADC inputs. GPIO 23
  drives the regulator's power-save pin, GPIO 24 senses VBUS, GPIO 25
  drives the user LED and GPIO 29 measures VSYS / 3 (section 3.1).
- Pin assignment: the receive AF goes to GPIO 26 (ADC0, pin 31), FSK comes
  from GPIO 17 (pin 22) and PTT from GPIO 16 (pin 21). GPIO 26 sits beside
  AGND, and GPIO 16 and 17 sit at the end of the board next to a GND pin,
  which keeps the wiring of the perfboard layout short. GPIO 15 (pin 20),
  across the board from GPIO 16, drives the status LED.
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

- Language and framework: Rust + embassy (`embassy-rp`, `embassy-usb`,
  `embassy-sync`).
- USB configuration: UAC (microphone, mono 48 kHz / 16 bit) + one CDC ACM
  port, the control port.
- The control port takes line-based text commands. The text to transmit and
  PTT are commands like any other, in the same stream as the commands that
  change settings and query status, so their order is defined. Later the
  port also carries decoded text.
- There is no port that takes raw Baudot at 45 baud / 5 bit with PTT on
  RTS / DTR, as MMTTY keys a UART. It depended on Windows passing that line
  coding through unchanged, which was never checked. MMTTY can still
  transmit through an `.fsk` add-on on the PC that translates to the command
  protocol, as JA7UDE's `tinyfsk.fsk` does for TinyFSK
  (https://www.qsl.net/ja7ude/tinyfsk/index.html). None is planned yet.

### License

The firmware and the rest of the software are licensed under
`MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). The license of the
hardware design files is undecided.

### USB VID / PID

- VID 0x1209 with a PID allocated by pid.codes. The PID has not been
  requested yet.
- Until it is allocated, the firmware uses the pid.codes test PIDs
  1209:0001–0010 (https://pid.codes/1209/). Move to the next one whenever
  the descriptor configuration changes, because Windows caches the old
  configuration. The firmware is at 0002.
- A request (https://pid.codes/howto/) needs a public repository with a
  recognised open source license, for the hardware as well as the software.
  It is a pull request to the pid.codes repository for an unused PID outside
  the reserved 0xxx and 1xxx ranges.

## 3. Firmware Design

### Tasks (draft)

Four tasks: the USB device, FSK transmit, control commands and audio,
connected with `embassy-sync` Channel / Signal.

### Keyer Interface

The control port and the transmitter meet at the `Keyer` trait in
`terminal_unit_core::control`. Line assembly, command parsing, settings and
the Baudot conversion live in `core` behind it and are tested on the host.

- `apply` takes the settings after every `SET` and `DEFAULTS`.
- `enqueue` takes code words, all or none: it fails with `QueueFull` when
  they do not fit. It also takes an optional `resync` word, the shift that
  the first character needs when the converter did not emit one. The keyer
  puts it in front only when the receiver may have lost the shift since the
  last word was queued: it sent a diddle LTRS, released PTT or aborted. The
  converter cannot know this, since the queue drains while it waits.
- `ptt_on` keys PTT. Words queued while PTT is released wait for it and for
  `TXDELAY`. `ptt_off` releases PTT once the queue has been sent in full and
  `TXTAIL` has passed. `abort` discards the queue and releases PTT at once.
- `status` reports whether PTT is keyed and how many words are queued and
  free.

`FskKeyer` in `firmware/app/src/keyer.rs` implements it: it holds the
8192-word queue and follows `TXDELAY`, `TXTAIL`, `DIDDLE` and `TOT`.
`FskOutput` and `PttOutput`, in `keyer/output.rs`, own the PIO state machines
that drive the FSK and PTT pins. `SAVE` goes through the
`Storage` trait, which the firmware answers with an error until check 9 is
done.

### FSK Transmit

- The state machine (PIO0, state machine 0) runs at the system clock without
  a divider and sends one character for each word of its TX FIFO, with the
  data bits of `SET BITS` (5 to 8) and the stop bits of `SET STOP` (1, 1.5
  or 2). The word holds the levels of the pin for the whole character, one
  bit for each half bit, first half bit in bit 0. The program shifts them
  out and holds each for the count in its Y register plus 4 clock cycles.
  The pull threshold of the output shift register is the number of half bits
  of a character, so the program needs no bit counter. It raises PIO
  interrupt flag 0 when it starts a character and flag 1 when it has
  finished one. With the FIFO empty it stalls and keeps the last level.
- `terminal_unit_core::fsk` turns a code word into that word: the start bit
  (space), the data bits LSB first, a 1 as mark, and the stop bits (mark),
  each bit as two half bits, with `INVERT` already applied. The half bit is
  the clock divided by twice the baud rate, rounded to the nearest cycle,
  and the stop bits are two, three or four of them, so 1.5 stop bits are
  exactly 1.5 bits. The longest character, 8 data bits and 2 stop bits, is
  22 half bits. At 150 MHz a half bit is 62500 cycles at 1200 baud and
  7.5 million at 10 baud, so the rounding stays under 10 ppm. The flags and
  the pull add 3 cycles, 20 ns, to the stop bits of each character.
- The half-bit count and the pull threshold change only when `BAUD`, `BITS`,
  `STOP` or `INVERT` has changed. The transmit task then waits until the
  character being sent has finished, stops the state machine, loads Y through
  the FIFO with executed `PULL` and `MOV` instructions and starts it again.
- The transmit task pushes one word, waits for flag 0, and then sleeps until
  the stop bits of that character begin. Only then does it decide what
  follows: the next queued word, a diddle, or holding mark. It reads the
  settings at that moment, so a change takes effect between characters, and
  it pushes the next word during the stop bits, so the next start bit follows
  without a gap. The sleep sets only the moment of the decision; the PIO
  alone times the signal. If the task is late, the stop bits stretch, which a
  receiver takes as idle mark.
- The pin is high when the optocoupler is to be on. With `INVERT OFF`, mark
  turns the optocoupler off and space turns it on, the usual keying in which
  an open input is mark. `INVERT ON` swaps them. The polarity each rig
  expects is still to be read from its manual (check 8).
- While PTT is released the pin is low whatever `INVERT` says. From the
  moment PTT is keyed until it is released the pin carries mark between
  characters: during `TXDELAY`, during `TXTAIL`, which starts when flag 1
  reports the end of the last stop bits, and with `DIDDLE OFF` while the
  queue is empty. The task sets that level with an executed `SET` while the
  state machine is stopped. `DIDDLE LTRS` sends LTRS only with `BITS 5`;
  otherwise the pin holds mark.
- PTT is GPIO 16, driven by state machine 1 of PIO0, which runs no program.
  The transmit task sets the pin with an executed `SET` after each decision:
  high from the moment PTT is keyed, before `TXDELAY`, until it is released
  after `TXTAIL` or by an abort. The PIO drives it, rather than SIO, so that
  the fault handlers release it the same way as the FSK pin.
- An abort, and `TOT` expiring, stop the state machine at once, also in the
  middle of a character: the task disables it, forces the pin low, empties
  the FIFO and restarts the program.
- The control port never stops reading because the transmit queue is full.
  A transmit command that does not fit is answered with an error, so that an
  abort never waits behind queued text.
- `firmware/app/src/fault.rs` holds the panic handler and the HardFault
  handler. Both first disable the state machines of PIO0 and force the FSK
  and PTT pins low through the registers, without the driver, with a `SET`
  written to `SM0_INSTR` and `SM1_INSTR`. The `SET` pins of state machine 1
  are the PTT pin from its configuration on. The panic handler then prints
  the message over defmt and raises a HardFault with `udf`, as `panic-probe`
  did (panic-probe 1.0.0 `src/lib.rs`), which it replaces. A reset returns
  the pins to their high-impedance default, and the external pull-downs
  hold the optocouplers off.
- Checked in `embassy-rp` 0.10.0 `src/pio/mod.rs`: `Common::make_pio_pin`
  selects the PIO function for the pin, after which SIO no longer drives it;
  `StateMachine::set_pins` and `set_pin_dirs` pause the state machine and
  execute `SET` instructions; `set_tx_threshold` rewrites the pull
  threshold alone; `Irq::wait` completes on the PIO interrupt flag and
  clears it; `set_pins` restores `PINCTRL` afterwards, so the `SET` pins
  from `set_config` stay. `PIN_16` to `PIN_19` implement `PioPin`. None of
  this has run on hardware yet.

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
  48 kHz capture.
- The UAC capture endpoint is asynchronous and sends samples on the device's
  clock.
- Samples pass through an interface boundary inside the firmware, so that an
  external I2S ADC (such as the PCM1808) can replace the internal ADC later.

### UAC Microphone

`embassy-usb` has no UAC microphone class (check 1), so
`firmware/app/src/microphone.rs` writes the UAC 1.0 descriptors itself.

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
Pico 2 is on the sheet as A1, and global labels join it to the circuits.

The USB side has two ground nets. GNDA carries the grounds of the receive AF
stage and of the op-amp supply and returns to AGND on pin 33. GND carries
the grounds of the FSK / PTT drivers and returns to the GND pins. The two
join only inside the Pico 2.

### Receive AF Input Stage

1. AC coupling on the rig side (C1)
2. Isolation transformer (10 kΩ : 10 kΩ), with one secondary end at VBIAS
3. Non-inverting amplifier with trimmer-set gain (3.3 V single supply,
   rail-to-rail, 1.65 V bias)
4. RC anti-aliasing LPF (a few kHz to 10 kHz)
5. Series resistor + small capacitor → ADC pin

- The op-amp is the MCP6022, a dual with rail-to-rail input and output that
  runs from 2.5 V to 5.5 V: gain bandwidth product 10 MHz, input noise
  density 8.7 nV/√Hz at 10 kHz, output to within 15 mV of ground and 20 mV
  of the supply into 10 kΩ (Microchip DS20001685E, front page and section
  1.0).
- R4 loads the transformer secondary with 10 kΩ for now. The value is
  undecided (section 8).
- The trimmer RV1, a Bourns 3296W in the feedback path, sets the gain
  between 3 and 23, so the level is adjusted once per rig without changing
  parts. This is the midband AC gain; R11 and C7 make the DC gain unity.
  Using the unverified FTDX10 assumptions below and R4 = 10 kΩ, a gain
  around 16–17 gives about 2.4 Vp-p at the ADC, allowing for the
  transformer's winding resistance but not its magnetizing inductance or
  leakage. Measure the actual level before choosing the gain.
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
  in the case with leads, which reach any hole of a universal board. The
  catalogue gives the frequency response as a graph from 0.2 kHz to 10 kHz
  (Hashimoto Denki, "トランジスタ用小型トランス" ST series catalogue,
  `akizukidenshi.com/goodsaffix/st.pdf`).

### Rig-Side Specifications and Assumptions

| Rig | Connector | Output impedance | Level |
|---|---|---|---|
| FTDX10 | DATA OUT on RTTY/DATA (6-pin mini-DIN) | 10 kΩ (unverified) | up to about 300 mVp-p (unverified) |
| Icom IC-7300 | AF/IF output on ACC, pin 12, selected as AF | 4.7 kΩ | 100–300 mV rms, adjustable |

- The FTDX10 Operation Manual (2307A-LS-1), p. 64, confirms the connector
  mapping. Its p. 91 documents DATA OUT LEVEL adjustment, so "fixed level"
  must not be interpreted as independent of menu settings. The numerical AF
  specifications and FSK / PTT electrical limits remain unverified.
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
  DIP4: current transfer ratio 100 % to 600 % at IF = 5 mA and VCE = 5 V,
  saturation voltage at most 0.4 V at IC = 2.4 mA and IF = 8 mA,
  collector-emitter rating 80 V, LED forward voltage 1.0 V to 1.3 V at
  10 mA, isolation 5000 Vrms. Saturated with a 1.9 kΩ load, it typically
  turns on in 1.5 µs and off in 50 µs (Toshiba TLP785 datasheet,
  2012-02-20). A coupler with a logic output would be faster but needs a
  supply on the rig side, which the rig's jack does not provide.
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
  The two LED branches together draw about 27 mA from 3V3. Each GPIO
  supplies about 2 mA to the base and to R8 / R9, and its indicator LED
  takes about 1.4 mA more (see "Status LEDs").
- So that nothing transmits right after power-up or while pins are
  undefined, R8 / R9 pull each GPIO to ground through 2.2 kΩ, which holds
  the transistor and so the LED off. 2.2 kΩ is well under the 8.2 kΩ limit
  of the E9 workaround.
- KiCad's 2SC1815 symbol numbers the pins emitter, collector, base. Check
  the pin order of the purchased part against its own datasheet.

### Status LEDs

- Three LEDs show the state of the unit: D3 (green) the status, D4 (red)
  PTT and D5 (yellow) FSK. Each runs from its GPIO through 1 kΩ (R14 to R16)
  to GND.
- D4 hangs on GPIO 16 and D5 on GPIO 17, beside the base resistors of Q2 and
  Q1, so they light exactly while the optocouplers are driven and need no
  firmware. They go dark with every path that forces the pins low, and D5
  flickers with the data.
- D3 is on GPIO 15 and only the firmware lights it, through SIO. For now it
  blinks at 2 Hz until the host has configured the device and stays lit
  once it has; what it should show is still open (section 8).
  `firmware/app/src/status.rs` registers an `embassy-usb` `Handler`, which
  reports `configured`, and takes a bus reset or the device being disabled
  as not configured, since `embassy-usb` 0.6.0 does not call
  `configured(false)` on either (`src/lib.rs`).
- The current is about 1.4 mA, assuming VF = 1.9 V, so use high-efficiency
  LEDs with VF around 2 V. An InGaN green or blue LED, with VF around 3 V,
  would barely light from 3.3 V.
- GPIO 16 and 17 then supply about 3.5 mA each. This is within the 4 mA of
  the default drive strength (unverified), and the 8 mA proposed above
  leaves more margin.

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

- The FTDX10 column is confirmed against the FTDX10 Operation Manual
  (2307A-LS-1), p. 64.
- Pin 2 is the return for the AF and for the keying current alike.
- The shell of the jack and the shield of the cable connect to the rig's
  ground only. Joining them to the USB ground or the USB shield, directly or
  through a metal enclosure, defeats the isolation.

### Perfboard Layout

`hardware/terminal-unit.kicad_pcb` is not a board to be fabricated. It places
the parts on a universal board with a 2.54 mm grid, 37 by 28 holes, about
98 mm by 76 mm, with the Pico 2 in sockets on the same board. It is a draft,
and the board to build on has not been chosen. The file holds the placement;
only its conventions are listed here.

- Hole (1, 1) is at (50.8 mm, 50.8 mm), so a 2.54 mm grid in the PCB editor
  falls on the holes. The `Dwgs.User` layer shows the holes and numbers
  their columns and rows.
- Tracks on `B.Cu` are bare links on the solder side and run from hole to
  adjacent hole. Tracks on `F.Cu` stand for insulated wires on the solder
  side; only their ends matter, not the path drawn.
- Columns 27 and 28 stay empty as the isolation gap, and a rule area keeps
  tracks and pads out of them. The USB side is columns 1 to 26 and the rig
  side columns 29 to 37. Only T1, U2 and U3 straddle the gap.
- GNDA returns to AGND on pin 33 and the GND of the drivers to pin 18,
  because the footprint shared by the Pico boards keeps copper out of the
  area under the antenna of the W variants. The footprint of the Pico 2 on
  the board lists its seven GND pads as a jumper pad group, which the
  library footprint does not, so that DRC treats them as joined inside the
  module.
- Where pads sit in adjacent holes DRC reports overlapping courtyards.
- The status LEDs stand in column 26 at the edge of the gap, at the end of
  the Pico away from the USB connector. Their cathodes join the GND of the
  drivers in column 23. Column 21 has no pads in rows 23 and 24, where the
  antenna keepout of the Pico footprint reaches beyond the module.
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
- Evaluation: once the ADC feeds USB Audio, record on the PC and look at an
  FFT. Measure the noise floor and spurs with no signal, and SINAD with a
  sine wave near full scale.

## 7. To Check Before Implementing

- [x] 1. Whether `embassy-usb` has a UAC class for the microphone direction
  (device → PC).
  - No. `embassy-usb` 0.6.0 `src/class/uac1/mod.rs` declares only
    `pub mod speaker`, and its `class_codes` and `terminal_type` modules are
    private. The descriptors are written in `firmware/app/src/microphone.rs`
    with the public `InterfaceAltBuilder::descriptor` and
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
  - Pico 2: see "Hardware" in section 2.
  - Tiny 2350, the board used before: GP0–GP7 and GP26–GP29 / ADC0–ADC3 are
    on the headers. ADC_IOVDD sits directly on the common 3V3 rail with
    100 nF (C12); there is no separate ADC supply filter or exposed
    reference input (PIM721 schematic dated 2024-08-29).
    Source: [Pimoroni schematic](https://cdn.shopify.com/s/files/1/0174/1800/files/Pimoroni_Tiny_2350_Schematic.pdf?v=1724926880).
- [x] 7. The effective resolution of the ADC in the RP2350 datasheet.
  - See section 6.
- [ ] 8. The manuals of the FTDX10 and each Icom model: AF output level and
  impedance, FSK / PTT input specification and polarity, pinout.
  - Partially checked: the FTDX10 connector mapping and DATA OUT LEVEL
    adjustment, and the IC-7300 ACC specifications, are recorded in section
    4. The FTDX10 numerical AF and keying specifications and other Icom
    models still need checking.
- [ ] 9. How `embassy-rp` writes the flash, whether XIP stops while a sector
  is erased, and what that does to the USB audio stream and the FSK output.
- [x] 10. How the firmware reboots the RP2350 into BOOTSEL mode with
  `embassy-rp`.
  - `embassy-rp` 0.10.0 `src/rom_data/rp235x.rs` `reset_to_usb_boot` calls
    the bootrom `reboot` with `REBOOT2_FLAG_REBOOT_TYPE_BOOTSEL` and
    `REBOOT2_FLAG_NO_RETURN_ON_SUCCESS`, translated from pico-sdk
    `bootrom.c`. The `BOOTSEL` command calls it with both masks 0, after up
    to 500 ms for the reply to drain. Not yet tried on hardware.
- [ ] 11. Whether the host asserts DTR when it opens the control port and
  drops it when it closes the port, also when a program exits without
  closing it: Windows `usbser.sys`, Linux `cdc_acm`, Tera Term and pyserial.
  Whether ModemManager probes the port, whose interface protocol is 0x00
  (`cdc_acm.rs`, `CDC_PROTOCOL_NONE`).
- [ ] 12. The ITA2 and US figures tables in `terminal_unit_core::baudot` and
  in "Baudot Conversion" of `control-port.md`, against ITU-T
  Recommendation S.1 and a source for the US teleprinter code.

## 8. Undecided

- What the status LED (D3) shows. The proposal is to blink it while the host
  has not configured the device and to keep it lit once it has, which the
  firmware does for now.
- The pins kept for an external I2S ADC. GPIO 18 to 21 are four consecutive
  free pins and are the proposal.
- Whether to hold GPIO 23 high for PWM mode, filter the op-amp supply or fit
  an external ADC reference. Decide after measuring the ADC noise.
- The load on the transformer secondary. 10 kΩ matches the transformer but
  halves the level from a 10 kΩ rig output; around 100 kΩ keeps the level but
  may peak at the high end. Measure both on the prototype.
- The tags of the lines the control port sends toward the PC
  (`control-port.md`).
- What `TX` does with `BITS` other than 5. For now it is answered with
  `ERR`. The proposal is to send ASCII codes as they are, without
  upper-casing, with 7 or 8 bits, and to answer `ERR` with 6.
- Whether a `PARITY` setting is needed for 7- and 8-bit transfers.
- The scale and default of `GAIN`.
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
  they are in the repository.
