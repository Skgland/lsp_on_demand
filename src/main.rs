use clap::StructOpt;
use log::{debug, error, info, warn, LevelFilter};
use rand::Rng;

use crate::error::ParsePortRangeError;
use std::fmt::Debug;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;

use crate::ParsePortRangeError::{MissingEndSeperator, StartLargerThanEnd};

mod error;

/// This program waits for connections and
/// for each connection spawns a new language server and relays the messages in both directions
#[derive(StructOpt)]
#[structopt(version)]
struct Arguments {
    /// The Path to the java executable
    #[structopt(long = "jvm", env = "JAVA_PATH", default_value = "java")]
    java: PathBuf,

    /// The Path to the lsp jar
    #[structopt(long="jar", env = "LSP_JAR_PATH", default_value = DEFAULT_JAR_PATH)]
    lsp_jar: PathBuf,

    /// The port to listen on for incoming connections
    #[structopt(
        short = 'p',
        long = "port",
        env = "LSP_LISTEN_PORT",
        default_value = "5007"
    )]
    lsp_listen_port: u16,

    /// The range of ports to use for spawning language servers
    ///
    /// The port is chosen randomly, without taking into account ports already in use!
    #[structopt(
        short = 's',
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

fn main() -> Result<(), String> {
    let mut logger_builder = pretty_env_logger::formatted_builder();
    logger_builder
        .filter_level(LevelFilter::Trace)
        .format_timestamp_secs();

    if let Ok(value) = std::env::var("RUST_LOG") {
        logger_builder.parse_filters(&value);
    }
    logger_builder.init();

    let args = Arguments::parse();

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

fn relay_connection(mut rx: TcpStream, mut tx: TcpStream) {
    let mut buf = [0; 1024];
    loop {
        match rx.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(bytes) => {
                let _ = tx.write_all(&buf[..bytes]);
            }
        }
    }
}

fn handle_connection(client_con: TcpStream, port: u16, args: &Arguments) {
    let mut lsp_cmd = lsp_command(port, args);

    std::thread::spawn(move || {
        let lsp_addrs = [
            SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        ];
        let mut lsp = format!("{} or {}", lsp_addrs[0], lsp_addrs[1]);

        let client_addr = client_con.peer_addr().ok();
        let client = match client_addr {
            Some(addr) => addr.to_string(),
            None => String::from("unknown"),
        };

        info!(
            "[{}] attempting to spawn LSP on port {}\n> {:?}",
            client, port, lsp_cmd
        );

        let mut lsp_proc = match lsp_cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                error!("[{}] Failed to spawn child lsp process: {}", client, err);
                return;
            }
        };

        let client_read = match client_con.try_clone() {
            Ok(x) => x,
            Err(err) => {
                error!(
                    "[{}] Failed to clone client stream, for independent processing of writes and reads: {}",
                    client, err
                );
                return;
            }
        };
        let client_write = client_con;

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

        let server_read = match server_con.try_clone() {
            Ok(x) => x,
            Err(err) => {
                error!(
                    "[{}] Failed to clone server stream, for independent processing of writes and reads: {}",
                    client, err
                );
                return;
            }
        };
        let server_write = server_con;

        let join_handle = std::thread::spawn(move || relay_connection(server_read, client_write));

        relay_connection(client_read, server_write);

        debug!("[{}] Killing LSP at {}", client, lsp);
        if let Err(err) = lsp_proc.kill() {
            warn!("[{}] Failed to kill lsp child process: {}", client, err);
            info!("[{}] Will not wait for lsp child process", client)
        } else {
            // only wait on lsp process if it was killed successfully
            if let Err(err) = lsp_proc.wait() {
                warn!("[{}] Failed to wait for lsp child process: {}", client, err)
            }
        }
        if let Err(_err) = join_handle.join() {
            warn!(
                "[{}] Failed to join panicked server -> client relay thread",
                client
            );
        }
        info!("[{}] Finished handling a connection and cleanup!", client)
    });
}

#[test]
fn verify_app() {
    use clap::IntoApp;
    Arguments::command().debug_assert()
}
