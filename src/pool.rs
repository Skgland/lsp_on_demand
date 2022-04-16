use crate::arguments::PortRange;
use log::{debug, error, info, warn};
use rand::{Rng, SeedableRng};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

pub(crate) struct LSPPoolManager {
    java_command: PathBuf,
    jar_path: PathBuf,
    rng: Mutex<rand::rngs::StdRng>,
    ports: PortRange,
}

impl LSPPoolManager {
    pub(crate) fn new(java: PathBuf, lsp_jar: PathBuf, ports: crate::arguments::PortRange) -> Self {
        Self {
            java_command: java,
            jar_path: lsp_jar,
            rng: Mutex::new(rand::rngs::StdRng::from_entropy()),
            ports,
        }
    }
}

impl r2d2::ManageConnection for LSPPoolManager {
    type Connection = LSPConnection;
    type Error = std::io::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let mut lsp_cmd = std::process::Command::new(&self.java_command);
        let port: u16 = self.rng.lock().unwrap().gen_range(self.ports.range.clone());
        lsp_cmd
            .args(&[
                &format!("-Dport={}", port),
                "-Dfile.encoding=UTF-8",
                "-Djava.awt.headless=true",
                "-Dlog4j.configuration=file:server/log4j.properties",
                "-XX:+IgnoreUnrecognizedVMOptions",
                "-XX:+ShowCodeDetailsInExceptionMessages",
                "-jar",
            ])
            .arg(&self.jar_path);

        info!("Attempting to spawn LSP on port {}\n> {:?}", port, lsp_cmd);

        let lsp_proc = match lsp_cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                error!("[LSP:{}] Failed to spawn child lsp process: {}", port, err);
                return Err(err);
            }
        };

        debug!("[LSP:{}] Giving the LSP time to startup!", port);
        std::thread::sleep(Duration::from_secs(3));

        Ok(LSPConnection {
            process: lsp_proc,
            port,
        })
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        match conn.process.try_wait() {
            Ok(None) => Ok(()),
            Err(err) => Err(err),
            Ok(Some(_exit_status)) => {
                info!("[LSP:{}] has been invalidated!", conn.port);
                Err(std::io::Error::from(std::io::ErrorKind::Other))
            }
        }
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        self.is_valid(conn).is_ok()
    }
}

pub(crate) struct LSPConnection {
    process: std::process::Child,
    port: u16,
}

impl LSPConnection {
    pub fn connect(&mut self, client: &str) -> Result<TcpStream, ()> {
        let lsp_addrs = [
            SocketAddr::from((Ipv6Addr::LOCALHOST, self.port)),
            SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)),
        ];

        let mut lsp = format!("{} or {}", lsp_addrs[0], lsp_addrs[1]);

        info!("[{}] Attempting to connect to LSP at {}", client, lsp);

        let server_con = loop {
            let server_con = std::net::TcpStream::connect(lsp_addrs.as_slice());
            if let Ok(con) = server_con {
                if let Ok(lsp_addr) = con.peer_addr() {
                    lsp = lsp_addr.to_string();
                }
                info!("[{}] Connected to LSP at {}", client, lsp);
                break con;
            } else if let Ok(Some(_exit)) = self.process.try_wait() {
                return Err(());
            } else {
                std::thread::sleep(Duration::from_secs(1));
                info!("[{}] Re-Attempting to connect to LSP at {}", client, lsp);
            }
        };

        Ok(server_con)
    }
}

impl Drop for LSPConnection {
    fn drop(&mut self) {
        debug!("[LSP:{}] Killing LSP", self.port);
        if let Err(err) = self.process.kill() {
            warn!(
                "[LSP:{}] Failed to kill lsp child process: {}",
                self.port, err
            );
            info!("[LSP:{}] Will not wait for lsp child process", self.port)
        } else {
            // only wait on lsp process if it was killed successfully
            if let Err(err) = self.process.wait() {
                warn!(
                    "[LSP:{}] Failed to wait for lsp child process: {}",
                    self.port, err
                )
            }
        }
        info!(
            "[LSP:{}] Finished handling a connection and cleanup!",
            self.port
        )
    }
}
