//! The connector's web interface (#165): where the customer's people sign
//! in, open and close access, and see running connections and the journal.
//!
//! Plain HTML forms, rendered here in the browser's language; a small script
//! formats times and sizes in the browser's locale and time zone and is the
//! only script the page runs. Served over HTTPS only (see [`serve`]).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Extension, Form, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Router, extract::Request};
use data_encoding::BASE64URL_NOPAD;
use remotehub_i18n::{self as i18n, Locale, Message};
use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};
use uuid::Uuid;

use crate::access::{self, Access, ChangeError, Changer, Scope, Snapshot, Stored};
use crate::agent::Site;
use crate::inventory::{self, Address, Device, InventoryError, Ports};
use crate::journal::{Entry, Event};
use crate::protocol::{Answer, RequestTarget};
use crate::users::Users;

/// Failed sign-ins in a row that lock a name, and for how long.
const ATTEMPTS: u32 = 5;
const LOCKED_FOR: Duration = Duration::from_secs(5 * 60);
/// A session ends after this long without a request, or this long after
/// signing in.
const IDLE: Duration = Duration::from_secs(30 * 60);
const LONGEST: Duration = Duration::from_secs(8 * 60 * 60);
/// `__Host-`: browsers take it only over HTTPS, for this host and path `/`.
const COOKIE: &str = "__Host-connector";
const LOG_ENTRIES: usize = 50;

/// The address a request came from, for the journal.
#[derive(Clone, Copy)]
pub struct Peer(pub SocketAddr);

pub struct Ui {
    site: Site,
    users: Users,
    /// remotehub's host, for the page header.
    remotehub: String,
    sessions: Mutex<HashMap<String, UiSession>>,
    attempts: Mutex<HashMap<String, Attempts>>,
    /// Each password check takes Argon2's 19 MiB: many sign-ins at once
    /// would take the host's memory, so they wait their turn.
    checking: tokio::sync::Semaphore,
}

struct UiSession {
    user: String,
    started: Instant,
    seen: Instant,
}

struct Attempts {
    failures: u32,
    last: Instant,
    locked_until: Option<Instant>,
}

/// Password checks at the same time.
const CHECKS: usize = 2;

impl Ui {
    pub fn new(site: Site, users: Users, remotehub: String) -> Arc<Ui> {
        Arc::new(Ui {
            site,
            users,
            remotehub,
            sessions: Mutex::default(),
            attempts: Mutex::default(),
            checking: tokio::sync::Semaphore::new(CHECKS),
        })
    }

    /// The signed-in user of a request, if its session is still valid.
    fn user(&self, headers: &HeaderMap) -> Option<String> {
        let token = cookie(headers)?;
        let mut sessions = self.sessions.lock().expect("no panics while locked");
        let now = Instant::now();
        sessions.retain(|_, s| now - s.seen < IDLE && now - s.started < LONGEST);
        let session = sessions.get_mut(&token)?;
        session.seen = now;
        Some(session.user.clone())
    }

    /// How long `name` stays locked, if it is.
    fn locked(&self, name: &str) -> Option<Duration> {
        let attempts = self.attempts.lock().expect("no panics while locked");
        let until = attempts.get(name)?.locked_until?;
        until.checked_duration_since(Instant::now())
    }

    /// Counts a failed sign-in; true if it locks the name.
    fn failed(&self, name: &str) -> bool {
        let mut attempts = self.attempts.lock().expect("no panics while locked");
        let now = Instant::now();
        // Names nobody tried for a while are forgotten, so made-up names
        // cannot fill the memory.
        attempts.retain(|_, a| {
            now - a.last < LOCKED_FOR || a.locked_until.is_some_and(|until| until > now)
        });
        let entry = attempts.entry(name.to_owned()).or_insert(Attempts {
            failures: 0,
            last: now,
            locked_until: None,
        });
        entry.last = now;
        entry.failures += 1;
        if entry.failures >= ATTEMPTS {
            entry.failures = 0;
            entry.locked_until = Some(Instant::now() + LOCKED_FOR);
            return true;
        }
        false
    }
}

pub fn router(ui: Arc<Ui>) -> Router {
    Router::new()
        .route("/", get(status))
        .route("/sign-in", get(sign_in_page).post(sign_in))
        .route("/sign-out", post(sign_out))
        .route("/access", post(change_access))
        .route("/devices", post(add_device))
        .route("/devices/remove", post(remove_device))
        .route("/groups", post(add_group))
        .route("/groups/remove", post(remove_group))
        .route("/requests/approve", post(approve_request))
        .route("/requests/refuse", post(refuse_request))
        .route("/app.css", get(stylesheet))
        .route("/app.js", get(script))
        .layer(middleware::from_fn(guard))
        .with_state(ui)
}

/// Every answer gets headers that keep it out of frames and caches and
/// allow only this origin's own script and style. A state-changing request
/// must come from a page of this origin: the cookie alone would let another
/// site's form act for a signed-in user.
async fn guard(request: Request, next: Next) -> Response {
    let refused = request.method() == Method::POST && !same_origin(request.headers());
    let mut response = if refused {
        StatusCode::FORBIDDEN.into_response()
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; \
             form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Not `no-referrer`: with it, browsers send `Origin: null` on the page's
    // own forms, and `same_origin` could not tell them from another site's.
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn same_origin(headers: &HeaderMap) -> bool {
    let site = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok());
    if site.is_some_and(|site| site != "same-origin") {
        return false;
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    matches!((host, origin), (Some(host), Some(origin)) if origin == format!("https://{host}"))
}

fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == COOKIE).then(|| value.to_owned())
        })
}

fn locale(headers: &HeaderMap) -> Locale {
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .map_or(Locale::En, Locale::from_accept_language)
}

async fn stylesheet() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../assets/app.css"),
    )
}

async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../assets/app.js"),
    )
}

// ── Rendering ────────────────────────────────────────────────────────────

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// A plain message, HTML-escaped.
fn t(locale: Locale, message: Message) -> String {
    escape(&i18n::render(locale, &message))
}

/// The marker for the `n`th element in a message: a character no
/// translation contains, which escaping leaves alone.
fn slot(n: usize) -> String {
    format!("\u{E000}{n}\u{E000}")
}

/// A message, HTML-escaped, with each [`slot`] replaced by its element.
fn rich(locale: Locale, message: Message, elements: &[String]) -> String {
    let mut text = t(locale, message);
    for (n, element) in elements.iter().enumerate() {
        text = text.replace(&slot(n), element);
    }
    text
}

/// A point in time the script shows in the browser's locale and zone; in
/// UTC without the script.
fn time_element(at: OffsetDateTime) -> String {
    let at = at.to_offset(UtcOffset::UTC);
    let machine = at
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let fallback = at
        .format(format_description!(
            "[year]-[month]-[day] [hour]:[minute] UTC"
        ))
        .unwrap_or_default();
    format!("<time datetime=\"{machine}\">{fallback}</time>")
}

/// A number of bytes the script shows with a unit.
fn bytes_element(bytes: u64) -> String {
    format!("<data value=\"{bytes}\" class=\"bytes\">{bytes} B</data>")
}

fn page(locale: Locale, body: &str) -> Html<String> {
    let lang = match locale {
        Locale::En => "en",
        Locale::De => "de",
    };
    let title = t(locale, Message::ConnectorUiTitle {});
    Html(format!(
        "<!doctype html>\n<html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><link rel=\"stylesheet\" href=\"/app.css\">\
         <script src=\"/app.js\" defer></script></head><body><main>{body}</main></body></html>"
    ))
}

const ICON_CLOSED: &str = "<svg class=\"icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><rect x=\"4\" y=\"11\" width=\"16\" height=\"10\" rx=\"2\"/><path d=\"M8 11V7a4 4 0 0 1 8 0v4\"/></svg>";
const ICON_WARNING: &str = "<svg class=\"icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M12 3 2 21h20L12 3z\"/><path d=\"M12 10v5M12 18v.5\"/></svg>";
const ICON_OPEN: &str = "<svg class=\"icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><rect x=\"4\" y=\"11\" width=\"16\" height=\"10\" rx=\"2\"/><path d=\"M8 11V7a4 4 0 0 1 7.9-1\"/></svg>";

// ── Signing in ───────────────────────────────────────────────────────────

async fn sign_in_page(headers: HeaderMap) -> Html<String> {
    sign_in_form(locale(&headers), None)
}

fn sign_in_form(locale: Locale, error: Option<String>) -> Html<String> {
    let error = error
        .map(|text| format!("<p class=\"error\" role=\"alert\">{text}</p>"))
        .unwrap_or_default();
    let body = format!(
        "<h1>{title}</h1><form class=\"card sign-in\" method=\"post\" action=\"/sign-in\">{error}\
         <label>{name}<input name=\"name\" autocomplete=\"username\" required autofocus></label>\
         <label>{password}<input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>\
         <label>{code}<input name=\"code\" inputmode=\"numeric\" autocomplete=\"one-time-code\"></label>\
         <p class=\"hint\">{hint}</p><button type=\"submit\">{submit}</button></form>",
        title = t(locale, Message::ConnectorUiTitle {}),
        name = t(locale, Message::ConnectorUiName {}),
        password = t(locale, Message::ConnectorUiPassword {}),
        code = t(locale, Message::ConnectorUiCode {}),
        hint = t(locale, Message::ConnectorUiCodeHint {}),
        submit = t(locale, Message::ConnectorUiSignIn {}),
    );
    page(locale, &body)
}

#[derive(Deserialize)]
struct SignIn {
    name: String,
    password: String,
    #[serde(default)]
    code: String,
}

async fn sign_in(
    State(ui): State<Arc<Ui>>,
    peer: Option<Extension<Peer>>,
    headers: HeaderMap,
    Form(form): Form<SignIn>,
) -> Response {
    let locale = locale(&headers);
    let address = peer.map_or_else(String::new, |Extension(Peer(a))| a.ip().to_string());
    let name = form.name.trim().to_owned();
    if let Some(left) = ui.locked(&name) {
        let minutes = i64::try_from(left.as_secs().div_ceil(60)).unwrap_or(i64::MAX);
        let text = t(locale, Message::ConnectorUiLocked { minutes });
        return (
            StatusCode::TOO_MANY_REQUESTS,
            sign_in_form(locale, Some(text)),
        )
            .into_response();
    }
    // Argon2 is slow on purpose; it runs beside the other requests.
    let checked = {
        let _turn = ui.checking.acquire().await.expect("never closed");
        let ui = ui.clone();
        let name = name.clone();
        tokio::task::spawn_blocking(move || ui.users.verify(&name, &form.password, &form.code))
            .await
            .expect("the check does not panic")
    };
    if checked.is_err() {
        ui.site.journal.append(Event::SignInFailed {
            user: name.clone(),
            address,
        });
        if ui.failed(&name) {
            ui.site.journal.append(Event::Locked { user: name });
        }
        let text = t(locale, Message::ConnectorUiSignInFailed {});
        return (StatusCode::UNAUTHORIZED, sign_in_form(locale, Some(text))).into_response();
    }
    ui.attempts
        .lock()
        .expect("no panics while locked")
        .remove(&name);
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).expect("the OS has randomness");
    let token = BASE64URL_NOPAD.encode(&random);
    let now = Instant::now();
    ui.sessions.lock().expect("no panics while locked").insert(
        token.clone(),
        UiSession {
            user: name.clone(),
            started: now,
            seen: now,
        },
    );
    ui.site.journal.append(Event::SignedIn {
        user: name,
        address,
    });
    let cookie = format!("{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict");
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

async fn sign_out(State(ui): State<Arc<Ui>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie(&headers) {
        ui.sessions
            .lock()
            .expect("no panics while locked")
            .remove(&token);
    }
    let expired = format!("{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0");
    ([(header::SET_COOKIE, expired)], Redirect::to("/sign-in")).into_response()
}

// ── Access ───────────────────────────────────────────────────────────────

async fn status(State(ui): State<Arc<Ui>>, headers: HeaderMap) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    status_page(&ui, locale(&headers), &user, None).into_response()
}

#[derive(Deserialize)]
struct Change {
    /// `hours`, `until`, `permanent` or `close`; `open` with `duration`.
    action: String,
    /// `network` (the default), `device` or `group`, named `name` (#180).
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    hours: Option<i64>,
    /// A number of hours, or `permanent`.
    #[serde(default)]
    duration: Option<String>,
    /// `datetime-local`: `2026-10-01T18:00`, in the browser's zone.
    #[serde(default)]
    until: Option<String>,
    /// The browser's offset from UTC at `until`, in minutes east; the script
    /// fills it in. UTC without it.
    #[serde(default)]
    offset: Option<i32>,
}

fn for_hours(hours: i64, now: OffsetDateTime) -> Option<Access> {
    (1..=24 * 7).contains(&hours).then(|| Access::Open {
        until: Some(now + time::Duration::hours(hours)),
    })
}

async fn change_access(
    State(ui): State<Arc<Ui>>,
    headers: HeaderMap,
    Form(form): Form<Change>,
) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    let locale = locale(&headers);
    let now = OffsetDateTime::now_utc();
    let access = match form.action.as_str() {
        "close" => Some(Access::Closed),
        "permanent" => Some(Access::Open { until: None }),
        "hours" => form.hours.and_then(|hours| for_hours(hours, now)),
        "open" => match form.duration.as_deref() {
            Some("permanent") => Some(Access::Open { until: None }),
            Some(hours) => hours.parse().ok().and_then(|hours| for_hours(hours, now)),
            None => None,
        },
        "until" => form
            .until
            .as_deref()
            .and_then(|until| local_time(until, form.offset.unwrap_or(0)))
            .filter(|until| *until > now)
            .map(|until| Access::Open { until: Some(until) }),
        _ => None,
    };
    let name = form.name.unwrap_or_default();
    let scope = match form.scope.as_deref() {
        None | Some("network") => Some(Scope::Network),
        Some("device") => Some(Scope::Device { name }),
        Some("group") => Some(Scope::Group { name }),
        // An approved request only closes; a longer time is a new request.
        Some("request") if access == Some(Access::Closed) => {
            name.parse().ok().map(|id| Scope::Request { id })
        }
        Some(_) => None,
    };
    let (Some(access), Some(scope)) = (access, scope) else {
        let error = t(locale, Message::ConnectorUiInvalidUntil {});
        return (
            StatusCode::BAD_REQUEST,
            status_page(&ui, locale, &user, Some(error)),
        )
            .into_response();
    };
    let changed = ui
        .site
        .gate
        .set(scope, access, Changer::Web { user: user.clone() });
    after_change(&ui, locale, &user, changed)
}

/// Back to the page after a change, or the page with why it failed.
fn after_change(ui: &Ui, locale: Locale, user: &str, changed: Result<(), ChangeError>) -> Response {
    match changed {
        Ok(()) => Redirect::to("/").into_response(),
        Err(ChangeError::Inventory(error)) => {
            let text = t(locale, inventory_message(&error));
            (
                StatusCode::BAD_REQUEST,
                status_page(ui, locale, user, Some(text)),
            )
                .into_response()
        }
        Err(ChangeError::Io(error)) => {
            tracing::error!(%error, "cannot store the change");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn inventory_message(error: &InventoryError) -> Message {
    match error {
        InventoryError::InvalidName => Message::ConnectorUiErrorName {},
        InventoryError::InvalidAddress(value) => Message::ConnectorUiErrorAddress {
            value: value.clone(),
        },
        InventoryError::InvalidPorts(value) => Message::ConnectorUiErrorPorts {
            value: value.clone(),
        },
        InventoryError::NoPorts => Message::ConnectorUiErrorNoPorts {},
        InventoryError::Exists(name) => Message::ConnectorUiErrorExists { name: name.clone() },
        InventoryError::Unknown(name) => Message::ConnectorUiErrorUnknown { name: name.clone() },
    }
}

/// Adds a device to the list, from a form with `name`, `address`, `ports`
/// and `group` once per checked group.
async fn add_device(
    State(ui): State<Arc<Ui>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    let locale = locale(&headers);
    // Checkboxes repeat a name; serde_urlencoded takes each pair.
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let field = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let groups = pairs
        .iter()
        .filter(|(k, _)| k == "group")
        .map(|(_, v)| v.clone())
        .collect();
    let device = (|| {
        Ok::<_, InventoryError>(Device {
            name: inventory::valid_name(&field("name"))?,
            address: field("address").parse()?,
            ports: Ports::parse_list(&field("ports"))?,
            groups,
        })
    })();
    let changed = device.map_err(ChangeError::from).and_then(|device| {
        let site = &ui.site;
        let result = access::add_device(
            site.gate.dir(),
            site.gate.journal(),
            device,
            Changer::Web { user: user.clone() },
        );
        site.gate.reload();
        result
    });
    after_change(&ui, locale, &user, changed)
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

async fn remove_device(
    State(ui): State<Arc<Ui>>,
    headers: HeaderMap,
    Form(form): Form<Named>,
) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    let site = &ui.site;
    let changed = access::remove_device(
        site.gate.dir(),
        site.gate.journal(),
        &form.name,
        Changer::Web { user: user.clone() },
    );
    site.gate.reload();
    after_change(&ui, locale(&headers), &user, changed)
}

async fn add_group(
    State(ui): State<Arc<Ui>>,
    headers: HeaderMap,
    Form(form): Form<Named>,
) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    let site = &ui.site;
    let changed = access::add_group(
        site.gate.dir(),
        site.gate.journal(),
        &form.name,
        Changer::Web { user: user.clone() },
    );
    site.gate.reload();
    after_change(&ui, locale(&headers), &user, changed)
}

async fn remove_group(
    State(ui): State<Arc<Ui>>,
    headers: HeaderMap,
    Form(form): Form<Named>,
) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    let site = &ui.site;
    let changed = access::remove_group(
        site.gate.dir(),
        site.gate.journal(),
        &form.name,
        Changer::Web { user: user.clone() },
    );
    site.gate.reload();
    after_change(&ui, locale(&headers), &user, changed)
}

#[derive(Deserialize)]
struct Answering {
    id: Uuid,
}

/// The page saying that a request no longer waits for an answer.
fn gone(ui: &Ui, locale: Locale, user: &str) -> Response {
    let text = t(locale, Message::ConnectorUiErrorRequestGone {});
    (
        StatusCode::CONFLICT,
        status_page(ui, locale, user, Some(text)),
    )
        .into_response()
}

/// Opens exactly the request's targets for the time asked (#181). Only here,
/// by a signed-in user: nothing remotehub sends opens anything.
async fn approve_request(
    State(ui): State<Arc<Ui>>,
    headers: HeaderMap,
    Form(form): Form<Answering>,
) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    let locale = locale(&headers);
    let Some(request) = ui.site.requests.get(form.id) else {
        return gone(&ui, locale, &user);
    };
    let now = OffsetDateTime::now_utc();
    let until = (now + time::Duration::minutes(i64::from(request.minutes)))
        .replace_nanosecond(0)
        .unwrap_or(now);
    let site = &ui.site;
    let approved = access::approve(
        site.gate.dir(),
        site.gate.journal(),
        &request,
        until,
        Changer::Web { user: user.clone() },
    );
    site.gate.reload();
    if approved.is_ok() {
        site.requests.answer(Answer {
            id: request.id,
            approved: true,
            by: user.clone(),
            until: until.format(&Rfc3339).ok(),
        });
    }
    after_change(&ui, locale, &user, approved)
}

async fn refuse_request(
    State(ui): State<Arc<Ui>>,
    headers: HeaderMap,
    Form(form): Form<Answering>,
) -> Response {
    let Some(user) = ui.user(&headers) else {
        return Redirect::to("/sign-in").into_response();
    };
    let locale = locale(&headers);
    let Some(request) = ui.site.requests.get(form.id) else {
        return gone(&ui, locale, &user);
    };
    access::refuse(
        &ui.site.journal,
        &request,
        Changer::Web { user: user.clone() },
    );
    ui.site.requests.answer(Answer {
        id: request.id,
        approved: false,
        by: user,
        until: None,
    });
    Redirect::to("/").into_response()
}

/// A `datetime-local` value at `offset` minutes east of UTC.
fn local_time(text: &str, offset: i32) -> Option<OffsetDateTime> {
    let minutes = format_description!("[year]-[month]-[day]T[hour]:[minute]");
    let seconds = format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");
    let local = PrimitiveDateTime::parse(text, minutes)
        .or_else(|_| PrimitiveDateTime::parse(text, seconds))
        .ok()?;
    let offset = UtcOffset::from_whole_seconds(offset.checked_mul(60)?).ok()?;
    Some(local.assume_offset(offset))
}

fn status_page(ui: &Ui, locale: Locale, user: &str, error: Option<String>) -> Html<String> {
    let snapshot = ui.site.gate.current();
    let stored = snapshot.access.network.clone();
    let now = OffsetDateTime::now_utc();
    let open = stored.access.is_open(now);
    let partly = !open && snapshot.any_open(now);
    let state = match stored.access {
        Access::Open { until: Some(until) } if open => rich(
            locale,
            Message::ConnectorUiOpenUntil { until: slot(0) },
            &[time_element(until)],
        ),
        Access::Open { until: None } => t(locale, Message::ConnectorUiOpenPermanent {}),
        _ if partly => t(locale, Message::ConnectorUiPartly {}),
        _ => t(locale, Message::ConnectorUiClosed {}),
    };
    let changed = match (&stored.changed_by, stored.changed_at) {
        (Some(by), Some(at)) => {
            let at_slot = slot(0);
            let message = match by {
                Changer::Web { user } => Message::ConnectorUiChangedBy {
                    who: user.clone(),
                    at: at_slot,
                },
                Changer::CommandLine => Message::ConnectorUiChangedCli { at: at_slot },
                Changer::Expiry => Message::ConnectorUiChangedExpiry { at: at_slot },
            };
            format!(
                "<p class=\"hint\">{}</p>",
                rich(locale, message, &[time_element(at)])
            )
        }
        _ => String::new(),
    };
    let (class, icon) = if open {
        ("state open", ICON_OPEN)
    } else if partly {
        ("state partly", ICON_OPEN)
    } else {
        ("state closed", ICON_CLOSED)
    };
    let error = error
        .map(|text| format!("<p class=\"error\" role=\"alert\">{text}</p>"))
        .unwrap_or_default();
    let hours: String = [1, 4, 8]
        .into_iter()
        .map(|hours| {
            format!(
                "<form method=\"post\" action=\"/access\"><input type=\"hidden\" name=\"action\" value=\"hours\">\
                 <input type=\"hidden\" name=\"hours\" value=\"{hours}\"><button type=\"submit\">{}</button></form>",
                t(locale, Message::ConnectorUiOpenFor { hours })
            )
        })
        .collect();
    let close = if open {
        format!(
            "<form method=\"post\" action=\"/access\" class=\"close\"><input type=\"hidden\" name=\"action\" value=\"close\">\
             <button type=\"submit\" class=\"danger\">{}</button><span class=\"hint\">{}</span></form>",
            t(locale, Message::ConnectorUiClose {}),
            t(locale, Message::ConnectorUiCloseHint {}),
        )
    } else {
        String::new()
    };
    let body = format!(
        "<header><div><h1>{title}</h1><p class=\"hint\">{access_for}</p></div>\
         <form method=\"post\" action=\"/sign-out\" class=\"who\"><span>{signed_in}</span>\
         <button type=\"submit\" class=\"quiet\">{sign_out}</button></form></header>\
         <section class=\"card\"><p class=\"{class}\">{icon}<span>{state}</span></p>{changed}{error}</section>\
         <section class=\"card\"><h2>{network}</h2><p class=\"hint\">{network_hint}</p><div class=\"actions\">{hours}</div>\
         <form method=\"post\" action=\"/access\" class=\"until\"><input type=\"hidden\" name=\"action\" value=\"until\">\
         <input type=\"hidden\" name=\"offset\" value=\"0\">\
         <label>{until_field}<input type=\"datetime-local\" name=\"until\" required></label>\
         <button type=\"submit\">{until_button}</button></form>\
         <form method=\"post\" action=\"/access\" class=\"permanent\"><input type=\"hidden\" name=\"action\" value=\"permanent\">\
         <button type=\"submit\">{permanent}</button><span class=\"hint\">{permanent_hint}</span></form>{close}</section>\
         {requests}{approved}{groups}{devices}{connections}{log}",
        title = t(locale, Message::ConnectorUiTitle {}),
        access_for = t(
            locale,
            Message::ConnectorUiAccessFor {
                remotehub: ui.remotehub.clone()
            }
        ),
        signed_in = t(
            locale,
            Message::ConnectorUiSignedInAs {
                name: user.to_owned()
            }
        ),
        sign_out = t(locale, Message::ConnectorUiSignOut {}),
        network = t(locale, Message::ConnectorUiNetwork {}),
        network_hint = t(locale, Message::ConnectorUiNetworkHint {}),
        requests = requests(ui, &snapshot, locale),
        approved = approved(&snapshot, locale),
        groups = groups(&snapshot, locale),
        devices = devices(&snapshot, locale),
        until_field = t(locale, Message::ConnectorUiUntilField {}),
        until_button = t(locale, Message::ConnectorUiOpenUntilButton {}),
        permanent = t(locale, Message::ConnectorUiOpenPermanentButton {}),
        permanent_hint = t(locale, Message::ConnectorUiOpenPermanentHint {}),
        connections = connections(ui, locale),
        log = log(ui, locale),
    );
    page(locale, &body)
}

/// A row's access: its own, or for a device through an open group.
fn row_state(stored: &Stored, via: Option<&str>, locale: Locale) -> (bool, String) {
    let now = OffsetDateTime::now_utc();
    match stored.access {
        Access::Open { until: Some(until) } if until > now => (
            true,
            rich(
                locale,
                Message::ConnectorUiRowOpenUntil { until: slot(0) },
                &[time_element(until)],
            ),
        ),
        Access::Open { until: None } => (true, t(locale, Message::ConnectorUiRowOpen {})),
        _ => match via {
            Some(group) => (
                true,
                t(
                    locale,
                    Message::ConnectorUiRowOpenVia {
                        group: group.to_owned(),
                    },
                ),
            ),
            None => (false, t(locale, Message::ConnectorUiRowClosed {})),
        },
    }
}

/// The state with its icon, and the forms to open, close and remove a
/// device or group.
fn row_cells(
    kind: &str,
    name: &str,
    open: bool,
    own_open: bool,
    state: &str,
    locale: Locale,
) -> String {
    let name = escape(name);
    let icon = if open { ICON_OPEN } else { ICON_CLOSED };
    let class = if open { "open" } else { "closed" };
    let durations: String = [1, 4, 8]
        .into_iter()
        .map(|hours| {
            format!(
                "<option value=\"{hours}\">{}</option>",
                t(locale, Message::ConnectorUiHours { hours })
            )
        })
        .collect();
    let scope = format!(
        "<input type=\"hidden\" name=\"scope\" value=\"{kind}\"><input type=\"hidden\" name=\"name\" value=\"{name}\">"
    );
    let close = if own_open {
        format!(
            "<form method=\"post\" action=\"/access\">{scope}<input type=\"hidden\" name=\"action\" value=\"close\">\
             <button type=\"submit\" class=\"danger\">{}</button></form>",
            t(locale, Message::ConnectorUiCloseRow {})
        )
    } else {
        String::new()
    };
    format!(
        "<td><span class=\"row-state {class}\">{icon}<span>{state}</span></span></td><td><div class=\"row-actions\">\
         <form method=\"post\" action=\"/access\">{scope}<input type=\"hidden\" name=\"action\" value=\"open\">\
         <select name=\"duration\" aria-label=\"{open_label}\">{durations}<option value=\"permanent\">{without_end}</option></select>\
         <button type=\"submit\">{open_label}</button></form>{close}\
         <form method=\"post\" action=\"/{kind}s/remove\"><input type=\"hidden\" name=\"name\" value=\"{name}\">\
         <button type=\"submit\" class=\"quiet\">{remove}</button></form></div></td>",
        open_label = t(locale, Message::ConnectorUiOpen {}),
        without_end = t(locale, Message::ConnectorUiWithoutEnd {}),
        remove = t(locale, Message::ConnectorUiRemove {}),
    )
}

/// The device of the customer's list that covers `target`, if one does,
/// without resolving host names: only an equal name or an address in a
/// range counts.
fn listed_as<'a>(snapshot: &'a Snapshot, target: &RequestTarget) -> Option<&'a str> {
    let ip = target.host.parse::<IpAddr>().ok();
    snapshot
        .inventory
        .devices
        .iter()
        .find(|device| {
            device.ports.iter().any(|ports| ports.contains(target.port))
                && match (&device.address, ip) {
                    (Address::Host(name), _) => name.eq_ignore_ascii_case(&target.host),
                    (Address::Range(network), Some(ip)) => network.contains(ip),
                    (Address::Range(_), None) => false,
                }
        })
        .map(|device| device.name.as_str())
}

/// A request's targets: remotehub's name, and the address and port an
/// approval opens, marked where the customer's list does not know them.
fn targets_table(snapshot: &Snapshot, targets: &[RequestTarget], locale: Locale) -> String {
    let rows: String = targets
        .iter()
        .map(|target| {
            let listed = match listed_as(snapshot, target) {
                Some(name) => format!(
                    "<span class=\"row-state open\">{}</span>",
                    t(
                        locale,
                        Message::ConnectorUiListedAs {
                            name: name.to_owned()
                        }
                    )
                ),
                None => format!(
                    "<span class=\"row-state unlisted\">{ICON_WARNING}<span>{}</span></span>",
                    t(locale, Message::ConnectorUiNotListed {})
                ),
            };
            format!(
                "<tr><td>{}</td><td><code>{}</code></td><td>{}</td><td>{listed}</td></tr>",
                escape(&target.name),
                escape(&target.host),
                target.port,
            )
        })
        .collect();
    format!(
        "<table><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>{rows}</tbody></table>",
        t(locale, Message::ConnectorUiColTarget {}),
        t(locale, Message::ConnectorUiColAddress {}),
        t(locale, Message::ConnectorUiColPort {}),
        t(locale, Message::ConnectorUiColListed {}),
    )
}

/// remotehub's requests waiting for an answer (#181); nothing while there
/// are none.
fn requests(ui: &Ui, snapshot: &Snapshot, locale: Locale) -> String {
    let waiting = ui.site.requests.waiting();
    if waiting.is_empty() {
        return String::new();
    }
    let items: String = waiting
        .iter()
        .map(|request| {
            let id = request.id;
            format!(
                "<article class=\"request\"><p><strong>{asks}</strong></p><p class=\"reason\">{reason}</p>{targets}\
                 <div class=\"actions\">\
                 <form method=\"post\" action=\"/requests/approve\"><input type=\"hidden\" name=\"id\" value=\"{id}\">\
                 <button type=\"submit\">{approve}</button></form>\
                 <form method=\"post\" action=\"/requests/refuse\"><input type=\"hidden\" name=\"id\" value=\"{id}\">\
                 <button type=\"submit\" class=\"danger\">{refuse}</button></form></div></article>",
                asks = t(
                    locale,
                    Message::ConnectorUiRequestAsks {
                        requester: request.requester.clone(),
                        minutes: i64::from(request.minutes),
                    }
                ),
                reason = t(
                    locale,
                    Message::ConnectorUiRequestReason {
                        reason: request.reason.clone()
                    }
                ),
                targets = targets_table(snapshot, &request.targets, locale),
                approve = t(locale, Message::ConnectorUiApprove {}),
                refuse = t(locale, Message::ConnectorUiRefuse {}),
            )
        })
        .collect();
    format!(
        "<section class=\"card requests\"><h2>{}</h2><p class=\"hint\">{}</p>{items}</section>",
        t(locale, Message::ConnectorUiRequests {}),
        t(locale, Message::ConnectorUiRequestsHint {}),
    )
}

/// The approved requests still open, each with its end and a way to close
/// it early.
fn approved(snapshot: &Snapshot, locale: Locale) -> String {
    let open = snapshot.open_requests(OffsetDateTime::now_utc());
    if open.is_empty() {
        return String::new();
    }
    let items: String = open
        .iter()
        .map(|(id, grant)| {
            let (_, state) = row_state(&grant.stored, None, locale);
            format!(
                "<article class=\"request\"><p><strong>{who}</strong></p>\
                 <p><span class=\"row-state open\">{ICON_OPEN}<span>{state}</span></span></p>{targets}\
                 <form method=\"post\" action=\"/access\"><input type=\"hidden\" name=\"scope\" value=\"request\">\
                 <input type=\"hidden\" name=\"name\" value=\"{id}\"><input type=\"hidden\" name=\"action\" value=\"close\">\
                 <button type=\"submit\" class=\"danger\">{close}</button></form></article>",
                who = t(
                    locale,
                    Message::ConnectorUiApprovedFor {
                        requester: grant.requester.clone(),
                        reason: grant.reason.clone(),
                    }
                ),
                targets = targets_table(snapshot, &grant.targets, locale),
                close = t(locale, Message::ConnectorUiCloseRow {}),
            )
        })
        .collect();
    format!(
        "<section class=\"card\"><h2>{}</h2>{items}</section>",
        t(locale, Message::ConnectorUiApproved {}),
    )
}

fn groups(snapshot: &Snapshot, locale: Locale) -> String {
    let list = &snapshot.inventory;
    let rows = if list.groups.is_empty() {
        format!(
            "<p class=\"hint\">{}</p>",
            t(locale, Message::ConnectorUiNoGroups {})
        )
    } else {
        let rows: String = list
            .groups
            .iter()
            .map(|group| {
                let stored = snapshot.access.get(&Scope::Group {
                    name: group.clone(),
                });
                let (open, state) = row_state(&stored, None, locale);
                format!(
                    "<tr><td>{}</td><td>{}</td>{}</tr>",
                    escape(group),
                    list.members(group).count(),
                    row_cells("group", group, open, open, &state, locale)
                )
            })
            .collect();
        format!(
            "<table><thead><tr><th>{}</th><th>{}</th><th>{}</th><th></th></tr></thead><tbody>{rows}</tbody></table>",
            t(locale, Message::ConnectorUiColName {}),
            t(locale, Message::ConnectorUiColMembers {}),
            t(locale, Message::ConnectorUiColAccess {}),
        )
    };
    format!(
        "<section class=\"card\"><h2>{title}</h2>{rows}\
         <form method=\"post\" action=\"/groups\" class=\"add\">\
         <label>{name}<input name=\"name\" required maxlength=\"64\"></label>\
         <button type=\"submit\">{add}</button></form></section>",
        title = t(locale, Message::ConnectorUiGroups {}),
        name = t(locale, Message::ConnectorUiColName {}),
        add = t(locale, Message::ConnectorUiAddGroup {}),
    )
}

fn devices(snapshot: &Snapshot, locale: Locale) -> String {
    let list = &snapshot.inventory;
    let now = OffsetDateTime::now_utc();
    let rows = if list.devices.is_empty() {
        format!(
            "<p class=\"hint\">{}</p>",
            t(locale, Message::ConnectorUiNoDevices {})
        )
    } else {
        let rows: String = list
            .devices
            .iter()
            .map(|device| {
                let stored = snapshot.access.get(&Scope::Device {
                    name: device.name.clone(),
                });
                let via = device.groups.iter().find(|group| {
                    snapshot
                        .access
                        .get(&Scope::Group {
                            name: (*group).clone(),
                        })
                        .access
                        .is_open(now)
                });
                let own_open = stored.access.is_open(now);
                let (open, state) = row_state(&stored, via.map(String::as_str), locale);
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>{}</tr>",
                    escape(&device.name),
                    escape(&device.address.to_string()),
                    escape(&inventory::ports_text(&device.ports)),
                    escape(&device.groups.join(", ")),
                    row_cells("device", &device.name, open, own_open, &state, locale)
                )
            })
            .collect();
        format!(
            "<table><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th></th></tr></thead>\
             <tbody>{rows}</tbody></table>",
            t(locale, Message::ConnectorUiColName {}),
            t(locale, Message::ConnectorUiColAddress {}),
            t(locale, Message::ConnectorUiColPorts {}),
            t(locale, Message::ConnectorUiColGroups {}),
            t(locale, Message::ConnectorUiColAccess {}),
        )
    };
    let group_boxes: String = list
        .groups
        .iter()
        .map(|group| {
            let group = escape(group);
            format!("<label class=\"check\"><input type=\"checkbox\" name=\"group\" value=\"{group}\">{group}</label>")
        })
        .collect();
    format!(
        "<section class=\"card\"><h2>{title}</h2>{rows}\
         <form method=\"post\" action=\"/devices\" class=\"add\">\
         <label>{name}<input name=\"name\" required maxlength=\"64\"></label>\
         <label>{address}<input name=\"address\" required maxlength=\"253\" placeholder=\"10.0.0.5\"></label>\
         <label>{ports}<input name=\"ports\" required maxlength=\"200\" placeholder=\"22, 3389\"></label>\
         {group_boxes}<button type=\"submit\">{add}</button></form>\
         <p class=\"hint\">{address_hint} {ports_hint}</p></section>",
        title = t(locale, Message::ConnectorUiDevices {}),
        name = t(locale, Message::ConnectorUiColName {}),
        address = t(locale, Message::ConnectorUiColAddress {}),
        ports = t(locale, Message::ConnectorUiColPorts {}),
        add = t(locale, Message::ConnectorUiAddDevice {}),
        address_hint = t(locale, Message::ConnectorUiAddressHint {}),
        ports_hint = t(locale, Message::ConnectorUiPortsHint {}),
    )
}

fn connections(ui: &Ui, locale: Locale) -> String {
    let running = ui.site.connections.list();
    let rows = if running.is_empty() {
        format!(
            "<p class=\"hint\">{}</p>",
            t(locale, Message::ConnectorUiNoConnections {})
        )
    } else {
        let rows: String = running
            .iter()
            .map(|r| {
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    escape(&labelled(r.device.as_deref(), &r.target)),
                    escape(r.user.as_deref().unwrap_or("remotehub")),
                    time_element(r.since),
                    rich(
                        locale,
                        Message::ConnectorUiTraffic {
                            sent: slot(0),
                            received: slot(1)
                        },
                        &[
                            bytes_element(r.sent.load(Ordering::Relaxed)),
                            bytes_element(r.received.load(Ordering::Relaxed)),
                        ]
                    ),
                )
            })
            .collect();
        format!(
            "<table><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>{rows}</tbody></table>\
             <p class=\"hint\">{}</p>",
            t(locale, Message::ConnectorUiColDevice {}),
            t(locale, Message::ConnectorUiColUser {}),
            t(locale, Message::ConnectorUiColSince {}),
            t(locale, Message::ConnectorUiColTraffic {}),
            t(locale, Message::ConnectorUiUserHint {}),
        )
    };
    format!(
        "<section class=\"card\"><h2>{}</h2>{rows}</section>",
        t(locale, Message::ConnectorUiConnections {})
    )
}

fn log(ui: &Ui, locale: Locale) -> String {
    let entries = ui.site.journal.recent(LOG_ENTRIES);
    let rows = if entries.is_empty() {
        format!(
            "<p class=\"hint\">{}</p>",
            t(locale, Message::ConnectorUiNoLog {})
        )
    } else {
        let rows: String = entries
            .iter()
            .map(|entry| {
                format!(
                    "<tr><td>{}</td><td>{}</td></tr>",
                    time_element(entry.at),
                    describe(locale, entry)
                )
            })
            .collect();
        format!(
            "<table><thead><tr><th>{}</th><th>{}</th></tr></thead><tbody>{rows}</tbody></table>",
            t(locale, Message::ConnectorUiColTime {}),
            t(locale, Message::ConnectorUiColEvent {}),
        )
    };
    format!(
        "<section class=\"card\"><h2>{}</h2>{rows}</section>",
        t(locale, Message::ConnectorUiLog {})
    )
}

/// A connection's target for people (#177): the device's name in remotehub
/// first, the address it went to behind it, as the name alone could be
/// anything.
fn labelled(device: Option<&str>, target: &str) -> String {
    match device.filter(|name| !name.trim().is_empty()) {
        Some(name) => format!("{name} ({target})"),
        None => target.to_owned(),
    }
}

/// What a journal entry opened or closed, as a phrase in a sentence.
fn what(locale: Locale, scope: &Scope) -> String {
    i18n::render(
        locale,
        &match scope {
            Scope::Network => Message::ConnectorLogWhatNetwork {},
            Scope::Device { name } => Message::ConnectorLogWhatDevice { name: name.clone() },
            Scope::Group { name } => Message::ConnectorLogWhatGroup { name: name.clone() },
            Scope::Request { id } => Message::ConnectorLogWhatRequest { id: short(*id) },
        },
    )
}

/// A request's id as the journal shows it: long enough to tell requests
/// apart, short enough to read out on the phone.
fn short(id: Uuid) -> String {
    id.simple().to_string()[..8].to_owned()
}

/// Who changed the list, as the subject of a sentence.
fn who(locale: Locale, by: &Changer) -> String {
    match by {
        Changer::Web { user } => user.clone(),
        _ => i18n::render(locale, &Message::ConnectorLogCommandLine {}),
    }
}

/// A journal entry in words, as HTML.
fn describe(locale: Locale, entry: &Entry) -> String {
    let someone = |user: &Option<String>| user.clone().unwrap_or_else(|| "remotehub".to_owned());
    match &entry.event {
        Event::Opened { by, until, scope } => {
            let until_element = until.map(time_element);
            let what = what(locale, scope);
            let message = match (by, until) {
                (Changer::Web { user }, Some(_)) => Message::ConnectorLogOpenedUntil {
                    who: user.clone(),
                    what,
                    until: slot(0),
                },
                (Changer::Web { user }, None) => Message::ConnectorLogOpenedPermanent {
                    who: user.clone(),
                    what,
                },
                (_, Some(_)) => Message::ConnectorLogOpenedUntilCli {
                    what,
                    until: slot(0),
                },
                (_, None) => Message::ConnectorLogOpenedPermanentCli { what },
            };
            rich(locale, message, until_element.as_slice())
        }
        Event::Closed {
            by: Changer::Web { user },
            scope,
        } => t(
            locale,
            Message::ConnectorLogClosed {
                who: user.clone(),
                what: what(locale, scope),
            },
        ),
        Event::Closed { scope, .. } => t(
            locale,
            Message::ConnectorLogClosedCli {
                what: what(locale, scope),
            },
        ),
        Event::Expired { scope } => t(
            locale,
            Message::ConnectorLogExpired {
                what: what(locale, scope),
            },
        ),
        Event::DeviceAdded {
            by,
            name,
            address,
            ports,
            ..
        } => t(
            locale,
            Message::ConnectorLogDeviceAdded {
                who: who(locale, by),
                name: name.clone(),
                address: address.clone(),
                ports: ports.clone(),
            },
        ),
        Event::DeviceRemoved { by, name } => t(
            locale,
            Message::ConnectorLogDeviceRemoved {
                who: who(locale, by),
                name: name.clone(),
            },
        ),
        Event::GroupAdded { by, name } => t(
            locale,
            Message::ConnectorLogGroupAdded {
                who: who(locale, by),
                name: name.clone(),
            },
        ),
        Event::GroupRemoved { by, name } => t(
            locale,
            Message::ConnectorLogGroupRemoved {
                who: who(locale, by),
                name: name.clone(),
            },
        ),
        Event::RequestReceived {
            id,
            requester,
            reason,
            minutes,
            targets,
        } => t(
            locale,
            Message::ConnectorLogRequestReceived {
                id: short(*id),
                requester: requester.clone(),
                targets: targets.join(", "),
                minutes: i64::from(*minutes),
                reason: reason.clone(),
            },
        ),
        Event::RequestApproved {
            by,
            id,
            requester,
            targets,
            until,
            ..
        } => rich(
            locale,
            Message::ConnectorLogRequestApproved {
                who: who(locale, by),
                id: short(*id),
                requester: requester.clone(),
                targets: targets.join(", "),
                until: slot(0),
            },
            &[time_element(*until)],
        ),
        Event::RequestRefused {
            by,
            id,
            requester,
            targets,
            ..
        } => t(
            locale,
            Message::ConnectorLogRequestRefused {
                who: who(locale, by),
                id: short(*id),
                requester: requester.clone(),
                targets: targets.join(", "),
            },
        ),
        Event::ConnectionStarted {
            target,
            device,
            user,
            ..
        } => t(
            locale,
            Message::ConnectorLogConnectionStarted {
                user: someone(user),
                target: labelled(device.as_deref(), target),
            },
        ),
        Event::ConnectionEnded {
            target,
            device,
            user,
            seconds,
            sent,
            received,
            ..
        } => rich(
            locale,
            Message::ConnectorLogConnectionEnded {
                user: someone(user),
                target: labelled(device.as_deref(), target),
                minutes: i64::try_from(seconds.div_ceil(60)).unwrap_or(i64::MAX),
                sent: slot(0),
                received: slot(1),
            },
            &[bytes_element(*sent), bytes_element(*received)],
        ),
        Event::ConnectionRefused {
            target,
            device,
            user,
            reason,
        } => t(
            locale,
            Message::ConnectorLogConnectionRefused {
                user: someone(user),
                target: labelled(device.as_deref(), target),
                reason: reason.clone(),
            },
        ),
        Event::SignedIn { user, address } => t(
            locale,
            Message::ConnectorLogSignedIn {
                user: user.clone(),
                address: address.clone(),
            },
        ),
        Event::SignInFailed { user, address } => t(
            locale,
            Message::ConnectorLogSignInFailed {
                user: user.clone(),
                address: address.clone(),
            },
        ),
        Event::Locked { user } => t(
            locale,
            Message::ConnectorLogLocked {
                user: user.clone(),
                minutes: i64::try_from(LOCKED_FOR.as_secs() / 60).unwrap_or(i64::MAX),
            },
        ),
    }
}

#[cfg(test)]
mod tests;
