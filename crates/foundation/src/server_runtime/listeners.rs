use crate::server_runtime::http1::create_listener;
use std::{
    io,
    net::{SocketAddr, TcpListener},
};
use tokio::io::unix::AsyncFd;

/// Fully bound sockets; constructing this value never accepts a connection.
pub struct BoundListeners {
    pub(crate) listeners: Vec<AsyncFd<TcpListener>>,
    addresses: Vec<SocketAddr>,
}

impl BoundListeners {
    /// The first successfully bound address establishes the shared port; any
    /// later address using port zero is rebound to that same fixed or dynamic port.
    /// Failure drops every previous bind before product state can be opened.
    pub fn bind(addresses: impl IntoIterator<Item = SocketAddr>) -> io::Result<Self> {
        let mut listeners = Vec::new();
        let mut bound = Vec::new();
        let mut dynamic_port = None;
        for mut address in addresses {
            if address.port() == 0
                && let Some(port) = dynamic_port
            {
                address.set_port(port);
            }
            let listener = create_listener(address)?;
            let actual = listener.get_ref().local_addr()?;
            dynamic_port.get_or_insert(actual.port());
            bound.push(actual);
            listeners.push(listener);
        }
        if listeners.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one listener is required",
            ));
        }
        Ok(Self {
            listeners,
            addresses: bound,
        })
    }

    pub fn from_listener(listener: tokio::net::TcpListener) -> io::Result<Self> {
        let address = listener.local_addr()?;
        Ok(Self {
            listeners: vec![AsyncFd::new(listener.into_std()?)?],
            addresses: vec![address],
        })
    }

    pub fn addresses(&self) -> &[SocketAddr] {
        &self.addresses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn later_bind_failure_releases_every_earlier_socket() {
        let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
        let free = TcpListener::bind("127.0.0.1:0").unwrap();
        let first = free.local_addr().unwrap();
        drop(free);
        assert!(BoundListeners::bind([first, occupied.local_addr().unwrap()]).is_err());
        assert!(TcpListener::bind(first).is_ok());
        assert!(BoundListeners::bind([]).is_err());
    }

    #[tokio::test]
    async fn zero_ports_share_one_assigned_port() {
        let bound = BoundListeners::bind([
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.2:0".parse().unwrap(),
        ])
        .unwrap();
        assert_ne!(bound.addresses()[0].port(), 0);
        assert_eq!(bound.addresses()[0].port(), bound.addresses()[1].port());
    }

    #[tokio::test]
    async fn zero_after_a_fixed_port_uses_the_fixed_port() {
        let available = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = available.local_addr().unwrap().port();
        drop(available);
        let bound = BoundListeners::bind([
            SocketAddr::from(([127, 0, 0, 1], port)),
            SocketAddr::from(([127, 0, 0, 2], 0)),
        ])
        .unwrap();
        assert_eq!(bound.addresses()[0].port(), port);
        assert_eq!(bound.addresses()[1].port(), port);
    }
}
