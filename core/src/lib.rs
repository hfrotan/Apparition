//! Apparition's native core. One process-wide Apple ID session, driven through a small
//! JSON-returning C ABI. Every call blocks; call it off the main thread.
//!
//! Secrets: the password is used once for sign-in and never stored or logged. The
//! certificate's private key and the anisette state live in `<data_dir>/storage.json`.

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use isideload::{
    anisette::remote_v3::RemoteV3AnisetteProvider,
    auth::apple_account::{AppleAccount, TwoFactorCallbackParams, TwoFactorCallbackResponse},
    dev::{developer_session::DeveloperSession, devices::DevicesApi, teams::DeveloperTeam},
    sideload::{SideloaderBuilder, TeamSelection, builder::MaxCertsBehavior, sideloader::Sideloader},
    util::{callbacks::MaxCertsCallbackBox, storage::SideloadingStorage},
};
use rootcause::prelude::*;
use serde::Serialize;
use serde_json::{Value, json};

// Keep the idevice FFI symbols in the static library.
pub use idevice_ffi;

struct Session {
    email: String,
    team: DeveloperTeam,
    sideloader: Sideloader<MaxCertsCallbackBox>,
}

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let _ = isideload::init();
        tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime")
    })
}

/// A key-value store in one JSON file. Holds the anisette state and the signing identity.
struct FileStorage {
    path: PathBuf,
    values: Mutex<HashMap<String, String>>,
}

impl FileStorage {
    fn open(path: PathBuf) -> Self {
        let values = std::fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        Self { path, values: Mutex::new(values) }
    }
}

impl SideloadingStorage for FileStorage {
    fn store(&self, key: &str, value: &str) -> Result<(), Report> {
        let mut values = self.values.lock().unwrap();
        if value.is_empty() {
            values.remove(key);
        } else {
            values.insert(key.to_string(), value.to_string());
        }
        std::fs::write(&self.path, serde_json::to_vec(&*values)?)?;
        Ok(())
    }

    fn retrieve(&self, key: &str) -> Result<Option<String>, Report> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }
}

/// Asked for a 2FA response. `params_json` describes the request; the callback returns a
/// string that stays valid until the next call: `code:<digits>`, `devices`, `sms:<id>`,
/// `resend` or `abort` (NULL also aborts).
pub type TwoFactorFn = extern "C" fn(context: *mut c_void, params_json: *const c_char) -> *const c_char;

struct Callback {
    function: TwoFactorFn,
    context: usize,
}

impl Callback {
    fn ask(&self, params: &TwoFactorCallbackParams) -> TwoFactorCallbackResponse {
        let numbers: Vec<Value> = params
            .numbers
            .iter()
            .map(|n| json!({"id": n.id, "number": n.number_with_dial_code}))
            .collect();
        let request = json!({
            "lastError": params.last_error,
            "unknown": params.unknown,
            "sms": params.sms,
            "numbers": numbers,
            "selectedNumberId": params.selected_number_id,
        });
        let request = CString::new(request.to_string()).unwrap_or_default();
        let reply = (self.function)(self.context as *mut c_void, request.as_ptr());
        if reply.is_null() {
            return TwoFactorCallbackResponse::Abort;
        }
        let reply = unsafe { CStr::from_ptr(reply) }.to_string_lossy().trim().to_string();
        match reply.split_once(':') {
            Some(("code", code)) => TwoFactorCallbackResponse::SubmitCode(code.trim().to_string()),
            Some(("sms", id)) => id
                .trim()
                .parse()
                .map(TwoFactorCallbackResponse::SendSms)
                .unwrap_or(TwoFactorCallbackResponse::Abort),
            _ => match reply.as_str() {
                "devices" => TwoFactorCallbackResponse::SendToDevices,
                "resend" => TwoFactorCallbackResponse::ResendCode,
                _ => TwoFactorCallbackResponse::Abort,
            },
        }
    }
}

fn reply(result: Result<Value, Report>) -> *mut c_char {
    let value = match result {
        Ok(Value::Object(mut map)) => {
            map.insert("ok".into(), true.into());
            Value::Object(map)
        }
        Ok(other) => json!({"ok": true, "value": other}),
        Err(error) => {
            let text = format!("{error}");
            // A security-key-only account answers the 2FA step with a WebAuthn challenge, and
            // isideload reports the raw response (account name, key names) as the error.
            if text.contains("fsaChallenge") {
                json!({
                    "ok": false,
                    "code": "securityKeysRequired",
                    "error": "This Apple ID uses security keys for two-factor authentication, which Apparition can't answer on the headset. Use an Apple ID without security keys.",
                })
            } else {
                json!({"ok": false, "error": text})
            }
        }
    };
    CString::new(value.to_string()).unwrap_or_default().into_raw()
}

unsafe fn text(pointer: *const c_char) -> Result<String, Report> {
    if pointer.is_null() {
        bail!("missing argument");
    }
    Ok(unsafe { CStr::from_ptr(pointer) }.to_str()?.to_string())
}

#[derive(Serialize)]
struct TeamInfo {
    id: String,
    name: Option<String>,
    kind: Option<String>,
}

fn team_info(team: &DeveloperTeam) -> TeamInfo {
    TeamInfo { id: team.team_id.clone(), name: team.name.clone(), kind: team.r#type.clone() }
}

/// Frees a string returned by any `apparition_*` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_string_free(string: *mut c_char) {
    if !string.is_null() {
        drop(unsafe { CString::from_raw(string) });
    }
}

/// Sends isideload's tracing output (info and up) to `log_path`. Call once, before anything else.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_init_logging(log_path: *const c_char) {
    let Ok(path) = (unsafe { text(log_path) }) else { return };
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
    let _ = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init();
}

/// Signs in with an Apple ID and opens a developer session, replacing any previous one.
/// Returns `{ok, email, team, teams}` or `{ok: false, error}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_login(
    email: *const c_char,
    password: *const c_char,
    anisette_url: *const c_char,
    data_dir: *const c_char,
    two_factor: TwoFactorFn,
    context: *mut c_void,
) -> *mut c_char {
    let callback = Callback { function: two_factor, context: context as usize };
    reply((|| {
        let email = unsafe { text(email) }?;
        let password = unsafe { text(password) }?;
        let anisette_url = unsafe { text(anisette_url) }?;
        let data_dir = PathBuf::from(unsafe { text(data_dir) }?);
        std::fs::create_dir_all(&data_dir)?;
        let storage_path = data_dir.join("storage.json");

        runtime().block_on(async move {
            let anisette = RemoteV3AnisetteProvider::new(
                &anisette_url,
                Box::new(FileStorage::open(storage_path.clone())),
                "0".to_string(),
            )?;
            let callback = &callback;
            let mut account = AppleAccount::builder(&email)
                .anisette_provider(anisette)
                .login(&password, |params: TwoFactorCallbackParams| {
                    let response = callback.ask(&params);
                    async move { Ok(response) }
                })
                .await
                .context("Apple ID sign-in failed")?;
            drop(password);

            let session = DeveloperSession::from_account(&mut account)
                .await
                .context("Failed to open a developer session")?;
            let mut sideloader: Sideloader<MaxCertsCallbackBox> = SideloaderBuilder::new(session, email.clone())
                .team_selection(TeamSelection::First)
                // Never revoke someone's existing certificates on our own: report instead.
                .max_certs_behavior(MaxCertsBehavior::Error)
                .storage(Box::new(FileStorage::open(storage_path)))
                .machine_name("Apparition".to_string())
                .delete_app_after_install(false)
                .build();
            let teams = isideload::dev::teams::TeamsApi::list_teams(sideloader.get_dev_session()).await?;
            let team = sideloader.get_team().await?;
            let result = json!({
                "email": email,
                "team": team_info(&team),
                "teams": teams.iter().map(team_info).collect::<Vec<_>>(),
            });
            *SESSION.lock().unwrap() = Some(Session { email, team, sideloader });
            Ok(result)
        })
    })())
}

/// The signed-in account, or `{ok, signedIn: false}`.
#[unsafe(no_mangle)]
pub extern "C" fn apparition_session() -> *mut c_char {
    let guard = SESSION.lock().unwrap();
    reply(Ok(match guard.as_ref() {
        Some(session) => json!({"signedIn": true, "email": session.email, "team": team_info(&session.team)}),
        None => json!({"signedIn": false}),
    }))
}

/// Registers the device on the team (tolerating "already registered"), then signs the
/// app or IPA at `app_path` for it. Returns `{ok, signedPath, devices}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_sign(
    app_path: *const c_char,
    udid: *const c_char,
    device_name: *const c_char,
    strip_extensions: bool,
) -> *mut c_char {
    reply((|| {
        let mut app_path = PathBuf::from(unsafe { text(app_path) }?);
        let mut stripped = Vec::new();
        if strip_extensions {
            (app_path, stripped) = extract_without_extensions(&app_path)?;
        }
        let udid = unsafe { text(udid) }?;
        let device_name = unsafe { text(device_name) }?;
        let mut guard = SESSION.lock().unwrap();
        let session = guard.as_mut().ok_or_else(|| report!("Not signed in"))?;
        let team = session.team.clone();

        runtime().block_on(async {
            let dev = session.sideloader.get_dev_session();
            dev.ensure_device_registered(&team, &device_name, &udid, None)
                .await
                .context("Device registration failed")?;
            let devices = dev.list_devices(&team, None).await?;
            let registered: Vec<Value> = devices
                .iter()
                .map(|d| json!({"name": d.name, "udid": d.device_number, "status": d.status}))
                .collect();

            let (signed, _) = session
                .sideloader
                .sign_app(app_path, Some(team), false, None::<fn(f32) -> std::future::Ready<()>>, None, Some(&udid))
                .await
                .context("Signing failed")?;
            Ok(json!({"signedPath": signed.to_string_lossy(), "devices": registered, "strippedExtensions": stripped}))
        })
    })())
}

/// Unpacks an IPA next to it and removes `Extensions/` (ExtensionKit) from the app, which a
/// re-signed app can't keep: installd rejects the extension under a renamed bundle ID.
/// Returns the unpacked .app and the names removed.
fn extract_without_extensions(ipa: &std::path::Path) -> Result<(PathBuf, Vec<String>), Report> {
    let target = std::env::temp_dir().join(format!(
        "{}_stripped",
        ipa.file_name().ok_or_else(|| report!("bad path"))?.to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&target)?;
    zip::ZipArchive::new(std::fs::File::open(ipa)?)?.extract(&target)?;
    let app = std::fs::read_dir(target.join("Payload"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or_else(|| report!("no .app in Payload"))?;
    let extensions = app.join("Extensions");
    let mut removed = Vec::new();
    if extensions.is_dir() {
        for entry in std::fs::read_dir(&extensions)?.filter_map(Result::ok) {
            removed.push(entry.file_name().to_string_lossy().to_string());
        }
        std::fs::remove_dir_all(&extensions)?;
    }
    Ok((app, removed))
}

/// Installs a signed app over an open RSD tunnel (from idevice's `tunnel_create_rppairing`):
/// AFC upload to PublicStaging, then installation_proxy. The handles are borrowed.
/// Returns `{ok, elapsedMs}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_install(
    adapter: *mut idevice_ffi::core_device_proxy::AdapterHandle,
    handshake: *mut idevice_ffi::rsd::RsdHandshakeHandle,
    app_path: *const c_char,
) -> *mut c_char {
    reply((|| {
        if adapter.is_null() || handshake.is_null() {
            bail!("missing tunnel");
        }
        let app_path = PathBuf::from(unsafe { text(app_path) }?);
        let adapter = unsafe { &mut (*adapter).0 };
        let handshake = unsafe { &mut (*handshake).0 };
        let started = std::time::Instant::now();
        runtime().block_on(async {
            isideload::sideload::install::install_app_rsd(adapter, handshake, &app_path, |progress| {
                tracing::info!("install progress {progress}%");
            })
            .await
            .context("Install failed")?;
            Ok(json!({"elapsedMs": started.elapsed().as_millis() as u64}))
        })
    })())
}
