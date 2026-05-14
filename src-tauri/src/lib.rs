mod client;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod host;
pub mod p2p_probe;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod pty;

use base64::{engine::general_purpose, Engine as _};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::{fs, path::PathBuf};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::Manager;
use tauri::{ipc::Channel, State};
use tokio::sync::mpsc;

const MAX_ENDPOINT_INPUT_LEN: usize = 256;
const MAX_ROOM_INPUT_LEN: usize = 64;
const MAX_TOKEN_INPUT_LEN: usize = 512;
const MAX_SESSION_ID_INPUT_LEN: usize = 64;

#[derive(Clone, Default)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

#[derive(Default)]
struct AppStateInner {
    host: Mutex<Option<HostServiceInfo>>,
    host_error: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    tracker_key: Mutex<Option<String>>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    host_settings: Mutex<HostSettings>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Clone, Debug, Default)]
pub struct HostSettings {
    pub shell: TerminalShell,
    pub password: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalShell {
    Cmd,
    #[default]
    PowerShell,
    Wsl,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostServiceInfo {
    pub platform: String,
    pub running: bool,
    pub status: String,
    pub listen: String,
    pub lan_address: Option<String>,
    pub cert_sha256: String,
    pub first_run: bool,
    pub shell: TerminalShell,
    pub password_enabled: bool,
    pub pairing: PairingPayload,
    pub pairing_json: String,
    pub qr_svg: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingPayload {
    pub v: u8,
    pub mode: String,
    pub host: Option<String>,
    pub token: String,
    pub cert_sha256: String,
    pub tracker: Option<String>,
    pub tracker_room: Option<String>,
    pub relay: Option<String>,
    pub relay_cert_sha256: Option<String>,
    pub requires_password: Option<bool>,
    #[serde(skip_serializing)]
    pub password: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum TerminalEvent {
    Status(String),
    Output(String),
    Error(String),
    Closed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub platform: String,
    pub is_desktop_host: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOption {
    pub shell: TerminalShell,
    pub label: String,
    pub available: bool,
}

impl AppState {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn set_host_info(&self, info: HostServiceInfo) {
        if let Ok(mut host) = self.inner.host.lock() {
            *host = Some(info);
        }
        if let Ok(mut err) = self.inner.host_error.lock() {
            *err = None;
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn set_host_error(&self, error: String) {
        if let Ok(mut err) = self.inner.host_error.lock() {
            *err = Some(error);
        }
    }

    fn host_info(&self) -> Result<HostServiceInfo, String> {
        let host = self
            .inner
            .host
            .lock()
            .map_err(|_| "host state lock poisoned".to_string())?;
        if let Some(info) = host.clone() {
            return Ok(info);
        }
        drop(host);

        let err = self
            .inner
            .host_error
            .lock()
            .map_err(|_| "host error lock poisoned".to_string())?;
        if let Some(error) = err.clone() {
            Err(error)
        } else if cfg!(any(target_os = "android", target_os = "ios")) {
            Err("host service is only available on desktop".to_string())
        } else {
            Err("host service is starting".to_string())
        }
    }

    fn insert_session(&self, session_id: String, tx: mpsc::Sender<Vec<u8>>) -> Result<(), String> {
        self.inner
            .sessions
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?
            .insert(session_id, tx);
        Ok(())
    }

    fn remove_session(&self, session_id: &str) {
        if let Ok(mut sessions) = self.inner.sessions.lock() {
            sessions.remove(session_id);
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn host_settings(&self) -> Result<HostSettings, String> {
        self.inner
            .host_settings
            .lock()
            .map_err(|_| "host settings lock poisoned".to_string())
            .map(|settings| settings.clone())
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn update_host_settings(
        &self,
        shell: TerminalShell,
        password: Option<String>,
    ) -> Result<(), String> {
        let password = password.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        if let Some(password) = password.as_deref() {
            rterm_protocol::validate_password(password).map_err(|err| err.to_string())?;
        }
        *self
            .inner
            .host_settings
            .lock()
            .map_err(|_| "host settings lock poisoned".to_string())? = HostSettings {
            shell: shell.clone(),
            password: password.clone(),
        };
        if let Ok(mut host) = self.inner.host.lock() {
            if let Some(info) = host.as_mut() {
                info.shell = shell;
                info.password_enabled = password.is_some();
                info.pairing.requires_password = Some(password.is_some());
                if let Ok(json) = crate::host::compact_pairing_json(&info.pairing) {
                    info.pairing_json = json;
                    if let Ok(svg) = crate::host::render_qr_svg(&info.pairing_json) {
                        info.qr_svg = svg;
                    }
                }
            }
        }
        Ok(())
    }

    fn session_sender(&self, session_id: &str) -> Result<mpsc::Sender<Vec<u8>>, String> {
        self.inner
            .sessions
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?
            .get(session_id)
            .cloned()
            .ok_or_else(|| "terminal session is not connected".to_string())
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn tracker_is_running(&self, key: &str) -> Result<bool, String> {
        let mut tracker_key = self
            .inner
            .tracker_key
            .lock()
            .map_err(|_| "tracker state lock poisoned".to_string())?;
        if tracker_key.as_deref() == Some(key) {
            return Ok(true);
        }
        *tracker_key = Some(key.to_string());
        Ok(false)
    }
}

#[tauri::command]
fn runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        platform: std::env::consts::OS.to_string(),
        is_desktop_host: !cfg!(any(target_os = "android", target_os = "ios")),
    }
}

#[tauri::command]
fn service_info(state: State<'_, AppState>) -> Result<HostServiceInfo, String> {
    state.host_info()
}

#[tauri::command]
fn available_terminals() -> Vec<TerminalOption> {
    vec![
        TerminalOption {
            shell: TerminalShell::PowerShell,
            label: "PowerShell".to_string(),
            available: shell_available(&TerminalShell::PowerShell),
        },
        TerminalOption {
            shell: TerminalShell::Cmd,
            label: "Command Prompt".to_string(),
            available: shell_available(&TerminalShell::Cmd),
        },
        TerminalOption {
            shell: TerminalShell::Wsl,
            label: "WSL".to_string(),
            available: shell_available(&TerminalShell::Wsl),
        },
    ]
}

fn shell_available(shell: &TerminalShell) -> bool {
    #[cfg(windows)]
    {
        let program = match shell {
            TerminalShell::PowerShell => "powershell.exe",
            TerminalShell::Cmd => "cmd.exe",
            TerminalShell::Wsl => "wsl.exe",
        };
        std::process::Command::new("where")
            .arg(program)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        matches!(
            shell,
            TerminalShell::PowerShell | TerminalShell::Cmd | TerminalShell::Wsl
        ) && false
    }
}

#[tauri::command]
fn pairing_payload(state: State<'_, AppState>) -> Result<PairingPayload, String> {
    Ok(state.host_info()?.pairing)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
fn update_host_settings(
    shell: TerminalShell,
    password: Option<String>,
    state: State<'_, AppState>,
) -> Result<HostServiceInfo, String> {
    state.update_host_settings(shell, password)?;
    state.host_info()
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
fn update_host_settings(
    _shell: TerminalShell,
    _password: Option<String>,
    _state: State<'_, AppState>,
) -> Result<HostServiceInfo, String> {
    Err("host settings are configured on desktop".to_string())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
fn enable_tracker_pairing(
    tracker: String,
    tracker_room: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<HostServiceInfo, String> {
    let tracker = sanitize_endpoint("tracker address", &tracker)?;
    let room = match tracker_room {
        Some(value) if !value.trim().is_empty() => sanitize_room(&value)?,
        _ => rterm_protocol::DEFAULT_TRACKER_ROOM.to_string(),
    };
    let key = format!("{tracker}\0{room}");
    let already_running = state.tracker_is_running(&key)?;
    let info =
        host::enable_tracker_pairing(app, state.inner().clone(), tracker, room, already_running)
            .map_err(|err| err.to_string())?;
    Ok(info)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
fn enable_relay_pairing(
    relay: String,
    relay_cert_sha256: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<HostServiceInfo, String> {
    let relay = sanitize_endpoint("relay address", &relay)?;
    let relay_cert_sha256 = relay_cert_sha256
        .map(|pin| sanitize_fingerprint("relay certificate pin", &pin))
        .transpose()?;
    let key = format!(
        "relay\0{relay}\0{}",
        relay_cert_sha256.as_deref().unwrap_or("")
    );
    let already_running = state.tracker_is_running(&key)?;
    host::enable_relay_pairing(
        app,
        state.inner().clone(),
        relay,
        relay_cert_sha256,
        already_running,
    )
    .map_err(|err| err.to_string())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
fn enable_relay_pairing(
    _relay: String,
    _relay_cert_sha256: Option<String>,
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
) -> Result<HostServiceInfo, String> {
    Err("relay pairing is configured on the desktop host".to_string())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
fn enable_tracker_pairing(
    _tracker: String,
    _tracker_room: Option<String>,
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
) -> Result<HostServiceInfo, String> {
    Err("tracker pairing is configured on the desktop host".to_string())
}

#[tauri::command]
async fn connect_terminal(
    pairing: PairingPayload,
    on_event: Channel<TerminalEvent>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    validate_pairing_payload(&pairing)?;
    let session_id = new_session_id();
    let (tx, rx) = mpsc::channel::<Vec<u8>>(rterm_protocol::config::STDIN_CHANNEL_CAPACITY);
    state.insert_session(session_id.clone(), tx)?;
    client::spawn_client_session(
        state.inner().clone(),
        session_id.clone(),
        pairing,
        rx,
        on_event,
    );
    Ok(session_id)
}

#[tauri::command]
async fn send_terminal_input(
    session_id: String,
    data: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_session_id(&session_id)?;
    if data.len() > rterm_protocol::config::MAX_FRAME {
        return Err(format!(
            "terminal input must be at most {} bytes",
            rterm_protocol::config::MAX_FRAME
        ));
    }
    let tx = state.session_sender(&session_id)?;
    tx.send(data.into_bytes())
        .await
        .map_err(|_| "terminal session is closed".to_string())
}

#[tauri::command]
fn disconnect_terminal(session_id: String, state: State<'_, AppState>) -> Result<(), String> {
    validate_session_id(&session_id)?;
    state.remove_session(&session_id);
    Ok(())
}

#[tauri::command]
async fn test_internet_p2p(
    tracker: String,
    room: String,
    on_event: Channel<TerminalEvent>,
) -> Result<(), String> {
    let tracker = sanitize_endpoint("tracker address", &tracker)?;
    let room = if room.trim().is_empty() {
        "sterm-probe".to_string()
    } else {
        sanitize_room(&room)?
    };
    p2p_probe::run(tracker, room, on_event)
        .await
        .map_err(|err| err.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    rterm_protocol::install_crypto_provider();
    let state = AppState::default();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state.clone())
        .setup(move |_app| {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let app_handle = _app.handle().clone();
                let state = state.clone();
                let (token, first_run) = load_or_create_token(_app.path().app_data_dir()?)?;
                tauri::async_runtime::spawn(async move {
                    host::start_host_service(app_handle, state, token, first_run).await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_info,
            service_info,
            available_terminals,
            pairing_payload,
            enable_tracker_pairing,
            enable_relay_pairing,
            update_host_settings,
            connect_terminal,
            send_terminal_input,
            disconnect_terminal,
            test_internet_p2p,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn load_or_create_token(
    app_data_dir: PathBuf,
) -> Result<(String, bool), Box<dyn std::error::Error>> {
    fs::create_dir_all(&app_data_dir)?;
    let path = app_data_dir.join("pairing-token");
    if path.exists() {
        let token = fs::read_to_string(path)?;
        return Ok((rterm_protocol::normalize_token(&token)?, false));
    }

    let token = generate_token();
    fs::write(path, &token)?;
    Ok((token, true))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn generate_token() -> String {
    let mut bytes = [0u8; 48];
    OsRng.fill_bytes(&mut bytes);
    general_purpose::STANDARD_NO_PAD.encode(bytes)
}

fn validate_pairing_payload(pairing: &PairingPayload) -> Result<(), String> {
    match pairing.mode.as_str() {
        "direct" => {
            sanitize_optional_endpoint("host address", pairing.host.as_deref())?
                .ok_or_else(|| "direct pairing is missing host".to_string())?;
        }
        "tracker" => {
            sanitize_optional_endpoint("host address", pairing.host.as_deref())?;
            sanitize_optional_endpoint("tracker address", pairing.tracker.as_deref())?
                .ok_or_else(|| "tracker pairing is missing tracker".to_string())?;
            if let Some(room) = pairing.tracker_room.as_deref() {
                sanitize_room(room)?;
            }
        }
        "relay" => {
            sanitize_optional_endpoint("relay address", pairing.relay.as_deref())?
                .ok_or_else(|| "relay pairing is missing relay".to_string())?;
            if let Some(pin) = pairing.relay_cert_sha256.as_deref() {
                sanitize_fingerprint("relay certificate pin", pin)?;
            }
        }
        _ => return Err("unsupported pairing mode".to_string()),
    }
    sanitize_text("pairing token", &pairing.token, MAX_TOKEN_INPUT_LEN)?;
    sanitize_fingerprint("certificate pin", &pairing.cert_sha256)?;
    if let Some(password) = pairing.password.as_deref() {
        rterm_protocol::validate_password(password).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty() || session_id.len() > MAX_SESSION_ID_INPUT_LEN {
        return Err("invalid terminal session id".to_string());
    }
    if !session_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("invalid terminal session id".to_string());
    }
    Ok(())
}

fn sanitize_optional_endpoint(label: &str, value: Option<&str>) -> Result<Option<String>, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| sanitize_endpoint(label, value))
        .transpose()
}

fn sanitize_endpoint(label: &str, value: &str) -> Result<String, String> {
    let value = sanitize_text(label, value, MAX_ENDPOINT_INPUT_LEN)?;
    if value.is_empty() {
        return Err(format!("{label} is required"));
    }
    if value.contains(['/', '\\']) && !value.starts_with("udp://") {
        return Err(format!("{label} must be a host:port value"));
    }
    Ok(value)
}

fn sanitize_room(value: &str) -> Result<String, String> {
    let value = sanitize_text("tracker room", value, MAX_ROOM_INPUT_LEN)?;
    if value.is_empty() {
        return Err("tracker room is required".to_string());
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "tracker room may only contain letters, numbers, '.', '-', and '_'".to_string(),
        );
    }
    Ok(value)
}

fn sanitize_fingerprint(label: &str, value: &str) -> Result<String, String> {
    let value = sanitize_text(label, value, rterm_protocol::config::SHA256_HEX_LEN + 31)?;
    rterm_protocol::parse_sha256_fingerprint(&value).map_err(|err| err.to_string())?;
    Ok(value)
}

fn sanitize_text(label: &str, value: &str, max_len: usize) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.len() > max_len {
        return Err(format!("{label} must be at most {max_len} bytes"));
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '<' | '>' | '"' | '\'' | '`'))
    {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(value)
}

fn new_session_id() -> String {
    let mut bytes = [0u8; 12];
    OsRng.fill_bytes(&mut bytes);
    general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
