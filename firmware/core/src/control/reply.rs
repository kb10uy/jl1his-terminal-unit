use super::{Mode, TxStatus, command};
use crate::settings::{Setting, Settings, Value};
use core::fmt;

pub(super) enum Reply<'a> {
    Ok,
    Bootsel,
    Value(Value<'a>),
    Settings(&'a Settings),
    Status(TxStatus, Mode),
    Version(&'static str),
    Help,
}

impl fmt::Display for Reply<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OK")?;
        match self {
            Reply::Ok | Reply::Bootsel => Ok(()),
            Reply::Value(value) => write!(f, " {value}"),
            Reply::Settings(settings) => Setting::ALL.into_iter().try_for_each(|setting| {
                write!(f, " {}={}", setting.name(), settings.value(setting))
            }),
            Reply::Status(status, mode) => {
                let ptt = if status.ptt { "ON" } else { "OFF" };
                let mode = match mode {
                    Mode::Command => "COMMAND",
                    Mode::Keyboard => "KEYBOARD",
                };
                write!(
                    f,
                    " PTT={ptt} MODE={mode} QUEUED={} FREE={}",
                    status.queued, status.free
                )
            }
            Reply::Version(version) => write!(f, " {version}"),
            Reply::Help => command::NAMES
                .into_iter()
                .try_for_each(|(name, _)| write!(f, " {name}")),
        }
    }
}
