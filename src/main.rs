use clap::StructOpt;
use log::{error, info, warn, LevelFilter};

use crate::error::{LspOnDemandError, ParsePortRangeError};
use arguments::Arguments;
use r2d2::Pool;
use socket2::{Domain, Protocol, Type};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::ops::DerefMut;
use std::time::Duration;

use crate::pool::LSPPoolManager;
use crate::ParsePortRangeError::{MissingEndSeperator, StartLargerThanEnd};

mod arguments;
mod error;
mod pool;

fn create_tcp_listener(candidate: SocketAddr) -> Result<TcpListener, std::io::Error> {
    let domain = Domain::for_address(candidate);
    let socket = socket2::Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;

    if domain == Domain::IPV6 {
        // force IPV6_only to false only when we are binding an IPv6 domain socket
        socket.set_only_v6(false)?;
    }
    socket.bind(&candidate.into())?;

    socket.listen(128)?;

    Ok(socket.into())
}

fn main() -> Result<(), LspOnDemandError> {
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
        return Err(LspOnDemandError::LSPNotFound(args.lsp_jar));
    }

    let lsp_pool = r2d2::Pool::builder()
        .max_lifetime(Some(Duration::from_secs(4 * 60)))
        .test_on_check_out(true)
        .min_idle(Some(2))
        .max_size(6)
        .build(pool::LSPPoolManager::new(
            args.java,
            args.lsp_jar,
            args.lsp_spawn_ports,
        ))?;

    let sock_ipv4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, args.lsp_listen_port));
    let sock_ipv6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, args.lsp_listen_port));

    // try to bind via IPv6 and fallback to IPv4
    // some systems binding an IPv6 socket also binds a corresponding IPv4 socket
    // the connections of the latter are then received by the IPv6 socket
    // by using IPv4-Compatible (deprecated) or IPv4-Mapped IPv6 addresses
    // so preferring IPv6 may allow us to handle both with one socket
    // See [RFC 3493](https://datatracker.ietf.org/doc/html/rfc3493) Sections 3.7 and 5.3
    //
    // down below we force IPv6_ONLY to false,
    // so that this works on systems which default to IPv6_ONLY set to true
    let socks = [sock_ipv6, sock_ipv4];

    info!("Attempting to start listening on one of {:?}", socks);

    let mut sockets: &[_] = &socks;

    let listener = loop {
        match sockets {
            [] => return Err(LspOnDemandError::LSPListenFailed),
            &[candidate, ref rem @ ..] => {
                sockets = rem;
                match create_tcp_listener(candidate) {
                    Ok(listener) => break listener,
                    Err(err) => warn!("Failed to create TcpListener for {}: {}", candidate, err),
                }
            }
        }
    };

    let address = listener
        .local_addr()
        .map_or_else(|_| String::from("unknown"), |address| address.to_string());

    info!("Waiting for connections on {}", address);

    for connection in listener.incoming() {
        match connection {
            Err(err) => error!("{}", err),
            Ok(con) => handle_connection(con, lsp_pool.clone()),
        }
    }

    Ok(())
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
    tx.shutdown(Shutdown::Both).unwrap();
    rx.shutdown(Shutdown::Both).unwrap();
}

fn handle_connection(client_con: TcpStream, lsp_pool: Pool<LSPPoolManager>) {
    std::thread::spawn(move || {
        let mut connection = lsp_pool.get().unwrap();

        let client_addr = client_con.peer_addr().ok();
        let client = match client_addr {
            Some(addr) => addr.to_string(),
            None => String::from("unknown"),
        };

        let server_con = connection.deref_mut().connect(&client).unwrap();

        let client_read = match client_con.try_clone() {
            Ok(x) => x,
            Err(err) => {
                error!(
                    "[Client:{}] Failed to clone client stream, for independent processing of writes and reads: {}",
                    client, err
                );
                return;
            }
        };
        let client_write = client_con;

        let server_read = match server_con.try_clone() {
            Ok(x) => x,
            Err(err) => {
                error!(
                    "[Client:{}] Failed to clone server stream, for independent processing of writes and reads: {}",
                    client, err
                );
                return;
            }
        };
        let server_write = server_con;

        let join_handle = std::thread::spawn(move || relay_connection(server_read, client_write));
        relay_connection(client_read, server_write);

        if let Err(_err) = join_handle.join() {
            warn!(
                "[Client:{}] Failed to join panicked server -> client relay thread",
                client
            );
        }
    });
}

#[test]
fn verify_app() {
    use clap::IntoApp;
    Arguments::command().debug_assert()
}
