use std::time::Duration;

pub const APP_NAME: &str = "rterm";
pub const TOKEN_ENV: &str = "RTERM_TOKEN";
pub const TOKEN_FILE_ENV: &str = "RTERM_TOKEN_FILE";
pub const MIN_TOKEN_LEN: usize = 32;
pub const SHA256_LEN: usize = 32;
pub const SHA256_HEX_LEN: usize = SHA256_LEN * 2;
pub const TOKEN_FILE_UNIX_PRIVATE_MODE_MASK: u32 = 0o077;

pub const DEFAULT_DIRECT_SERVER_NAME: &str = "localhost";
pub const DIRECT_CERT_PIN_LABEL: &str = "direct mode certificate SHA-256 fingerprint";
pub const RELAY_CERT_PIN_LABEL: &str = "relay certificate SHA-256 fingerprint";
pub const ALPN: &[u8] = b"rterm-poc/1";

pub const NONCE_LEN: usize = 32;
pub const AUTH_CONTEXT: &[u8] = b"rterm-poc-auth-v1";
pub const CHALLENGE_PREFIX: &[u8] = b"CHAL1\0";
pub const AUTH_PREFIX: &[u8] = b"AUTH1\0";
pub const WIRE_SEPARATOR: &[u8] = b"\0";

pub const ROLE_HOST: &[u8] = b"HOST";
pub const ROLE_CLIENT: &[u8] = b"CLIENT";
pub const HELLO_HOST: &[u8] = b"HELLO\0HOST";
pub const HELLO_CLIENT: &[u8] = b"HELLO\0CLIENT";
pub const RESPONSE_OK: &[u8] = b"OK\n";
pub const RESPONSE_WAIT: &[u8] = b"WAIT\n";
pub const RESPONSE_ERR_BAD_HELLO: &[u8] = b"ERR bad hello\n";
pub const RESPONSE_ERR_AUTH_FAILED: &[u8] = b"ERR auth failed\n";
pub const RESPONSE_ERR_NO_HOST_WAITING: &[u8] = b"ERR no host waiting\n";
pub const RESPONSE_ERR_REPLACED_HOST: &[u8] = b"ERR replaced by newer host\n";
pub const RESPONSE_ERR_ROLE_MISMATCH: &[u8] = b"ERR role mismatch\n";

pub const CLOSE_NORMAL: u32 = 0;
pub const CLOSE_AUTH_FAILED: u32 = 1;
pub const CLOSE_REASON_DONE: &[u8] = b"done";
pub const CLOSE_REASON_AUTH_FAILED: &[u8] = b"auth failed";

pub const MAX_FRAME: usize = 1024 * 1024;
pub const CONTROL_FRAME_LIMIT: usize = 64;
pub const AUTH_FRAME_LIMIT: usize = 128;
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
pub const ERROR_RESPONSE_GRACE: Duration = Duration::from_millis(250);
pub const RELAY_RECONNECT_DELAY: Duration = Duration::from_secs(2);
pub const TRACKER_RETRY_DELAY: Duration = Duration::from_secs(10);
pub const TRACKER_CLIENT_DISCOVERY_ATTEMPTS: usize = 6;
pub const TLS_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);
pub const MAX_CONCURRENT_BIDI_STREAMS: u32 = 32;
pub const MAX_CONCURRENT_UNI_STREAMS: u32 = 8;

pub const RELAY_CONNECTION_LIMIT: usize = 256;
pub const DIRECT_HOST_CONNECTION_LIMIT: usize = 64;
pub const STDIN_CHANNEL_CAPACITY: usize = 64;
pub const PTY_CHANNEL_CAPACITY: usize = 64;
pub const IO_BUFFER_SIZE: usize = 8192;
pub const DEFAULT_PTY_ROWS: u16 = 24;
pub const DEFAULT_PTY_COLS: u16 = 80;
pub const DEFAULT_UPNP_LEASE_SECONDS: u32 = 3600;
pub const UPNP_DESCRIPTION: &str = "rterm-poc quic";
pub const UPNP_PROBE_IPV4: [u8; 4] = [8, 8, 8, 8];
pub const UPNP_PROBE_PORT: u16 = 80;

pub const DEFAULT_WINDOWS_SHELL_ENV: &str = "COMSPEC";
pub const DEFAULT_WINDOWS_SHELL: &str = "cmd.exe";
pub const DEFAULT_UNIX_SHELL_ENV: &str = "SHELL";
pub const DEFAULT_UNIX_SHELL: &str = "/bin/sh";
