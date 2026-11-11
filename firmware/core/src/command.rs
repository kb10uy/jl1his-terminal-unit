use crate::settings::Setting;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command<'a> {
    Tx(&'a [u8]),
    Send(&'a [u8]),
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

pub const NAMES: [&str; 13] = [
    "TX", "SEND", "PTT", "KEY", "ABORT", "SET", "GET", "STATUS", "SAVE", "DEFAULTS", "VER",
    "BOOTSEL", "HELP",
];

const MAX_NAME_LEN: usize = 8;

pub fn parse(line: &[u8]) -> Result<Command<'_>, ParseError> {
    let (name, rest) = match line.iter().position(|&b| b == b' ') {
        Some(space) => (&line[..space], &line[space + 1..]),
        None => (line, &[][..]),
    };
    let mut upper = [0; MAX_NAME_LEN];
    let upper = upper
        .get_mut(..name.len())
        .ok_or(ParseError::UnknownCommand)?;
    upper.copy_from_slice(name);
    upper.make_ascii_uppercase();

    if *upper == *b"TX" {
        return Ok(Command::Tx(rest));
    }

    let mut args = rest.split(|&b| b == b' ').filter(|arg| !arg.is_empty());
    let command = match &*upper {
        b"SEND" => Command::Send(args.next().unwrap_or_default()),
        b"PTT" => {
            let state = args.next().ok_or(ParseError::BadArgument)?;
            if state.eq_ignore_ascii_case(b"ON") {
                Command::Ptt(true)
            } else if state.eq_ignore_ascii_case(b"OFF") {
                Command::Ptt(false)
            } else {
                return Err(ParseError::BadArgument);
            }
        }
        b"KEY" => Command::Key,
        b"ABORT" => Command::Abort,
        b"SET" => {
            let setting = setting(args.next().ok_or(ParseError::BadArgument)?)?;
            Command::Set(setting, args.next().ok_or(ParseError::BadArgument)?)
        }
        b"GET" => Command::Get(args.next().map(setting).transpose()?),
        b"STATUS" => Command::Status,
        b"SAVE" => Command::Save,
        b"DEFAULTS" => Command::Defaults,
        b"VER" => Command::Ver,
        b"BOOTSEL" => Command::Bootsel,
        b"HELP" => Command::Help,
        _ => return Err(ParseError::UnknownCommand),
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
    use super::{Command, ParseError, parse};
    use crate::settings::Setting;
    use rstest::rstest;

    #[rstest]
    #[case("TX CQ CQ ", Command::Tx(b"CQ CQ "))]
    #[case("tx  de", Command::Tx(b" de"))]
    #[case("TX", Command::Tx(b""))]
    #[case("TX ", Command::Tx(b""))]
    #[case("SEND 1f04", Command::Send(b"1f04"))]
    #[case("SEND", Command::Send(b""))]
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
