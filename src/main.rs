use rand::Rng;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::process::Child;
use std::time::Duration;

fn main() {
    let sock_ipv4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 5007));
    let sock_ipv6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, 5007));

    let socket = std::net::TcpListener::bind([sock_ipv6, sock_ipv4].as_slice()).unwrap();

    let mut rng = rand::thread_rng();

    for connection in socket.incoming() {
        match connection {
            Err(err) => eprint!("{}", err),
            Ok(con) => handle_connection(con, rng.gen_range(5008..=65535)),
        }
    }
}

fn spawn_lsp(port: u16) -> Child {
    let java = std::env::var("JAVA_PATH").unwrap_or("java".into());
    let infix = if cfg!(target_os = "windows") {
        "win"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };

    std::process::Command::new(java)
        .args(&[
            &format!("-Dport={}", port),
            "-Dfile.encoding=UTF-8",
            "-Djava.awt.headless=true",
            "-XX:+ShowCodeDetailsInExceptionMessages",
            "-jar",
            &format!("./server/kieler-language-server.{infix}.jar", infix = infix),
        ])
        .spawn()
        .unwrap()
}

fn handle_connection(con: TcpStream, port: u16) {
    std::thread::spawn(move || {
        let mut proc = spawn_lsp(port);

        let mut client_read = con.try_clone().unwrap();
        let mut client_write = con;

        std::thread::sleep(Duration::from_secs(5));

        let server_con = loop {
            let server_con =
                std::net::TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
            if let Ok(con) = server_con {
                println!("Connected to Server");
                break con;
            } else if let Ok(Some(_exit)) = proc.try_wait() {
                return;
            } else {
                println!("Trying again!");
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

        let _ = proc.kill();
        let _ = proc.wait();
        let _ = join_handle.join();
        println!("Finished handling a connection and cleanup!")
    });
}
