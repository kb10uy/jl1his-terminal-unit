use crate::{
    keyword::{SWITCH, parse_keyword},
    settings::Setting,
};

#[derive(Clone, Copy)]
pub(super) enum Name {
    Tx,
    Send,
    Sendm,
    Ptt,
    Key,
    Abort,
    Set,
    Get,
    Status,
    Save,
    Defaults,
    Ver,
    Bootsel,
    Help,
}

pub(super) const NAMES: [(&str, Name); 14] = [
    ("TX", Name::Tx),
    ("SEND", Name::Send),
    ("SENDM", Name::Sendm),
    ("PTT", Name::Ptt),
    ("KEY", Name::Key),
    ("ABORT", Name::Abort),
    ("SET", Name::Set),
    ("GET", Name::Get),
    ("STATUS", Name::Status),
    ("SAVE", Name::Save),
    ("DEFAULTS", Name::Defaults),
    ("VER", Name::Ver),
    ("BOOTSEL", Name::Bootsel),
    ("HELP", Name::Help),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOrder {
    LsbFirst,
    MsbFirst,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command<'a> {
    Tx(&'a [u8]),
    Send(&'a [u8], BitOrder),
    Ptt(bool),
    Key,
    Abort,
    Set(Setting, &'a [u8]),
    Get(Option<Setting>),
    Status,
    Save,
    Defaults,
    Ver,
    Bootsel,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    UnknownCommand,
    BadArgument,
    UnknownSetting,
}

pub fn parse(line: &[u8]) -> Result<Command<'_>, ParseError> {
    let (name, rest) = match line.iter().position(|&b| b == b' ') {
        Some(space) => (&line[..space], &line[space + 1..]),
        None => (line, &[][..]),
    };
    let name = parse_keyword(name, &NAMES).ok_or(ParseError::UnknownCommand)?;
    let mut args = rest.split(|&b| b == b' ').filter(|arg| !arg.is_empty());
    let command = match name {
        Name::Tx => return Ok(Command::Tx(rest)),
        Name::Send => Command::Send(args.next().unwrap_or_default(), BitOrder::LsbFirst),
        Name::Sendm => Command::Send(args.next().unwrap_or_default(), BitOrder::MsbFirst),
        Name::Ptt => {
            let state = args.next().ok_or(ParseError::BadArgument)?;
            Command::Ptt(parse_keyword(state, &SWITCH).ok_or(ParseError::BadArgument)?)
        }
        Name::Key => Command::Key,
        Name::Abort => Command::Abort,
        Name::Set => {
            let setting = setting(args.next().ok_or(ParseError::BadArgument)?)?;
            Command::Set(setting, args.next().ok_or(ParseError::BadArgument)?)
        }
        Name::Get => Command::Get(args.next().map(setting).transpose()?),
        Name::Status => Command::Status,
        Name::Save => Command::Save,
        Name::Defaults => Command::Defaults,
        Name::Ver => Command::Ver,
        Name::Bootsel => Command::Bootsel,
        Name::Help => Command::Help,
    };
    match args.next() {
        Some(_) => Err(ParseError::BadArgument),
        None => Ok(command),
    }
}

fn setting(name: &[u8]) -> Result<Setting, ParseError> {
    Setting::from_name(name).ok_or(ParseError::UnknownSetting)
}

#[cfg(test)]
mod tests {
    use super::{BitOrder, Command, ParseError, parse};
    use crate::settings::Setting;
    use rstest::rstest;

    #[rstest]
    #[case("TX CQ CQ ", Command::Tx(b"CQ CQ "))]
    #[case("tx  de", Command::Tx(b" de"))]
    #[case("TX", Command::Tx(b""))]
    #[case("TX ", Command::Tx(b""))]
    #[case("SEND 1f04", Command::Send(b"1f04", BitOrder::LsbFirst))]
    #[case("SEND", Command::Send(b"", BitOrder::LsbFirst))]
    #[case("sendm 1f04", Command::Send(b"1f04", BitOrder::MsbFirst))]
    #[case("ptt on", Command::Ptt(true))]
    #[case("PTT Off", Command::Ptt(false))]
    #[case("KEY", Command::Key)]
    #[case("Abort", Command::Abort)]
    #[case("SET baud 45.45", Command::Set(Setting::Baud, b"45.45"))]
    #[case("SET  TOT   OFF ", Command::Set(Setting::Tot, b"OFF"))]
    #[case("GET", Command::Get(None))]
    #[case("GET usos", Command::Get(Some(Setting::Usos)))]
    #[case("STATUS ", Command::Status)]
    #[case("SAVE", Command::Save)]
    #[case("DEFAULTS", Command::Defaults)]
    #[case("VER", Command::Ver)]
    #[case("BOOTSEL", Command::Bootsel)]
    #[case("help", Command::Help)]
    fn parses_commands(#[case] line: &str, #[case] command: Command) {
        assert_eq!(parse(line.as_bytes()), Ok(command));
    }

    #[rstest]
    #[case("TRANSMIT", ParseError::UnknownCommand)]
    #[case("BOOTSELECT", ParseError::UnknownCommand)]
    #[case(" TX A", ParseError::UnknownCommand)]
    #[case("PTT", ParseError::BadArgument)]
    #[case("PTT 1", ParseError::BadArgument)]
    #[case("PTT ON NOW", ParseError::BadArgument)]
    #[case("SEND 1f 04", ParseError::BadArgument)]
    #[case("SENDMSB 1f", ParseError::UnknownCommand)]
    #[case("SET BAUD", ParseError::BadArgument)]
    #[case("SET", ParseError::BadArgument)]
    #[case("SET BAUD 45 45", ParseError::BadArgument)]
    #[case("SET GAIN 1", ParseError::UnknownSetting)]
    #[case("GET GAIN", ParseError::UnknownSetting)]
    #[case("GET BAUD BITS", ParseError::BadArgument)]
    #[case("VER 1", ParseError::BadArgument)]
    fn rejects_bad_lines(#[case] line: &str, #[case] error: ParseError) {
        assert_eq!(parse(line.as_bytes()), Err(error));
    }
}
