//! The connector as a Windows service (#166): installing, updating and
//! removing it, the service itself, and its entries in the event log.
//!
//! - The program: `C:\Program Files\remotehub-connector\remotehub-connector.exe`
//! - Its data: `C:\ProgramData\remotehub-connector`, readable only by SYSTEM,
//!   Administrators and the service's account, `LocalService`; the settings
//!   are in `connector.conf` there ([`crate::settings::CONF`])
//! - The service: `remotehub-connector`, started automatically, restarted
//!   after a failure
//! - The event log: Application, source `remotehub-connector`

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use remotehub_i18n::{self as i18n, Locale, Message};
use tokio_tungstenite::tungstenite::http::Uri;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::daemon::{self, Logs};
use crate::{VERSION, files, settings};

pub const SERVICE: &str = "remotehub-connector";
const EXE: &str = "remotehub-connector.exe";
/// The account the service runs as: no rights beyond the machine's own, and
/// network access in the machine's name.
const ACCOUNT: &str = r"NT AUTHORITY\LocalService";

fn program_dir() -> PathBuf {
    let base = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_owned());
    PathBuf::from(base).join("remotehub-connector")
}

fn data_dir() -> anyhow::Result<PathBuf> {
    Ok(settings::data_dir(&|name: &str| std::env::var(name).ok())?)
}

fn say(locale: Locale, message: Message) {
    println!("{}", i18n::render(locale, &message));
}

// ── The service ─────────────────────────────────────────────────────────────

define_windows_service!(ffi_service_main, service_main);

/// Hands the process to the service manager, which calls [`service_main`];
/// returns when the service has stopped. Only the service manager starts
/// the program this way (`remotehub-connector service`).
pub fn dispatch() -> anyhow::Result<()> {
    service_dispatcher::start(SERVICE, ffi_service_main)
        .context("not started by the service manager")
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = run_service() {
        eventlog::report(eventlog::Kind::Error, 100, &format!("{error:#}"));
    }
}

fn run_service() -> anyhow::Result<()> {
    let stop = Arc::new(tokio::sync::Notify::new());
    let handler = {
        let stop = stop.clone();
        move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                stop.notify_one();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status = service_control_handler::register(SERVICE, handler)?;
    let report = |state: ServiceState, exit_code: u32| {
        status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(exit_code),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
    };
    report(ServiceState::Running, 0)?;
    let result = (|| {
        let data = data_dir()?;
        daemon::init_logs(&data, Logs::File)?;
        tokio::runtime::Runtime::new()?.block_on(async {
            tokio::select! {
                result = daemon::run(&data) => result,
                () = stop.notified() => {
                    tracing::info!("stopping");
                    Ok(())
                }
            }
        })
    })();
    // A non-zero exit code makes the service manager restart the service.
    report(ServiceState::Stopped, u32::from(result.is_err()))?;
    result
}

// ── Installing ──────────────────────────────────────────────────────────────

fn manager(access: ServiceManagerAccess) -> anyhow::Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access)
        .context("cannot reach the service manager: run this as an administrator")
}

/// Installs and starts the service for the remotehub at `url`, with the
/// token read from standard input, never from the command line.
pub fn install(url: &str, allow: Option<&str>, locale: Locale) -> anyhow::Result<()> {
    let parsed: Uri = url.parse().context("--url: not an address")?;
    if !matches!(parsed.scheme_str(), Some("https" | "http")) || parsed.host().is_none() {
        bail!("--url: an address like https://remotehub.example.com");
    }
    let manager = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    if manager
        .open_service(SERVICE, ServiceAccess::QUERY_STATUS)
        .is_ok()
    {
        bail!(
            "the service {SERVICE} exists already: `update` replaces the program, `uninstall` removes it"
        );
    }

    say(locale, Message::ConnectorCliTokenPrompt {});
    std::io::stdout().flush()?;
    let mut token = zeroize::Zeroizing::new(String::new());
    std::io::stdin().lock().read_line(&mut token)?;
    let token = token.trim();
    if !token.starts_with("rhc_") {
        bail!("that is not a connector token: it starts with rhc_");
    }

    let program = program_dir();
    std::fs::create_dir_all(&program)?;
    let exe = program.join(EXE);
    copy_self(&exe)?;

    let data = data_dir()?;
    std::fs::create_dir_all(&data)?;
    restrict(&data)?;
    let token_file = data.join("token");
    files::write_private(&token_file, token.as_bytes())?;
    let mut conf = format!(
        "# Settings of the remotehub connector; see docs/configuration.md.\r\n\
         REMOTEHUB_URL={url}\r\n\
         REMOTEHUB_CONNECTOR_TOKEN_FILE={}\r\n",
        token_file.display()
    );
    if let Some(allow) = allow {
        conf.push_str(&format!("REMOTEHUB_CONNECTOR_ALLOW={allow}\r\n"));
    }
    std::fs::write(data.join(settings::CONF), conf)?;
    register_event_source()?;

    let info = ServiceInfo {
        name: SERVICE.into(),
        display_name: "remotehub connector".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec!["service".into()],
        dependencies: Vec::new(),
        account_name: Some(ACCOUNT.into()),
        account_password: None,
    };
    let service =
        manager.create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
    service.set_description(
        "Reaches devices in this network for remotehub while the customer keeps access open.",
    )?;
    let restart = ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(10),
    };
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart.clone(), restart.clone(), restart]),
    })?;
    service.set_failure_actions_on_non_crash_failures(true)?;
    service.start::<&str>(&[])?;
    say(
        locale,
        Message::ConnectorCliInstalled {
            url: url.to_owned(),
        },
    );
    Ok(())
}

/// Replaces the installed program with this one and restarts the service.
pub fn update(locale: Locale) -> anyhow::Result<()> {
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(
            SERVICE,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START,
        )
        .with_context(|| format!("the service {SERVICE} is not installed"))?;
    stop(&service)?;
    copy_self(&program_dir().join(EXE))?;
    // Releases before #166 registered no event source.
    register_event_source()?;
    service.start::<&str>(&[])?;
    say(
        locale,
        Message::ConnectorCliUpdated {
            version: VERSION.to_owned(),
        },
    );
    Ok(())
}

/// Stops and removes the service and the program; the data only with
/// `purge`.
pub fn uninstall(purge: bool, locale: Locale) -> anyhow::Result<()> {
    let program = program_dir();
    if std::env::current_exe()?.starts_with(&program) {
        bail!(
            "run the uninstall from a copy of remotehub-connector.exe outside {}: Windows keeps a running program's file",
            program.display()
        );
    }
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    if let Ok(service) = manager.open_service(
        SERVICE,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        stop(&service)?;
        service.delete()?;
    }
    if program.exists() {
        std::fs::remove_dir_all(&program)
            .with_context(|| format!("removing {}", program.display()))?;
    }
    let _ = Command::new("reg")
        .args(["delete", EVENT_SOURCE_KEY, "/f"])
        .output();
    say(locale, Message::ConnectorCliUninstalled {});
    let data = data_dir()?;
    if purge {
        if data.exists() {
            std::fs::remove_dir_all(&data)
                .with_context(|| format!("removing {}", data.display()))?;
        }
    } else if data.exists() {
        say(
            locale,
            Message::ConnectorCliDataKept {
                path: data.display().to_string(),
            },
        );
    }
    Ok(())
}

fn stop(service: &windows_service::service::Service) -> anyhow::Result<()> {
    if service.query_status()?.current_state == ServiceState::Stopped {
        return Ok(());
    }
    service.stop()?;
    for _ in 0..60 {
        if service.query_status()?.current_state == ServiceState::Stopped {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("the service {SERVICE} did not stop within 30 seconds")
}

/// Copies the running program to `target`, unless it is running from there.
fn copy_self(target: &Path) -> anyhow::Result<()> {
    let own = std::env::current_exe()?;
    if own == target {
        return Ok(());
    }
    std::fs::copy(&own, target).with_context(|| format!("copying to {}", target.display()))?;
    Ok(())
}

/// Leaves the data directory to SYSTEM, the Administrators and the service's
/// account, for the directory and all it holds. Well-known SIDs, so the
/// names do not depend on the system's language.
fn restrict(dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("icacls")
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
            "*S-1-5-19:(OI)(CI)M",
            "/T",
            "/Q",
        ])
        .output()
        .context("running icacls")?;
    if !output.status.success() {
        bail!(
            "icacls {}: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
    Ok(())
}

const EVENT_SOURCE_KEY: &str =
    r"HKLM\SYSTEM\CurrentControlSet\Services\EventLog\Application\remotehub-connector";

/// Makes the event viewer show the connector's entries as written. The
/// message file of .NET, part of every current Windows, has a message that
/// is only its text for every event ID.
fn register_event_source() -> anyhow::Result<()> {
    for arguments in [
        [
            "/v",
            "EventMessageFile",
            "/t",
            "REG_EXPAND_SZ",
            "/d",
            r"%SystemRoot%\Microsoft.NET\Framework64\v4.0.30319\EventLogMessages.dll",
        ],
        ["/v", "TypesSupported", "/t", "REG_DWORD", "/d", "7"],
    ] {
        let output = Command::new("reg")
            .args(["add", EVENT_SOURCE_KEY])
            .args(arguments)
            .arg("/f")
            .output()
            .context("running reg")?;
        if !output.status.success() {
            bail!(
                "reg add {EVENT_SOURCE_KEY}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    Ok(())
}

/// The language of the signed-in Windows user, for the command line's
/// output.
#[allow(unsafe_code)]
pub fn user_locale() -> Option<Locale> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    // LOCALE_NAME_MAX_LENGTH
    let mut name = [0u16; 85];
    // SAFETY: the buffer is as long as the length passed; the function
    // writes at most that many characters.
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    let length = usize::try_from(length).ok().filter(|&l| l > 0)?;
    Locale::of_tag(&String::from_utf16_lossy(&name[..length - 1]))
}

// ── The event log ───────────────────────────────────────────────────────────

pub mod eventlog {
    use std::sync::OnceLock;

    use windows_sys::Win32::System::EventLog::{
        EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE,
        RegisterEventSourceW, ReportEventW,
    };

    use crate::journal::Event;

    pub enum Kind {
        Information,
        Warning,
        Error,
    }

    /// A journal entry as an event: ID 1 for changes of the access and of
    /// the list of devices, 2 for connections, 3 for sign-ins to the web
    /// interface. `line` is the journal's JSON line, which log collectors
    /// can read.
    pub fn journal(event: &Event, line: &str) {
        let (kind, id) = match event {
            Event::Opened { .. }
            | Event::Closed { .. }
            | Event::Expired { .. }
            | Event::DeviceAdded { .. }
            | Event::DeviceRemoved { .. }
            | Event::GroupAdded { .. }
            | Event::GroupRemoved { .. } => (Kind::Information, 1),
            Event::ConnectionStarted { .. } | Event::ConnectionEnded { .. } => {
                (Kind::Information, 2)
            }
            Event::ConnectionRefused { .. } => (Kind::Warning, 2),
            Event::SignedIn { .. } => (Kind::Information, 3),
            Event::SignInFailed { .. } | Event::Locked { .. } => (Kind::Warning, 3),
        };
        report(kind, id, line);
    }

    /// The handle of the event source, as a number: a raw handle is not
    /// `Sync`. Zero if Windows refused it.
    static SOURCE: OnceLock<usize> = OnceLock::new();

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Writes `text` to the Application log. Failing to is not reported:
    /// the journal file and the log output have every entry anyway.
    #[allow(unsafe_code)]
    pub fn report(kind: Kind, id: u32, text: &str) {
        let source = *SOURCE.get_or_init(|| {
            let name = wide(super::SERVICE);
            // SAFETY: `name` is a NUL-terminated UTF-16 string that lives
            // through the call; a null server name means this computer.
            unsafe { RegisterEventSourceW(std::ptr::null(), name.as_ptr()) as usize }
        });
        if source == 0 {
            return;
        }
        let kind = match kind {
            Kind::Information => EVENTLOG_INFORMATION_TYPE,
            Kind::Warning => EVENTLOG_WARNING_TYPE,
            Kind::Error => EVENTLOG_ERROR_TYPE,
        };
        let text = wide(text);
        let strings = [text.as_ptr()];
        // SAFETY: `source` came from RegisterEventSourceW and is never
        // closed; `strings` holds one NUL-terminated string, as the count
        // says, and both outlive the call. No SID and no raw data.
        unsafe {
            ReportEventW(
                source as _,
                kind,
                0,
                id,
                std::ptr::null_mut(),
                1,
                0,
                strings.as_ptr(),
                std::ptr::null(),
            );
        }
    }
}
