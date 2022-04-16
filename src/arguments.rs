use crate::{MissingEndSeperator, ParsePortRangeError, StartLargerThanEnd};
use clap::StructOpt;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str::FromStr;

/// This program waits for connections and
/// for each connection spawns a new language server and relays the messages in both directions
#[derive(StructOpt)]
#[structopt(version)]
pub struct Arguments {
    /// The Path to the java executable
    #[structopt(long = "jvm", env = "JAVA_PATH", default_value = "java")]
    pub(crate) java: PathBuf,

    /// The Path to the lsp jar
    #[structopt(long="jar", env = "LSP_JAR_PATH", default_value = DEFAULT_JAR_PATH)]
    pub(crate) lsp_jar: PathBuf,

    /// The port to listen on for incoming connections
    #[structopt(
        short = 'p',
        long = "port",
        env = "LSP_LISTEN_PORT",
        default_value = "5007"
    )]
    pub(crate) lsp_listen_port: u16,

    /// The range of ports to use for spawning language servers
    ///
    /// The port is chosen randomly, without taking into account ports already in use!
    #[structopt(
        short = 's',
        long = "spawn",
        env = "LSP_SPAWN_PORTS",
        default_value = "5008-65535"
    )]
    pub(crate) lsp_spawn_ports: PortRange,
}

const DEFAULT_JAR_PATH: &str = {
    if cfg!(target_os = "windows") {
        "./server/kieler-language-server.win.jar"
    } else if cfg!(target_os = "macos") {
        "./server/kieler-language-server.osx.jar"
    } else if cfg!(target_os = "linux") {
        "./server/kieler-language-server.linux.jar"
    } else {
        "./server/kieler-language-server.unknown.jar"
    }
};

#[derive(Debug)]
pub(crate) struct PortRange {
    pub(crate) range: RangeInclusive<u16>,
}

impl FromStr for PortRange {
    type Err = ParsePortRangeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (start, end) = s.split_once('-').ok_or(MissingEndSeperator)?;
        let start = start.trim().parse()?;
        let end = end.trim().parse()?;

        if start > end {
            Err(StartLargerThanEnd)
        } else {
            Ok(PortRange { range: start..=end })
        }
    }
}
