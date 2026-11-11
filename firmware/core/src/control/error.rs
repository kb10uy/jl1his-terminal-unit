use super::{QueueFull, command::ParseError};
use crate::baudot::EncodeError;
use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    UnknownCommand,
    BadArgument,
    UnknownSetting,
    BadValue,
    LineTooLong,
    QueueFull,
    BadEscape,
    Unencodable,
    OddDigits,
    WordTooWide,
    NeedsBaudot,
    Transmitting,
    Save(&'static str),
}

impl From<ParseError> for Error {
    fn from(error: ParseError) -> Self {
        match error {
            ParseError::UnknownCommand => Error::UnknownCommand,
            ParseError::BadArgument => Error::BadArgument,
            ParseError::UnknownSetting => Error::UnknownSetting,
        }
    }
}

impl From<EncodeError> for Error {
    fn from(error: EncodeError) -> Self {
        match error {
            EncodeError::Unencodable => Error::Unencodable,
            EncodeError::Full => Error::QueueFull,
        }
    }
}

impl From<QueueFull> for Error {
    fn from(_: QueueFull) -> Self {
        Error::QueueFull
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::UnknownCommand => "unknown command",
            Error::BadArgument => "bad argument",
            Error::UnknownSetting => "unknown setting",
            Error::BadValue => "bad value",
            Error::LineTooLong => "line too long",
            Error::QueueFull => "queue full",
            Error::BadEscape => "bad escape",
            Error::Unencodable => "character not in code",
            Error::OddDigits => "odd number of digits",
            Error::WordTooWide => "word does not fit BITS",
            Error::NeedsBaudot => "TX needs BITS 5",
            Error::Transmitting => "transmitting",
            Error::Save(reason) => reason,
        })
    }
}
