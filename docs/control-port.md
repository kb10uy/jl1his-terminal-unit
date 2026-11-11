# Control Port Protocol

The text protocol of the CDC ACM control port. The firmware does not
implement it yet. Open points are listed under "Undecided" in `design.md`.
Items marked "unverified" come from memory, as in `design.md`.

## Control Port Framing

One text protocol serves both a person typing into a serial terminal and a
program. There is no binary mode.

- Input is assembled into lines. CR, LF and CRLF each end a line: an LF
  right after a CR is dropped. Empty lines are ignored.
- BS (0x08) and DEL (0x7F) delete the last character of the line being
  assembled.
- Command names are not case-sensitive. Transmit text is converted to upper
  case, since Baudot has no lower case.
- Ctrl-C (0x03) aborts the transmission and discards the queue as soon as it
  arrives, in every mode and without waiting for the end of the line. It is
  handled byte by byte, before line assembly.
- Ctrl-S (0x13) and Ctrl-Q (0x11) are not given any meaning: a terminal with
  XON / XOFF flow control consumes them.
- The firmware does not echo its input. The terminal's local echo shows what
  is typed.
- Each command is answered with one line, `OK`, or `ERR` followed by a reason
  a person can read. Toward the PC the port also carries logs, an echo of the
  characters as they are sent and, later, decoded text. Each of those lines
  begins with a tag naming its kind. The tags are part of the command set.

## Transmit Text

- The text of a transmit command is the rest of its line. No CR or LF is
  appended, so one sentence can be sent across several commands, and a line
  break is written with escapes.
- A backslash (0x5C) starts an escape: `\r` is CR, `\n` is LF and `\a` is
  BEL. Baudot has no backslash, so reserving it takes no character away. Any
  other escape is answered with `ERR`.
- Keyboard mode, entered by a command, queues each character as it arrives,
  and the end of a line, as defined above, is sent as CR LF. Ctrl-Z (0x1A)
  leaves the mode and releases PTT once the queue has been sent in full.
  Ctrl-C aborts as in any mode.

## Control Commands

A command is its name, a space and its arguments. `TX` takes the rest of the
line after that space as it is. "While transmitting" below means while PTT
is keyed or the queue is not empty.

| Command | Action |
|---|---|
| `TX <text>` | Queue text, converted as described in "Transmit Text" |
| `SEND <hex>` | Queue code words without conversion, two hexadecimal digits per word |
| `PTT ON` | Key PTT |
| `PTT OFF` | Release PTT once the queue has been sent in full |
| `KEY` | Enter keyboard mode |
| `ABORT` | Abort as Ctrl-C does |
| `SET <name> <value>` | Change a setting at once. It is not stored until `SAVE` |
| `GET <name>` | Reply `OK <value>` |
| `GET` | Reply every setting on one line, `OK NAME=value ...` |
| `STATUS` | Reply on one line, `OK NAME=value ...`: PTT, mode, queued and free bytes (`FREE`) |
| `SAVE` | Store the settings in flash. Refused with `ERR` while transmitting |
| `DEFAULTS` | Set every setting to its default. It is not stored until `SAVE` |
| `VER` | Reply the firmware version |
| `BOOTSEL` | Reboot into BOOTSEL mode for flashing |
| `HELP` | List the commands |

- `SEND` writes data as hexadecimal rather than raw bytes, so it stays a
  line of text: Ctrl-C keeps working, no line end can be mistaken for data,
  a tty that is not in raw mode or XON / XOFF flow control cannot alter it,
  and a sender that stops halfway leaves no parser waiting. Doubling the
  length costs nothing at RTTY speeds.
- `SEND` takes no length; the number of digits gives it. An odd number of
  digits, or a word that does not fit in `BITS`, is answered with `ERR`.
  With `BITS 5` it sends raw Baudot codes, leaving the shifts to the PC,
  which suits an `.fsk` add-on; with `BITS 8` it sends 8-bit data.

## Settings

| Name | Values | Meaning |
|---|---|---|
| `BAUD` | decimal, such as `45.45` | Baud rate |
| `BITS` | `5` to `8` | Data bits |
| `STOP` | `1`, `1.5`, `2` | Stop bits |
| `INVERT` | `ON`, `OFF` | Invert the FSK polarity |
| `TXDELAY` | milliseconds | Mark sent after PTT is keyed, before the first character |
| `TXTAIL` | milliseconds | Wait after the last character before PTT is released |
| `DIDDLE` | `OFF`, `LTRS` | Hold mark, or send LTRS, while PTT is keyed and the queue is empty |
| `USOS` | `ON`, `OFF` | Unshift on space: send FIGS again after a space before figures |
| `CODE` | `ITA2`, `US` | The figures table, which differs in `$`, `'`, BEL and others |
| `TOT` | seconds, `OFF` | Abort and release PTT after transmitting this long |
| `GAIN` | to be defined | Digital gain of the UAC microphone |

- `DIDDLE`, `USOS` and `CODE` apply to Baudot, with `BITS 5`, only.
- Settings are stored in flash only by `SAVE`, so that trying values neither
  wears the flash nor keeps them. At power-up the stored settings are
  loaded. They carry a format version and a CRC; settings that fail either
  check are replaced by the defaults.
- `SAVE` is refused while transmitting because XIP stops while the flash is
  erased, which may disturb the USB audio stream and the refilling of the
  FSK output (unverified).

## Limits

- A command line holds up to 16 KB (16384 bytes), not counting its end. A
  longer line is not run: the rest of it up to its end is discarded and it is
  answered with one `ERR`. Running a truncated line could pass a different
  value, such as `SET BAUD 45.4`.
- The transmit queue holds 8 KB (8192 code words). A `TX` or `SEND` that
  does not fit is answered with `ERR` and nothing of it is queued. `SEND`
  fits a 16 KB line with room for about 8 KB of data. `TX` can need up to
  twice as many codes as characters because of the inserted shifts. At
  45.45 baud, 8192 characters take about 22 minutes.
- In keyboard mode a character that does not fit is dropped and answered
  with `ERR`.
- `STATUS` reports the free space, so that a program sending much data can
  wait for room instead of waiting for `ERR`.
