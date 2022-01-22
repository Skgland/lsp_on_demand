use crate::error::ParsePortRangeError::*;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::num::ParseIntError;

#[derive(Debug)]
pub enum ParsePortRangeError {
    ParseInt(ParseIntError),
    MissingEndSeperator,
    StartLargerThanEnd,
}

impl From<ParseIntError> for ParsePortRangeError {
    fn from(int_err: ParseIntError) -> Self {
        ParseInt(int_err)
    }
}

impl Display for ParsePortRangeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseInt(int_err) => write!(
                f,
                "the start and end of the port range should be integers in the range {}-{}: {}",
                u16::MIN,
                u16::MAX,
                int_err
            )?,
            Self::MissingEndSeperator => write!(
                f,
                "the start port should be separated from the end port of the port range by a '-'"
            )?,
            Self::StartLargerThanEnd => write!(
                f,
                "the end of the port range should not be smaller than the start"
            )?,
        }
        Ok(())
    }
}

impl Error for ParsePortRangeError {}
