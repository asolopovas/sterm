use anyhow::{Context, Result};
use std::net::{SocketAddr, ToSocketAddrs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub addr: SocketAddr,
    pub server_name: String,
}

pub fn resolve_target(host: &str) -> Result<Target> {
    let server_name = host
        .rsplit_once(':')
        .map(|(name, _)| name)
        .unwrap_or(host)
        .trim_matches(['[', ']'])
        .to_owned();
    let addr = host
        .to_socket_addrs()?
        .next()
        .with_context(|| format!("could not resolve {host}"))?;
    Ok(Target { addr, server_name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_resolution_extracts_ipv4_server_name() {
        let target = resolve_target("127.0.0.1:4433").unwrap();

        assert_eq!(target.addr.port(), 4433);
        assert_eq!(target.server_name, "127.0.0.1");
    }

    #[test]
    fn target_resolution_extracts_dns_server_name() {
        let target = resolve_target("localhost:4433").unwrap();

        assert_eq!(target.addr.port(), 4433);
        assert_eq!(target.server_name, "localhost");
    }

    #[test]
    fn target_resolution_extracts_bracketed_ipv6_server_name() {
        let target = resolve_target("[::1]:4433").unwrap();

        assert_eq!(target.addr.port(), 4433);
        assert_eq!(target.server_name, "::1");
    }

    #[test]
    fn target_resolution_rejects_missing_port() {
        assert!(resolve_target("localhost").is_err());
    }
}
