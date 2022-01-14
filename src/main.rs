use rand::Rng;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn main() -> Result<(), String> {
    let java = std::env::var("JAVA_PATH").unwrap_or_else(|_| "java".into());
    let lsp_jar = std::env::var("LSP_JAR_PATH").unwrap_or_else(|_| jar_path());
    let lsp_path: &Path = lsp_jar.as_ref();

    if !lsp_path.exists() || !lsp_path.is_file() {
        return Err(format!(
            "Can't find language server jar at {}",
            lsp_path.display()
        ));
    }

    let sock_ipv4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 5007));
    let sock_ipv6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, 5007));

    // try to bind via IPv6 and fallback to IPv4
    // some systems binding an IPv6 socket also binds a corresponding IPv4 socket
    // the connections of the latter are then received by the IPv6 socket
    // by using IPv4-Compatible (deprecated) or IPv4-Mapped IPv6 addresses
    // so preferring IPv6 may allow us to handle both with one socket
    // See [RFC 3493](https://datatracker.ietf.org/doc/html/rfc3493) Sections 3.7 and 5.3
    let socket = std::net::TcpListener::bind([sock_ipv6, sock_ipv4].as_slice()).unwrap();

    let addr = socket
        .local_addr()
        .map_or_else(|_| String::from("unknown"), |addr| addr.to_string());

    let mut rng = rand::thread_rng();

    println!("Waiting for connections on {}", addr);

    for connection in socket.incoming() {
        match connection {
            Err(err) => eprint!("{}", err),
            Ok(con) => handle_connection(con, rng.gen_range(5008..=65535), &java, &lsp_jar),
        }
    }

    Ok(())
}

fn jar_path() -> String {
    let infix = if cfg!(target_os = "windows") {
        "win"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };

    format!("./server/kieler-language-server.{infix}.jar", infix = infix)
}

fn lsp_command(port: u16, java: &str, lsp_jar: &str) -> Command {
    let mut command = std::process::Command::new(java);
    command.args(&[
        &format!("-Dport={}", port),
        "-Dfile.encoding=UTF-8",
        "-Djava.awt.headless=true",
        "-Dlog4j.configuration=file:server/log4j.properties",
        "-XX:+IgnoreUnrecognizedVMOptions",
        "-XX:+ShowCodeDetailsInExceptionMessages",
        "-jar",
        lsp_jar,
    ]);
    command
}

fn handle_connection(con: TcpStream, port: u16, java: &str, lsp_jar: &str) {
    let mut lsp_cmd = lsp_command(port, java, lsp_jar);

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

        println!(
            "[{}] attempting to spawn LSP on port {}\n> {:?}",
            client, port, lsp_cmd
        );

        let mut lsp_proc = lsp_cmd.spawn().unwrap();
        let mut client_read = con.try_clone().unwrap();
        let mut client_write = con;

        println!("[{}] Giving the LSP time to startup!", client);
        std::thread::sleep(Duration::from_secs(5));
        println!("[{}] Attempting to connect to LSP at {}", client, lsp);

        let server_con = loop {
            let server_con = std::net::TcpStream::connect(lsp_addrs.as_slice());
            if let Ok(con) = server_con {
                if let Ok(lsp_addr) = con.peer_addr() {
                    lsp = lsp_addr.to_string();
                }
                println!("[{}] Connected to LSP at {}", client, lsp);
                break con;
            } else if let Ok(Some(_exit)) = lsp_proc.try_wait() {
                return;
            } else {
                std::thread::sleep(Duration::from_secs(1));
                println!("[{}] Re-Attempting to connect to LSP at {}", client, lsp);
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

        println!("[{}] Killing LSP at {}", client, lsp);
        let _ = lsp_proc.kill();
        let _ = lsp_proc.wait();
        let _ = join_handle.join();
        println!("[{}] Finished handling a connection and cleanup!", client)
    });
}
