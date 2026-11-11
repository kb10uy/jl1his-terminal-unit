# Circuit Review, 2026-11-11

Reviewed `hardware/terminal-unit.kicad_sch` with KiCad CLI 10.0.8, its
exported XML netlist and a rendered schematic. This is a schematic and
component-value review, without hardware measurements or a PCB layout.
Component values remain provisional; proposals below do not decide the
transformer termination or GPIO assignment.

## Findings

### P2: Optocoupler drive does not establish the quoted saturation guarantee

R6 / R7 are 470 Ω and R8 / R9 are 2.2 kΩ across the LEDs. The LED current is

`IF = (VOH - VF) / 470 - VF / 2200`.

At VOH = 3.3 V and VF = 1.15 V, this gives 4.05 mA. Substituting VF =
1.3 V gives 3.66 mA. These are estimates: the specified VF range is at
10 mA, not at this operating current. GPIO voltage drop reduces current
further. The RP2350's guaranteed VOH floor at 3.3 V is 2.62 V at the current
corresponding to the configured drive strength; the default drive setting
must not be assumed adequate for this load.

TLP785(GB)'s minimum CTR of 100% is specified at IF = 5 mA and VCE = 5 V.
It does not guarantee saturation at a collector voltage below the rig's
Low threshold. The relevant 0.4 V maximum VCE(sat) specification at
IC = 2.4 mA requires IF = 8 mA. The separate guarantee at IF = 1 mA covers
only IC = 0.2 mA. The present circuit may work with a lightly loaded input,
but the cited specifications do not establish that it will pull every
intended rig input Low.

Measure FSK and PTT sink currents and Low thresholds separately. If keeping
TLP785, a proposed improvement is a small transistor or MOSFET driver with
an external pull-down, and an LED resistor chosen to maintain at least
8 mA across supply, component and temperature tolerances for loads covered
by that saturation specification. About 180–220 Ω from 3.3 V is a starting
range for a separate driver, not a recommendation to put that load directly
on an unconfigured GPIO. Higher collector loads need their own budget.

Sources: [Toshiba TLP785 datasheet, coupled electrical characteristics](https://toshiba.semicon-storage.com/info/TLP785_datasheet_en_20190311.pdf?did=10569&prodName=TLP785),
[RP2350 datasheet, section 14.9](https://pip-assets.raspberrypi.com/categories/1214-rp2350/documents/RP-008373-DS-2-rp2350-datasheet.pdf).

### P2: The documented 100 kΩ input does not describe the schematic

R4 terminates the complete secondary with 10 kΩ. In the midband, a 1:1
ideal transformer reflects this as approximately 10 kΩ at the primary,
plus winding resistance. R1 = 4.7 kΩ leads into a high-impedance amplifier
input and does not make the termination approximately 100 kΩ.

Using the still-unverified FTDX10 model of 300 mVp-p open-circuit and
10 kΩ source resistance, plus nominal ST-78 winding resistances of 330 Ω
and 420 Ω, gives about 145 mVp-p at the secondary. A gain of 16.6 then
produces about 2.4 Vp-p before the small audio-band filter loss. At maximum
gain the same model predicts about 3.3 Vp-p, so clipping is expected at the
top of the adjustment. This is an adjustment limit, not a wiring fault.

Changing R4 to 100 kΩ would change the same estimated gain requirement to
about 8.9. Keep both termination choices open pending a frequency sweep;
do not select the gain using the 100 kΩ model while fitting 10 kΩ.
Magnetizing inductance, leakage inductance, capacitance and the actual rig
level are omitted from these estimates.

Source: [Hashimoto ST-series catalogue, ST-78 and connection diagram C](https://akizukidenshi.com/goodsaffix/st.pdf).

### Verification requirement: Analog filtering depends on the sampling plan

R5 = 1 kΩ and C3 = 15 nF give a pole at 10.61 kHz. At 24 kHz that stage
attenuates only 7.86 dB. C6 = 330 pF is across the entire feedback
resistance, so its pole moves from 241 kHz to 21.9 kHz as the AC gain moves
from 3 to 23. It cannot be treated as an independent, fixed second-order
anti-alias filter; at sufficiently high frequency the amplifier gain tends
toward unity.

The following calculations assume an ideal op-amp and omit the transformer,
ADC loading and component tolerances. Values are relative to each selected
midband gain and include R11 / C7, C6 and R5 / C3.

| Frequency | Gain 3 | Gain 23 |
|---|---:|---:|
| 2,125 Hz | −0.17 dB | −0.21 dB |
| 2,295 Hz | −0.20 dB | −0.25 dB |
| 24 kHz | −7.90 dB | −11.28 dB |
| 45,705 Hz | −13.05 dB | −20.16 dB |
| 189,705 Hz | −26.86 dB | −43.29 dB |

At 48 kS/s, 45,705 Hz aliases onto 2,295 Hz; at 192 kS/s, 189,705 Hz does.
The present parts are reasonable starting values for rig-filtered RTTY
audio at 192 kS/s, provided a digital low-pass precedes downsampling.
Neither four-sample picking nor a four-sample average establishes a strong
decimation stopband by itself. Test out-of-band injection at minimum gain,
where analog rejection is weakest, and choose a rejection target before
claiming adequacy. Direct 48 kS/s capture needs a separate assessment.

## Values and connections that look reasonable

| Parts | Result | Assessment |
|---|---|---|
| R10 = 2 kΩ, RV1 = 20 kΩ, R11 = 1 kΩ | Midband gain 3–23 | Useful adjustment range; the wiper is tied to the output end so loss of wiper contact leaves the full track in feedback. |
| R11 = 1 kΩ, C7 = 10 µF | Gain-transition pole about 15.9 Hz | Negligible loss at RTTY tones; DC gain is unity. C7's positive end is correctly toward the amplifier. |
| R2 = R3 = 10 kΩ, C2 = 10 µF | 1.65 V bias; 50 ms time constant; 3.18 Hz pole | Reasonable. Use 1% resistors; a buffer is not required by the present high-impedance bias loads. Check startup settling before accepting ADC samples. |
| C1 = 1 µF, nonpolar | Rig-side DC blocking | Reasonable for kilohertz audio; low-frequency response also depends on transformer inductance, so a simple RC estimate is insufficient. |
| R1 = 4.7 kΩ, D1 / D2 = BAT43 | Series limiting and rail clamps at U1A input | Correct clamp polarity; assess any fault specification and rail injection with USB power absent. |
| R5 = 1 kΩ, C3 = 15 nF | RC filter and capacitive-load isolation | The capacitor is beyond R5 and the feedback is taken before R5, which avoids directly loading the op-amp with 15 nF. |
| C4 = 100 nF, C5 = 10 µF | Supply decoupling | Suitable values; place C4 close to U1 pins 8 / 4. |
| R8 / R9 = 2.2 kΩ | GPIO sees approximately 2.67 kΩ to ground through R6 / R7 while the LED is off | Consistent with the A2 E9 external pull-down workaround. This protects undefined inputs, not a GPIO actively held High after a software fault. |
| U1B | VBIAS follower, unloaded | Correct treatment of the unused amplifier. |
| T1, J1, U2 / U3 | Separate rig and USB grounds | Netlist preserves galvanic isolation. Keep connector shell and enclosure wiring consistent with the schematic note. |

The ST-78 connection diagram C agrees with using primary pins 1 / 2,
secondary pins 3 / 5, and leaving centre tap 4 open. J1 agrees with the
official FTDX10 TU connection drawing on p. 64. These checks do not verify
the physical pin mapping of an eventual breakout board or footprint.

Sources: [Microchip MCP6022 datasheet, sections 4.1–4.4 and 4.7](https://ww1.microchip.com/downloads/en/DeviceDoc/20001685E.pdf),
[RP2350 datasheet, erratum E9](https://pip-assets.raspberrypi.com/categories/1214-rp2350/documents/RP-008373-DS-2-rp2350-datasheet.pdf),
[ST catalogue](https://akizukidenshi.com/goodsaffix/st.pdf),
[FTDX10 manual](https://yaesu.com/Files/4CB893D7-1018-01AF-FA97E9E9AD48B50C/FTDX10_OM_ENG_EH076H202_2307A-LS-1.pdf).

## Remaining checks before connecting a rig

- Establish actual FTDX10 AF level, source impedance, keying current and Low
  thresholds. Its official operating manual confirms the pinout, but the
  numerical assumptions in the design are still unverified.
- For Icom, qualify each model. The IC-7300 manual confirms AF impedance
  4.7 kΩ and adjustable 100–300 mV rms, FSKK below 0.6 V at less than 2 mA,
  and a separate SEND specification. It does not establish universal Icom
  compatibility. Source: [IC-7300 Basic Manual, pp. 12-1–12-2](https://www.icomeurope.com/wp-content/uploads/2019/07/IC-7300_ENG_Basic_6.pdf).
- Scope optocoupler collector voltages during cold start, BOOTSEL, reset and
  watchdog reset, then test software fault handling once outputs exist.
- Sweep AF amplitude and frequency with the actual rig and transformer;
  compare the two proposed terminations and measure clipping margin.
- Measure ADC noise with USB active and with FSK / PTT switching. The Tiny
  2350 connects ADC_IOVDD to common 3V3 with local decoupling, rather than
  through a separate ADC supply filter. Adding an op-amp supply filter
  alone cannot clean the internal ADC reference. Source:
  [Pimoroni PIM721 schematic, 2024-08-29](https://cdn.shopify.com/s/files/1/0174/1800/files/Pimoroni_Tiny_2350_Schematic.pdf?v=1724926880).

## ERC

KiCad CLI reported zero errors and two warnings, both `isolated_pin_label`
for GPIO_FSK and GPIO_PTT, expected while the MCU is absent. The project
disables some ERC categories, including `single_global_label`; ERC success
does not establish the missing MCU connections, current margins or analog
performance. No firmware was changed or flashed.

## Follow-up, 2026-11-11

After this review the LED drive was changed along the lines of the first
finding. Each LED now runs from 3V3 through 150 Ω and is switched by a
2SC1815, with 4.7 kΩ into the base and 2.2 kΩ from the GPIO to ground. The
figures and their limits are in section 4 of `design.md`. The statements
above about R6 to R9 describe the schematic as reviewed, not the present
one. With the GPIO labels now on two pins each, KiCad CLI reports no ERC
violations. The other findings stand.
