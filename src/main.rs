use log::{debug, error, info, LevelFilter};
use rand::Rng;
use structopt::StructOpt;

use pretty_env_logger::env_logger::Env;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::num::ParseIntError;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;

use crate::ParsePortRangeError::{OrderError, PortError, RangeError};

/// This program waits for connections and
/// for each connection spawns a new language server and relays the messages in both directions
#[derive(StructOpt)]
struct Arguments {
    /// The Path to the java executable
    #[structopt(long = "jvm", env = "JAVA_PATH", default_value = "java")]
    java: PathBuf,

    /// The Path to the lsp jar
    #[structopt(long="jar", env = "LSP_JAR_PATH", default_value = DEFAULT_JAR_PATH)]
    lsp_jar: PathBuf,

    /// The port to listen on for incoming connections
    #[structopt(
        short = "p",
        long = "port",
        env = "LSP_LISTEN_PORT",
        default_value = "5007"
    )]
    lsp_listen_port: u16,

    /// The range of ports to use for spawning language servers
    ///
    /// The port is chosen randomly, without taking into account ports already in use!
    #[structopt(
        short = "s",
        long = "spawn",
        env = "LSP_SPAWN_PORTS",
        default_value = "5008-65535"
    )]
    lsp_spawn_ports: PortRange,
}

#[derive(Debug)]
struct PortRange {
    range: RangeInclusive<u16>,
}

#[derive(Debug)]
enum ParsePortRangeError {
    PortError(ParseIntError),
    RangeError,
    OrderError,
}

impl From<ParseIntError> for ParsePortRangeError {
    fn from(int_err: ParseIntError) -> Self {
        PortError(int_err)
    }
}

impl FromStr for PortRange {
    type Err = ParsePortRangeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (start, end) = s.split_once('-').ok_or(RangeError)?;
        let start = start.trim().parse()?;
        let end = end.trim().parse()?;

        if start > end {
            Err(OrderError)
        } else {
            Ok(PortRange { range: start..=end })
        }
    }
}

impl Display for ParsePortRangeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PortError(int_err) => write!(
                f,
                "the start and end of the port range should be integers in the range {}-{}: {}",
                u16::MIN,
                u16::MAX,
                int_err
            )?,
            RangeError => write!(
                f,
                "the start port should be separated from the end port of the port range by a '-'"
            )?,
            OrderError => write!(
                f,
                "the end of the port range should not be smaller than the start"
            )?,
        }
        Ok(())
    }
}

impl Error for ParsePortRangeError {}

fn main() -> Result<(), String> {
    let mut logger_builder = pretty_env_logger::formatted_builder();
    logger_builder
        .filter_level(LevelFilter::Trace)
        .format_timestamp_secs();

    if let Ok(value) = std::env::var("RUST_LOG") {
        logger_builder.parse_filters(&value);
    }
    logger_builder.init();

    let args = Arguments::from_args();

    if !args.lsp_jar.exists() || !args.lsp_jar.is_file() {
        return Err(format!(
            "Can't find language server jar at {}",
            args.lsp_jar.display()
        ));
    }

    let sock_ipv4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, args.lsp_listen_port));
    let sock_ipv6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, args.lsp_listen_port));

    info!(
        "Attempting to start listening on {} or {}",
        sock_ipv6, sock_ipv4
    );

    // try to bind via IPv6 and fallback to IPv4
    // some systems binding an IPv6 socket also binds a corresponding IPv4 socket
    // the connections of the latter are then received by the IPv6 socket
    // by using IPv4-Compatible (deprecated) or IPv4-Mapped IPv6 addresses
    // so preferring IPv6 may allow us to handle both with one socket
    // See [RFC 3493](https://datatracker.ietf.org/doc/html/rfc3493) Sections 3.7 and 5.3
    let socks = [sock_ipv6, sock_ipv4];

    let listener = std::net::TcpListener::bind(socks.as_slice()).unwrap();

    let address = listener
        .local_addr()
        .map_or_else(|_| String::from("unknown"), |address| address.to_string());

    let mut rng = rand::thread_rng();

    info!("Waiting for connections on {}", address);

    for connection in listener.incoming() {
        match connection {
            Err(err) => error!("{}", err),
            Ok(con) => handle_connection(
                con,
                rng.gen_range(args.lsp_spawn_ports.range.clone()),
                &args,
            ),
        }
    }

    Ok(())
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

fn lsp_command(port: u16, args: &Arguments) -> Command {
    let mut command = std::process::Command::new(&args.java);
    command
        .args(&[
            &format!("-Dport={}", port),
            "-Dfile.encoding=UTF-8",
            "-Djava.awt.headless=true",
            "-Dlog4j.configuration=file:server/log4j.properties",
            "-XX:+IgnoreUnrecognizedVMOptions",
            "-XX:+ShowCodeDetailsInExceptionMessages",
            "-jar",
        ])
        .arg(&args.lsp_jar);
    command
}

fn handle_connection(con: TcpStream, port: u16, args: &Arguments) {
    let mut lsp_cmd = lsp_command(port, args);

    std::thread::spawn(move || {
        let lsp_addrs = [
            SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        ];
        let mut lsp = format!("{} or {}", lsp_addrs[0], lsp_addrs[1]);

        let client_addr = con.peer_addr().ok();
        let client = match client_addr {
            Some(addr) => addr.to_string(),
            None => String::from("unknown"),
        };

        info!(
            "[{}] attempting to spawn LSP on port {}\n> {:?}",
            client, port, lsp_cmd
        );

        let mut lsp_proc = lsp_cmd.spawn().unwrap();
        let mut client_read = con.try_clone().unwrap();
        let mut client_write = con;

        debug!("[{}] Giving the LSP time to startup!", client);
        std::thread::sleep(Duration::from_secs(5));
        info!("[{}] Attempting to connect to LSP at {}", client, lsp);

        let server_con = loop {
            let server_con = std::net::TcpStream::connect(lsp_addrs.as_slice());
            if let Ok(con) = server_con {
                if let Ok(lsp_addr) = con.peer_addr() {
                    lsp = lsp_addr.to_string();
                }
                info!("[{}] Connected to LSP at {}", client, lsp);
                break con;
            } else if let Ok(Some(_exit)) = lsp_proc.try_wait() {
                return;
            } else {
                std::thread::sleep(Duration::from_secs(1));
                info!("[{}] Re-Attempting to connect to LSP at {}", client, lsp);
            }
        };

        let mut server_read = server_con.try_clone().unwrap();
        let mut server_write = server_con;

        let join_handle = std::thread::spawn(move || {
            let mut buf = [0; 1024];
            loop {
                match server_read.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(bytes) => {
                        let _ = client_write.write_all(&buf[..bytes]);
                    }
                }
            }
        });

        let mut buf = [0; 1024];

        loop {
            match client_read.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(bytes) => {
                    let _ = server_write.write_all(&buf[..bytes]);
                }
            }
        }

        debug!("[{}] Killing LSP at {}", client, lsp);
        let _ = lsp_proc.kill();
        let _ = lsp_proc.wait();
        let _ = join_handle.join();
        info!("[{}] Finished handling a connection and cleanup!", client)
    });
}
