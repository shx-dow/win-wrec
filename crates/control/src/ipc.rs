use std::io;

#[cfg(unix)]
mod platform {
    use super::io;
    use crate::paths::socket_path;
    use std::os::unix::net::{UnixListener, UnixStream};

    pub type IpcListener = UnixListener;
    pub type IpcStream = UnixStream;

    pub fn endpoint_display() -> String {
        socket_path().display().to_string()
    }

    pub fn connect_stream() -> io::Result<IpcStream> {
        UnixStream::connect(socket_path())
    }

    pub fn endpoint_connectable() -> bool {
        UnixStream::connect(socket_path()).is_ok()
    }

    pub fn cleanup_stale_endpoint() -> io::Result<()> {
        let socket = socket_path();
        if socket.exists() {
            std::fs::remove_file(socket)?;
        }
        Ok(())
    }

    pub fn bind_listener() -> io::Result<IpcListener> {
        UnixListener::bind(socket_path())
    }

    pub fn remove_endpoint() -> io::Result<()> {
        let socket = socket_path();
        if socket.exists() {
            std::fs::remove_file(socket)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::io;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};

    const DEFAULT_DAEMON_PORT: u16 = 38087;

    pub type IpcListener = TcpListener;
    pub type IpcStream = TcpStream;

    pub fn endpoint_display() -> String {
        daemon_addr().to_string()
    }

    pub fn connect_stream() -> io::Result<IpcStream> {
        TcpStream::connect(daemon_addr())
    }

    pub fn endpoint_connectable() -> bool {
        TcpStream::connect(daemon_addr()).is_ok()
    }

    pub fn cleanup_stale_endpoint() -> io::Result<()> {
        Ok(())
    }

    pub fn bind_listener() -> io::Result<IpcListener> {
        TcpListener::bind(daemon_addr())
    }

    pub fn remove_endpoint() -> io::Result<()> {
        Ok(())
    }

    fn daemon_addr() -> SocketAddr {
        std::env::var("WREC_DAEMON_ADDR")
            .ok()
            .and_then(|addr| addr.parse::<SocketAddr>().ok())
            .unwrap_or_else(|| {
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, DEFAULT_DAEMON_PORT))
            })
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::io;

    pub struct IpcListener;
    pub struct IpcStream;

    pub fn endpoint_display() -> String {
        "unsupported".into()
    }

    pub fn connect_stream() -> io::Result<IpcStream> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "IPC is not supported on this platform",
        ))
    }

    pub fn endpoint_connectable() -> bool {
        false
    }

    pub fn cleanup_stale_endpoint() -> io::Result<()> {
        Ok(())
    }

    pub fn bind_listener() -> io::Result<IpcListener> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "IPC is not supported on this platform",
        ))
    }

    pub fn remove_endpoint() -> io::Result<()> {
        Ok(())
    }
}

pub use platform::*;
