//! One browser session per connection from remotehub: an Xvnc display with
//! a random VNC password, a Chromium on it with a fresh profile that reaches
//! only the device, and the sign-in. Everything ends with the connection,
//! and the connection ends with Chromium or Xvnc.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::{Child, Command};
use zeroize::Zeroizing;

use crate::cdp::Cdp;
use crate::fill::{self, Outcome};
use crate::protocol::{self, MAX_LINE, Open, Reply};
use crate::proxy;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Xvnc creates its socket within a second; slow machines get more.
const DISPLAY_TIMEOUT: Duration = Duration::from_secs(10);
/// VNC's own authentication compares at most eight characters.
const VNC_PASSWORD_LENGTH: usize = 8;

pub struct Settings {
    pub chromium: String,
    pub xvnc: String,
    pub vncpasswd: String,
    /// Profiles and VNC password files, one directory per display; a tmpfs.
    pub work: PathBuf,
    /// Display numbers to hand out; display n listens for VNC on 5900 + n.
    pub displays: std::ops::RangeInclusive<u16>,
    /// How long to wait for the device's sign-in form.
    pub fill_patience: Duration,
}

/// The display numbers in use.
#[derive(Default)]
pub struct Displays(Mutex<Vec<u16>>);

/// A display number, returned when dropped.
struct Display<'a> {
    number: u16,
    displays: &'a Displays,
}

impl Displays {
    fn take(&self, range: &std::ops::RangeInclusive<u16>) -> Option<Display<'_>> {
        let mut used = self.0.lock().expect("no panics while locked");
        let number = range.clone().find(|n| !used.contains(n))?;
        used.push(number);
        Some(Display {
            number,
            displays: self,
        })
    }
}

impl Drop for Display<'_> {
    fn drop(&mut self) {
        self.displays
            .0
            .lock()
            .expect("no panics while locked")
            .retain(|n| *n != self.number);
    }
}

/// Serves one connection from remotehub until the session ends.
pub async fn serve(stream: TcpStream, settings: Arc<Settings>, displays: Arc<Displays>) {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    let Some(open) = read_open(&mut read).await else {
        reply(&mut write, &failed("invalid_request")).await;
        return;
    };
    if !open.valid_host() || open.spki.is_empty() {
        reply(&mut write, &failed("invalid_request")).await;
        return;
    }
    let Some(display) = displays.take(&settings.displays) else {
        tracing::warn!("every display is in use");
        reply(&mut write, &failed("busy")).await;
        return;
    };
    let number = display.number;
    let dir = settings.work.join(number.to_string());
    let result = run(&open, &settings, number, &dir, read, &mut write).await;
    if let Err(reason) = result {
        tracing::warn!(display = number, host = %open.host, reason, "browser session failed");
        reply(&mut write, &failed(reason)).await;
    }
    let _ = tokio::fs::remove_dir_all(&dir).await;
    remove_display_files(number).await;
}

async fn run(
    open: &Open,
    settings: &Settings,
    number: u16,
    dir: &Path,
    mut read: BufReader<OwnedReadHalf>,
    write: &mut OwnedWriteHalf,
) -> Result<(), &'static str> {
    let _ = tokio::fs::remove_dir_all(dir).await;
    tokio::fs::create_dir_all(dir.join("home"))
        .await
        .map_err(|_| "work_dir")?;
    let password = vnc_password();
    write_vnc_password(&settings.vncpasswd, &password, &dir.join("passwd")).await?;

    let mut xvnc = start_xvnc(settings, number, open, dir).await?;
    let (proxy_port, proxy) = proxy::start(open.authority(), open.via.clone())
        .await
        .map_err(|_| "proxy")?;
    let _proxy = AbortOnDrop(proxy);
    let mut chromium = start_chromium(settings, number, open, dir, proxy_port)?;
    let group = chromium.id();
    let _group = KillGroupOnDrop(group);
    let mut cdp = Cdp::new(
        chromium.stdin.take().ok_or("chromium")?,
        chromium.stdout.take().ok_or("chromium")?,
    );

    tracing::info!(display = number, host = %open.host, "browser session started");
    reply(
        write,
        &Reply::Ready {
            vnc_port: 5900 + number,
            vnc_password: password.to_string(),
        },
    )
    .await;

    // Sign in, unless remotehub hangs up or the browser ends first.
    let pipe = if let Some(login) = &open.login {
        let origin = open.origin();
        let filling = fill::fill(
            &mut cdp,
            &origin,
            &login.username,
            &login.password,
            settings.fill_patience,
        );
        tokio::select! {
            outcome = filling => {
                let answer = match outcome {
                    Ok(Outcome::Filled) => Reply::Filled,
                    Ok(Outcome::NoForm) => Reply::NotFilled { reason: "no_form".into() },
                    Ok(Outcome::PageError) => Reply::NotFilled { reason: "page_error".into() },
                    Err(error) => {
                        tracing::warn!(display = number, %error, "sign-in failed");
                        Reply::NotFilled { reason: "browser".into() }
                    }
                };
                reply(write, &answer).await;
            }
            () = closed(&mut read) => return Ok(()),
            _ = chromium.wait() => return Ok(()),
            _ = xvnc.wait() => return Ok(()),
        }
        cdp.idle()
    } else {
        cdp.idle()
    };

    tokio::select! {
        () = closed(&mut read) => {}
        _ = chromium.wait() => tracing::info!(display = number, "chromium ended"),
        _ = xvnc.wait() => tracing::info!(display = number, "the display ended"),
    }
    drop(pipe);
    tracing::info!(display = number, "browser session ended");
    Ok(())
}

async fn read_open(read: &mut BufReader<OwnedReadHalf>) -> Option<Open> {
    let mut line = Zeroizing::new(String::new());
    let mut limited = read.take(MAX_LINE as u64);
    tokio::time::timeout(REQUEST_TIMEOUT, limited.read_line(&mut line))
        .await
        .ok()?
        .ok()?;
    serde_json::from_str(&line).ok()
}

/// Resolves when remotehub closes the connection; it sends nothing after
/// the first line.
async fn closed(read: &mut BufReader<OwnedReadHalf>) {
    let mut buffer = [0u8; 64];
    while read.read(&mut buffer).await.is_ok_and(|n| n > 0) {}
}

async fn reply(write: &mut OwnedWriteHalf, reply: &Reply) {
    let mut line = serde_json::to_vec(reply).expect("replies serialize");
    line.push(b'\n');
    let _ = write.write_all(&line).await;
}

fn failed(reason: &str) -> Reply {
    Reply::Failed {
        reason: reason.to_owned(),
    }
}

fn vnc_password() -> Zeroizing<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut bytes = [0u8; VNC_PASSWORD_LENGTH];
    getrandom::fill(&mut bytes).expect("the OS has randomness");
    // 256 is not a multiple of the alphabet's 57 letters; the slight bias
    // does not matter for a password that lives as long as one session.
    Zeroizing::new(
        bytes
            .iter()
            .map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()]))
            .collect(),
    )
}

/// Xvnc reads its password from a file in VNC's obfuscated format, which
/// `vncpasswd -f` writes.
async fn write_vnc_password(
    vncpasswd: &str,
    password: &str,
    file: &Path,
) -> Result<(), &'static str> {
    let mut child = Command::new(vncpasswd)
        .arg("-f")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "vncpasswd")?;
    let mut stdin = child.stdin.take().ok_or("vncpasswd")?;
    stdin
        .write_all(format!("{password}\n").as_bytes())
        .await
        .map_err(|_| "vncpasswd")?;
    drop(stdin);
    let output = child.wait_with_output().await.map_err(|_| "vncpasswd")?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err("vncpasswd");
    }
    tokio::fs::write(file, &output.stdout)
        .await
        .map_err(|_| "vncpasswd")
}

async fn start_xvnc(
    settings: &Settings,
    number: u16,
    open: &Open,
    dir: &Path,
) -> Result<Child, &'static str> {
    remove_display_files(number).await;
    let socket = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
    let child = Command::new(&settings.xvnc)
        .arg(format!(":{number}"))
        .args(["-rfbport", &(5900 + number).to_string()])
        .arg("-rfbauth")
        .arg(dir.join("passwd"))
        .args([
            "-SecurityTypes",
            "VncAuth",
            "-depth",
            "24",
            "-nolisten",
            "tcp",
        ])
        .args(["-desktop", "remotehub"])
        .args(["-geometry", &geometry(open)])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "xvnc")?;
    let deadline = tokio::time::Instant::now() + DISPLAY_TIMEOUT;
    while !socket.exists() {
        if tokio::time::Instant::now() >= deadline {
            return Err("xvnc");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(child)
}

/// Xvnc's lock file and socket: a killed Xvnc leaves them behind, and the
/// next Xvnc would refuse the display number.
async fn remove_display_files(number: u16) {
    let _ = tokio::fs::remove_file(format!("/tmp/.X{number}-lock")).await;
    let _ = tokio::fs::remove_file(format!("/tmp/.X11-unix/X{number}")).await;
}

fn geometry(open: &Open) -> String {
    format!(
        "{}x{}",
        open.width.clamp(640, 4096),
        open.height.clamp(480, 4096)
    )
}

fn start_chromium(
    settings: &Settings,
    number: u16,
    open: &Open,
    dir: &Path,
    proxy_port: u16,
) -> Result<Child, &'static str> {
    let size = geometry(open).replace('x', ",");
    let mut command = Command::new("sh");
    // `--remote-debugging-pipe` talks on descriptors 3 and 4: the shell puts
    // our pipes there before it becomes Chromium.
    command
        .args(["-c", r#"exec "$0" "$@" 3<&0 4>&1 0</dev/null 1>/dev/null"#])
        .arg(&settings.chromium)
        .arg(format!("--user-data-dir={}", dir.join("profile").display()))
        .args([
            // No address bar, and the developer tools stay closed: the only
            // thing keeping the user from reading the typed password
            // (ADR 0007).
            "--kiosk",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-dev-shm-usage",
            "--disable-gpu",
            "--password-store=basic",
            "--remote-debugging-pipe",
            "--window-position=0,0",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--disable-features=Translate,MediaRouter",
            "--noerrdialogs",
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
            "--proxy-bypass-list=<-loopback>",
        ])
        .arg(format!("--window-size={size}"))
        .arg(format!("--proxy-server=http://127.0.0.1:{proxy_port}"))
        .arg(format!(
            "--ignore-certificate-errors-spki-list={}",
            open.spki
        ))
        .arg(open.url())
        .env_clear()
        .env("DISPLAY", format!(":{number}"))
        .env("HOME", dir.join("home"))
        // Chromium's own temporary directories go with the session's.
        .env("TMPDIR", dir.join("home"))
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Its own process group, so the renderers end with it.
        .process_group(0)
        .kill_on_drop(true);
    if let Some(zone) = protocol::timezone(open.timezone.as_deref()) {
        command.env("TZ", zone);
    }
    command.spawn().map_err(|_| "chromium")
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Kills Chromium's process group: the browser process and every renderer,
/// GPU and zygote process it started.
struct KillGroupOnDrop(Option<u32>);

impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", "--", &format!("-{group}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_are_handed_out_once() {
        let displays = Displays::default();
        let range = 1..=2;
        let first = displays.take(&range).unwrap();
        let second = displays.take(&range).unwrap();
        assert_eq!((first.number, second.number), (1, 2));
        assert!(displays.take(&range).is_none());
        drop(first);
        assert_eq!(displays.take(&range).unwrap().number, 1);
    }

    #[test]
    fn vnc_passwords_differ() {
        let (a, b) = (vnc_password(), vnc_password());
        assert_eq!(a.len(), VNC_PASSWORD_LENGTH);
        assert_ne!(*a, *b);
    }
}
