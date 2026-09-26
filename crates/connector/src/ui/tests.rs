use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;
use crate::access::Gate;
use crate::journal::Journal;

const HOST: &str = "connector.test";

struct Setup {
    router: Router,
    site: Site,
    password: String,
    _dir: tempfile::TempDir,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let journal = Arc::new(Journal::new(dir.path()));
    let site = Site {
        gate: Gate::new(dir.path(), journal.clone()).unwrap(),
        journal,
        connections: Arc::default(),
    };
    let users = Users::new(dir.path());
    let password = users.add("carol", false).unwrap().password.to_string();
    let router = router(Ui::new(site.clone(), users, "remotehub.example.com".into()));
    Setup {
        router,
        site,
        password,
        _dir: dir,
    }
}

fn form(path: &str, body: &str, cookie: Option<&str>) -> Request {
    let mut request = Request::post(path)
        .header(header::HOST, HOST)
        .header(header::ORIGIN, format!("https://{HOST}"))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    request.body(Body::from(body.to_owned())).unwrap()
}

async fn send(router: &Router, request: Request) -> (StatusCode, HeaderMap, String) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

/// Signs in and returns the session cookie.
async fn sign_in(setup: &Setup) -> String {
    let body = format!("name=carol&password={}", setup.password);
    let (status, headers, _) = send(&setup.router, form("/sign-in", &body, None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("Secure") && cookie.contains("HttpOnly"));
    cookie.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn five_failed_sign_ins_lock_the_name() {
    let setup = setup();
    for _ in 0..ATTEMPTS {
        let (status, _, body) = send(
            &setup.router,
            form("/sign-in", "name=carol&password=x", None),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("Name, password or code is wrong."), "{body}");
    }
    // Even the right password waits now.
    let right = format!("name=carol&password={}", setup.password);
    let (status, _, body) = send(&setup.router, form("/sign-in", &right, None)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body.contains("Try again in 5 minutes."), "{body}");
    let events: Vec<Event> = setup
        .site
        .journal
        .recent(10)
        .into_iter()
        .map(|entry| entry.event)
        .collect();
    assert_eq!(
        events[0],
        Event::Locked {
            user: "carol".into()
        }
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::SignInFailed { .. }))
            .count(),
        5
    );
}

#[tokio::test]
async fn a_signed_in_user_opens_and_closes_access() {
    let setup = setup();
    let (status, headers, _) = send(
        &setup.router,
        Request::get("/").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/sign-in");

    let cookie = sign_in(&setup).await;
    let (status, _, _) = send(
        &setup.router,
        form("/access", "action=hours&hours=4", Some(&cookie)),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let stored = setup.site.gate.current();
    let until = stored.access.closes_at().unwrap();
    let minutes = (until - OffsetDateTime::now_utc()).whole_minutes();
    assert!((239..=240).contains(&minutes), "{minutes} minutes");
    assert_eq!(
        stored.changed_by,
        Some(Changer::Web {
            user: "carol".into()
        })
    );

    let (_, _, page) = send(
        &setup.router,
        Request::get("/")
            .header(header::COOKIE, &cookie)
            .header(header::ACCEPT_LANGUAGE, "de")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains("<html lang=\"de\">"));
    assert!(page.contains("Offen bis <time datetime="), "{page}");
    assert!(page.contains("carol hat den Zugang bis <time"), "{page}");

    // A time in the past opens nothing.
    let (status, _, _) = send(
        &setup.router,
        form(
            "/access",
            "action=until&until=2020-01-01T10:00&offset=0",
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, _) = send(
        &setup.router,
        form("/access", "action=close", Some(&cookie)),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(setup.site.gate.current().access, Access::Closed);
}

/// Another site's form cannot act for a signed-in user.
#[tokio::test]
async fn changes_come_only_from_the_own_origin() {
    let setup = setup();
    let cookie = sign_in(&setup).await;
    for (origin, fetch_site) in [
        (None, None),
        (Some("https://evil.example"), None),
        (Some("http://connector.test"), None),
        (Some("https://connector.test"), Some("cross-site")),
    ] {
        let mut request = Request::post("/access")
            .header(header::HOST, HOST)
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        if let Some(fetch_site) = fetch_site {
            request = request.header("sec-fetch-site", fetch_site);
        }
        let request = request.body(Body::from("action=permanent")).unwrap();
        let (status, headers, _) = send(&setup.router, request).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin:?} {fetch_site:?}");
        assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY));
        // With `no-referrer`, browsers send `Origin: null` on the page's own
        // forms, and every change would be refused like these.
        assert_eq!(headers[header::REFERRER_POLICY], "same-origin");
    }
    assert_eq!(setup.site.gate.current().access, Access::Closed);
}

/// What remotehub reports ends up in the page as text, never as markup.
#[tokio::test]
async fn reported_names_are_escaped() {
    let setup = setup();
    setup.site.journal.append(Event::ConnectionStarted {
        id: uuid::Uuid::nil(),
        target: "<script>x</script>:22".into(),
        user: Some("\"><img src=x>".into()),
    });
    let cookie = sign_in(&setup).await;
    let (_, _, page) = send(
        &setup.router,
        Request::get("/")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(!page.contains("<script>x"), "{page}");
    assert!(!page.contains("<img"), "{page}");
    assert!(page.contains("&lt;script&gt;x&lt;/script&gt;:22"), "{page}");
}

#[test]
fn a_local_time_takes_the_browsers_offset() {
    let at = local_time("2026-10-01T18:00", 120).unwrap();
    assert_eq!(at, time::macros::datetime!(2026-10-01 16:00 UTC));
    assert_eq!(
        local_time("2026-10-01T18:00:30", 0).unwrap(),
        time::macros::datetime!(2026-10-01 18:00:30 UTC)
    );
    assert!(local_time("tomorrow", 0).is_none());
}
