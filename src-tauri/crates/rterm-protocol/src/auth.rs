use anyhow::{anyhow, bail, Context, Result};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::config;

type HmacSha256 = Hmac<Sha256>;

pub fn new_auth_challenge() -> Result<[u8; config::NONCE_LEN]> {
    let mut nonce = <[u8; config::NONCE_LEN]>::default();
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut nonce)
        .map_err(|_| anyhow!("secure randomness unavailable"))?;
    Ok(nonce)
}

pub fn challenge_frame(nonce: &[u8; config::NONCE_LEN]) -> Vec<u8> {
    [config::CHALLENGE_PREFIX, nonce].concat()
}

pub fn parse_challenge_frame(frame: &[u8]) -> Result<[u8; config::NONCE_LEN]> {
    if !frame.starts_with(config::CHALLENGE_PREFIX)
        || frame.len() != config::CHALLENGE_PREFIX.len() + config::NONCE_LEN
    {
        bail!("invalid auth challenge");
    }
    frame[config::CHALLENGE_PREFIX.len()..]
        .try_into()
        .context("copy auth challenge nonce")
}

pub fn auth_response_frame(
    role: &[u8],
    token: &[u8],
    nonce: &[u8; config::NONCE_LEN],
) -> Result<Vec<u8>> {
    if role.is_empty() || role.contains(&0) {
        bail!("invalid auth role");
    }
    let mac = auth_mac(role, token, nonce)?;
    Ok([
        config::AUTH_PREFIX,
        role,
        config::WIRE_SEPARATOR,
        mac.as_slice(),
    ]
    .concat())
}

pub fn verify_auth_response(
    frame: &[u8],
    token: &[u8],
    nonce: &[u8; config::NONCE_LEN],
) -> Result<Vec<u8>> {
    if !frame.starts_with(config::AUTH_PREFIX) {
        bail!("invalid auth response");
    }
    let rest = &frame[config::AUTH_PREFIX.len()..];
    let pos = rest
        .iter()
        .position(|b| *b == 0)
        .context("missing auth role separator")?;
    let role = &rest[..pos];
    if role.is_empty() {
        bail!("invalid auth role");
    }
    let got_mac = &rest[pos + 1..];
    let mac = auth_mac_inner(role, token, nonce)?;
    mac.verify_slice(got_mac)
        .map_err(|_| anyhow!("authentication failed"))?;
    Ok(role.to_vec())
}

fn auth_mac(role: &[u8], token: &[u8], nonce: &[u8; config::NONCE_LEN]) -> Result<Vec<u8>> {
    Ok(auth_mac_inner(role, token, nonce)?
        .finalize()
        .into_bytes()
        .to_vec())
}

fn auth_mac_inner(
    role: &[u8],
    token: &[u8],
    nonce: &[u8; config::NONCE_LEN],
) -> Result<HmacSha256> {
    let mut mac = HmacSha256::new_from_slice(token).map_err(|_| anyhow!("invalid token"))?;
    mac.update(config::AUTH_CONTEXT);
    mac.update(config::WIRE_SEPARATOR);
    mac.update(role);
    mac.update(config::WIRE_SEPARATOR);
    mac.update(nonce);
    Ok(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_round_trips() {
        let nonce = test_nonce(7);
        let frame = auth_response_frame(config::ROLE_CLIENT, b"correct-token", &nonce).unwrap();

        assert_eq!(
            verify_auth_response(&frame, b"correct-token", &nonce).unwrap(),
            config::ROLE_CLIENT
        );
    }

    #[test]
    fn auth_rejects_bad_token() {
        let nonce = test_nonce(7);
        let frame = auth_response_frame(config::ROLE_CLIENT, b"correct-token", &nonce).unwrap();

        assert!(verify_auth_response(&frame, b"wrong-token", &nonce).is_err());
    }

    #[test]
    fn auth_response_rejects_empty_role() {
        let nonce = test_nonce(0);

        assert!(auth_response_frame(b"", b"token", &nonce).is_err());
    }

    #[test]
    fn auth_response_rejects_role_with_separator() {
        let nonce = test_nonce(0);

        assert!(auth_response_frame(b"BAD\0ROLE", b"token", &nonce).is_err());
    }

    #[test]
    fn verify_auth_response_rejects_bad_prefix() {
        let nonce = test_nonce(1);

        assert!(verify_auth_response(b"wrong", b"token", &nonce).is_err());
    }

    #[test]
    fn verify_auth_response_rejects_missing_role_separator() {
        let nonce = test_nonce(1);
        let frame = [config::AUTH_PREFIX, b"CLIENT"].concat();

        assert!(verify_auth_response(&frame, b"token", &nonce).is_err());
    }

    #[test]
    fn verify_auth_response_rejects_empty_role() {
        let nonce = test_nonce(1);
        let frame = [config::AUTH_PREFIX, config::WIRE_SEPARATOR, b"mac"].concat();

        assert!(verify_auth_response(&frame, b"token", &nonce).is_err());
    }

    #[test]
    fn verify_auth_response_rejects_truncated_mac() {
        let nonce = test_nonce(1);
        let mut frame = auth_response_frame(config::ROLE_CLIENT, b"token", &nonce).unwrap();
        frame.truncate(frame.len() - 1);

        assert!(verify_auth_response(&frame, b"token", &nonce).is_err());
    }

    #[test]
    fn challenge_round_trips() {
        let nonce = test_nonce(3);

        assert_eq!(
            parse_challenge_frame(&challenge_frame(&nonce)).unwrap(),
            nonce
        );
    }

    #[test]
    fn challenge_rejects_bad_prefix() {
        assert!(parse_challenge_frame(b"wrong").is_err());
    }

    #[test]
    fn challenge_rejects_bad_length() {
        let nonce = test_nonce(3);
        let mut frame = challenge_frame(&nonce);
        frame.push(0);

        assert!(parse_challenge_frame(&frame).is_err());
    }

    #[test]
    fn auth_challenge_uses_secure_randomness() {
        let nonce = new_auth_challenge().unwrap();

        assert_eq!(nonce.len(), config::NONCE_LEN);
    }

    fn test_nonce(seed: u8) -> [u8; config::NONCE_LEN] {
        std::array::from_fn(|idx| seed.wrapping_add(idx as u8))
    }
}
