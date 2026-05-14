use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::config;

pub fn load_token(token: Option<String>, token_file: Option<PathBuf>) -> Result<String> {
    let raw = if let Some(token) = token {
        token
    } else if let Some(path) = token_file {
        read_token_file(&path)?
    } else if let Ok(path) = std::env::var(config::TOKEN_FILE_ENV) {
        read_token_file(Path::new(&path))?
    } else if let Ok(token) = std::env::var(config::TOKEN_ENV) {
        token
    } else {
        bail!("pass --token, --token-file, RTERM_TOKEN, or RTERM_TOKEN_FILE")
    };

    normalize_token(raw)
}

pub fn normalize_token(raw: impl AsRef<str>) -> Result<String> {
    let token = raw.as_ref().trim().to_owned();
    anyhow::ensure!(
        token.len() >= config::MIN_TOKEN_LEN,
        "token must be at least 32 characters; generate one with: openssl rand -base64 32"
    );
    Ok(token)
}

pub fn validate_password(password: &str) -> Result<()> {
    anyhow::ensure!(
        password.len() <= config::MAX_PASSWORD_LEN,
        "password must be at most {} bytes",
        config::MAX_PASSWORD_LEN
    );
    anyhow::ensure!(
        !password.contains('\0'),
        "password must not contain NUL bytes"
    );
    Ok(())
}

pub fn auth_secret(token: &str, password: Option<&str>) -> String {
    match password.filter(|value| !value.is_empty()) {
        Some(password) => format!("{token}\0{password}"),
        None => token.to_string(),
    }
}

fn read_token_file(path: &Path) -> Result<String> {
    validate_token_file_permissions(path)?;
    std::fs::read_to_string(path).with_context(|| format!("read token file {}", path.display()))
}

#[cfg(unix)]
fn validate_token_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)
        .with_context(|| format!("inspect token file {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "token file {} must be a regular file",
        path.display()
    );

    let mode = metadata.permissions().mode();
    anyhow::ensure!(
        mode & config::TOKEN_FILE_UNIX_PRIVATE_MODE_MASK == 0,
        "token file {} must not be readable, writable, or executable by group/other; run: chmod 600 {}",
        path.display(),
        path.display()
    );
    Ok(())
}

#[cfg(not(unix))]
fn validate_token_file_permissions(path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("inspect token file {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "token file {} must be a regular file",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_normalization_trims_value() {
        assert_eq!(
            normalize_token(" abcdefghijklmnopqrstuvwxyz123456 \n").unwrap(),
            "abcdefghijklmnopqrstuvwxyz123456"
        );
    }

    #[test]
    fn token_normalization_rejects_short_value() {
        assert!(normalize_token("short").is_err());
    }

    #[test]
    fn auth_secret_combines_token_and_non_empty_password() {
        assert_eq!(auth_secret("token", Some("pw")), "token\0pw");
        assert_eq!(auth_secret("token", Some("")), "token");
        assert_eq!(auth_secret("token", None), "token");
    }

    #[test]
    fn password_validation_rejects_nul_and_excessive_length() {
        assert!(validate_password("correct horse battery staple").is_ok());
        assert!(validate_password("bad\0password").is_err());
        assert!(validate_password(&"x".repeat(config::MAX_PASSWORD_LEN + 1)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn token_loading_rejects_group_or_other_accessible_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_token_path("loose");
        std::fs::write(&path, "abcdefghijklmnopqrstuvwxyz123456").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(load_token(None, Some(path.clone())).is_err());

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn token_loading_uses_direct_value_before_file() {
        let path = temp_token_path("private");
        std::fs::write(&path, "abcdefghijklmnopqrstuvwxyz123456").unwrap();
        make_token_file_private(&path);

        assert_eq!(
            load_token(None, Some(path.clone())).unwrap(),
            "abcdefghijklmnopqrstuvwxyz123456"
        );
        assert_eq!(
            load_token(
                Some("12345678901234567890123456789012".to_owned()),
                Some(path.clone())
            )
            .unwrap(),
            "12345678901234567890123456789012"
        );

        std::fs::remove_file(path).unwrap();
    }

    fn temp_token_path(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let thread = std::thread::current().id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "rterm-{label}-token-{}-{thread:?}-{nanos}",
            std::process::id()
        ));
        path
    }

    #[cfg(unix)]
    fn make_token_file_private(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(not(unix))]
    fn make_token_file_private(_path: &std::path::Path) {}
}
