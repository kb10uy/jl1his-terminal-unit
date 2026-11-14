# Control Port Protocol

The text protocol of the CDC ACM control port, implemented in
`terminal_unit_core::control`. `SAVE` answers `ERR` until settings can be
stored in flash. Open points are listed under "Undecided" in `design.md`.
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
  handled byte by byte, before line assembly. It also discards the line being
  assembled, leaves keyboard mode and is answered with `OK`.
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
- Keyboard mode, entered by `KEY`, keys PTT and queues each character as it
  arrives, and the end of a line, as defined above, is sent as CR LF. A
  queued character is not answered. BS and DEL are ignored, since what they
  would delete may already be on the air. Ctrl-Z (0x1A) leaves the mode,
  releases PTT once the queue has been sent in full and is answered with
  `OK`. Ctrl-C aborts as in any mode.
- Text is sent as Baudot only, so `TX` and `KEY` are answered with `ERR`
  unless `BITS` is 5.
- A character that the figures table of `CODE` does not hold is answered with
  `ERR`, and nothing of the command is queued.

## Control Commands

A command is its name, a space and its arguments. `TX` takes the rest of the
line after that space as it is. The arguments of the other commands are
separated by one or more spaces, and spaces at the end of the line are
ignored. "While transmitting" below means while PTT is keyed or the queue is
not empty.

| Command | Action |
|---|---|
| `TX <text>` | Queue text, converted as described in "Transmit Text" |
| `SEND <hex>` | Queue code words without conversion, two hexadecimal digits per word |
| `SENDM <hex>` | As `SEND`, with the uppermost of the `BITS` bits of each word sent first |
| `PTT ON` | Key PTT. Text queued while PTT is released waits for it and for `TXDELAY` |
| `PTT OFF` | Release PTT once the queue has been sent in full |
| `KEY` | Key PTT and enter keyboard mode |
| `ABORT` | Abort as Ctrl-C does |
| `SET <name> <value>` | Change a setting at once. It is not stored until `SAVE` |
| `GET <name>` | Reply `OK <value>` |
| `GET` | Reply every setting on one line, `OK NAME=value ...` |
| `STATUS` | Reply on one line, such as `OK PTT=ON MODE=KEYBOARD QUEUED=12 FREE=8180 SENT=3 LAST_TX=NONE`, described under "Status" |
| `SAVE` | Store the settings in flash. Refused with `ERR` while transmitting |
| `DEFAULTS` | Set every setting to its default. It is not stored until `SAVE` |
| `VER` | Reply the firmware version |
| `BOOTSEL` | Abort, reply `OK` and reboot into BOOTSEL mode for flashing |
| `HELP` | List the command names on one line |

- `PTT ON` always keys PTT once, however soon `PTT OFF` follows it, also in
  the same USB packet: PTT is keyed for `TXDELAY`, the queue is sent and PTT
  is released after `TXTAIL`. With an empty queue that is `TXDELAY` and
  `TXTAIL` of mark. `KEY` followed at once by Ctrl-Z does the same. Only an
  abort takes a `PTT ON` back.
- `SEND` writes data as hexadecimal rather than raw bytes, so it stays a
  line of text: Ctrl-C keeps working, and neither a line end, a tty that is
  not in raw mode nor XON / XOFF flow control can alter the data.
- `SEND` takes no length; the number of digits gives it. An odd number of
  digits, or a word that does not fit in `BITS`, is answered with `ERR`.
  With `BITS 5` it sends raw Baudot codes, leaving the shifts to the PC,
  which suits an `.fsk` add-on; with `BITS 8` it sends 8-bit data.
- `SEND` sends bit 0 of a word first, the usual numbering of ITA2, in which
  E is `01`. `SENDM` is for a program that holds a word with the first bit
  on the line uppermost: it reverses the lowest `BITS` bits of each word, so
  with `BITS 5` E is `10`. A word has to fit in `BITS` for both.

## Status

`STATUS` replies with these fields, in this order. A program reads them by
name, since more may follow.

| Field | Values | Meaning |
|---|---|---|
| `PTT` | `ON`, `OFF` | Whether PTT is keyed, from the moment `PTT ON` or `KEY` is accepted until the end of `TXTAIL`, an abort or `TOT` |
| `MODE` | `COMMAND`, `KEYBOARD` | Whether keyboard mode is on |
| `QUEUED` | code words | Words waiting in the transmit queue |
| `FREE` | code words | Room left in the transmit queue |
| `SENT` | code words | Queued words whose start bit has begun in this transmission |
| `LAST_TX` | `NONE`, `DONE`, `ABORT`, `TOT` | How the latest transmission ended |

- `SENT` counts a word when its start bit begins on the FSK output, not when
  it leaves the queue. It counts every word that came through the queue,
  the shifts that `TX` inserts included, and not the LTRS of `DIDDLE`. A
  word cut off by an abort after its start bit is counted.
- `PTT ON` and `KEY` set `SENT` to 0 and `LAST_TX` to `NONE` when they
  arrive while PTT is released. Both then stay until the next such command,
  so they can be read after the transmission has ended.
- `LAST_TX` is `NONE` while PTT is keyed. It becomes `DONE` when PTT is
  released after the queue and `TXTAIL`, `ABORT` when Ctrl-C, `ABORT` or
  `BOOTSEL` releases it, and `TOT` when `TOT` does. An abort while PTT is
  released leaves it as it is.
- `PTT` is `ON` as soon as `PTT ON` or `KEY` has been answered, a moment
  before the transmit task drives the pin. An abort in that moment gives
  `PTT=OFF` and `LAST_TX=ABORT`, as one after it does.

## Settings

| Name | Values | Default | Meaning |
|---|---|---|---|
| `BAUD` | `10` to `1200`, up to three decimals, such as `45.45` | `45.45` | Baud rate |
| `BITS` | `5` to `8` | `5` | Data bits |
| `STOP` | `1`, `1.5`, `2` | `1.5` | Stop bits |
| `INVERT` | `ON`, `OFF` | `OFF` | Invert the FSK polarity. With `OFF`, mark turns the FSK output off and space turns it on |
| `TXDELAY` | milliseconds, `0` to `65535` | `100` | Mark sent after PTT is keyed, before the first character |
| `TXTAIL` | milliseconds, `0` to `65535` | `50` | Wait after the last character before PTT is released |
| `DIDDLE` | `OFF`, `LTRS` | `LTRS` | Hold mark, or send LTRS, while PTT is keyed and the queue is empty |
| `USOS` | `ON`, `OFF` | `ON` | Unshift on space: send FIGS again after a space before figures |
| `CODE` | `ITA2`, `US` | `ITA2` | The figures table, which differs in `$`, `'`, BEL and others |
| `TOT` | seconds, `1` to `65535`, `OFF` | `600` | Abort and release PTT after transmitting this long |

- Setting names and keyword values are not case-sensitive. `GET` shows
  keywords in upper case and `BAUD` without trailing zeros.
- `DIDDLE`, `USOS` and `CODE` apply to Baudot, with `BITS 5`, only.
- Settings are stored in flash only by `SAVE`, so that trying values neither
  wears the flash nor keeps them. At power-up the stored settings are
  loaded. They carry a format version and a CRC; settings that fail either
  check are replaced by the defaults.
- `SAVE` is refused while transmitting because XIP stops while the flash is
  erased, which may disturb the USB audio stream and the refilling of the
  FSK output (unverified).

## Baudot Conversion

`TX` and keyboard mode convert text to ITA2 code words and insert LTRS and
FIGS where the shift changes. Both figures tables share `-` `?` `:` `(` `)`
`.` `,` `/` and the digits. They differ in these codes (unverified; see check
12 in `design.md`):

| Letter key | `ITA2` | `US` |
|---|---|---|
| D | not sent (WRU) | `$` |
| F | not sent | `!` |
| G | not sent | `&` |
| H | not sent | `#` |
| J | BEL | `'` |
| S | `'` | BEL |
| V | `=` | `;` |
| Z | `+` | `"` |

- The first character after power-up, `SEND`, `SENDM`, `SET BITS` or
  `DEFAULTS` is preceded by its shift.
- When the receiver may have lost the shift since the last code word was
  queued, because the transmitter sent a diddle LTRS, released PTT or
  aborted, the shift is sent again before the next character that needs it.
- With `USOS ON` a space puts the converter in letters, so the next figure
  is preceded by FIGS and the next letter by nothing.

## Errors

`ERR` is followed by one of these reasons: `unknown command`,
`bad argument`, `unknown setting`, `bad value`, `line too long`,
`queue full`, `bad escape`, `character not in code`,
`odd number of digits`, `word does not fit BITS`, `TX needs BITS 5`,
`transmitting` (`SAVE`), or the reason a save failed.

## Limits

- A command line holds up to 16384 bytes, not counting its end. A longer
  line is not run: the rest of it up to its end is discarded and it is
  answered with one `ERR`. Running a truncated line could pass a different
  value, such as `SET BAUD 45.4`.
- The transmit queue holds 8192 code words. A `TX`, `SEND` or `SENDM` that
  does not fit is answered with `ERR` and nothing of it is queued. `TX` can need up
  to twice as many codes as characters because of the inserted shifts. At
  45.45 baud, 8192 characters take about 22 minutes.
- In keyboard mode a character that does not fit is dropped and answered
  with `ERR`.
- `STATUS` reports the free space, so that a program sending much data can
  wait for room instead of waiting for `ERR`.

## Opening and Closing the Port

- Opening the port resets nothing. DTR and RTS reach the firmware only as
  flags (check 5 in `design.md`), and the firmware ignores the line coding,
  so a 1200 baud open does not reset it either. Neither line keys PTT.
- Output toward the PC passes through a bounded buffer, and lines that do
  not fit are dropped. Writing to the endpoint waits until the host has
  taken the previous packet, and a host that is not reading must not stop
  command handling or transmission.
- When DTR falls, the firmware aborts as Ctrl-C does, without a reply: it
  discards the queue and the line being assembled, releases PTT and leaves
  keyboard mode. The host drops DTR when the port is closed, also when the
  program holding it is killed (check 11 in `design.md`), so a program that
  goes away does not leave the unit keyed. A bus reset clears DTR as well
  and so aborts too.
- A program therefore keeps the port open until its transmission has been
  sent. Writing commands to the port and closing it at once, as
  `echo PTT ON > COM8` does, is not supported: the transmission is aborted
  when the port closes.
- Raising DTR does nothing, and the output toward the PC is kept while DTR
  is low. A program that hangs with the port open keeps DTR raised, and
  `TOT` releases PTT that it left keyed.
