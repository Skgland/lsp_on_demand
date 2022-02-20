use crate::error::ParsePortRangeError::*;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::num::ParseIntError;
use std::path::PathBuf;

#[derive(Debug)]
pub enum LspOnDemandError {
    LSPNotFound(PathBuf),
    LSPListenFailed,
    IOError(std::io::Error),
}

impl Display for LspOnDemandError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LspOnDemandError::LSPNotFound(path) => {
                write!(f, "LSP jar not found at {}", path.display())
            }
            LspOnDemandError::IOError(ioe) => {
                write!(f, "{}", ioe)
            }
            LspOnDemandError::LSPListenFailed => {
                write!(f, "Out of socket candidates to listen for LSP connections, can't listen for LSP connections!")
            }
        }
    }
}

impl Error for LspOnDemandError {}

impl From<std::io::Error> for LspOnDemandError {
    fn from(ioe: std::io::Error) -> Self {
        Self::IOError(ioe)
    }
}

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
