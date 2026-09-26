//! Asking the customer for access through a site connector (#181).

use axum::Router;
use remotehub_connector::access::{self, Access, Changer};
use remotehub_connector::journal::Event;
use remotehub_connector::protocol::{self, Answer};
use remotehub_server::connectors::STREAM_TIMEOUT;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{authed, get, send, sign_in_request};
use crate::connectors::{echo, new_connector, start_connector, wait_for};
use crate::terminal::{create, serve, setup};

/// A device behind connector `connector` at `target` (`host:port`).
async fn device(
    app: &Router,
    token: &str,
    folder: &str,
    name: &str,
    target: &str,
    connector: Uuid,
) -> String {
    let (host, port) = target.rsplit_once(':').unwrap();
    create(
        app,
        token,
        "/api/devices",
        json!({ "folder_id": folder, "name": name, "protocol": "ssh", "host": host,
                "port": port.parse::<u16>().unwrap(), "auth_mode": "ask", "credential_id": null,
                "connector_mode": "connector", "connector_id": connector }),
    )
    .await
}

fn ask(kind: &str, id: &str, minutes: i64, reason: &str) -> Value {
    json!({ "object": { "kind": kind, "id": id }, "minutes": minutes, "reason": reason })
}

async fn requests(app: &Router, token: &str) -> Value {
    send(app, get("/api/connector-requests", Some(token)))
        .await
        .json()
}

/// Asked in remotehub, shown at the connector, approved there: exactly the
/// device opens, and both sides record request and answer.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_customer_approves_a_request_at_the_connector(pool: PgPool) {
    let (state, app, token, folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let address = serve(state.clone()).await;
    let router = echo().await;
    let other = echo().await;
    let agent = start_connector(address, &secret, "", Access::Closed);
    let router_id = device(&app, &token, &folder, "router", &router, id).await;

    let asked = send(
        &app,
        authed(
            "POST",
            "/api/connector-requests",
            Some(ask("device", &router_id, 60, "ERP update")),
            &token,
        ),
    )
    .await;
    assert_eq!(asked.status, 201, "{}", asked.json());
    let again = send(
        &app,
        authed(
            "POST",
            "/api/connector-requests",
            Some(ask("device", &router_id, 60, "ERP update")),
            &token,
        ),
    )
    .await;
    assert_eq!(again.json()["code"], "request_pending");

    // The connector shows it; nothing opens by itself.
    wait_for(
        || agent.site.requests.waiting().len() == 1,
        "the request at the connector",
    )
    .await;
    let request = agent.site.requests.waiting().remove(0);
    let (host, port) = router.rsplit_once(':').unwrap();
    assert_eq!(request.requester, "alice");
    assert_eq!(request.minutes, 60);
    assert_eq!(request.targets.len(), 1);
    assert_eq!(
        (
            request.targets[0].host.as_str(),
            request.targets[0].port.to_string()
        ),
        (host, port.to_owned())
    );
    assert!(
        agent
            .site
            .journal
            .recent(5)
            .iter()
            .any(|e| matches!(e.event, Event::RequestReceived { .. }))
    );
    assert!(!state.connectors.is_online(id));
    assert_eq!(requests(&app, &token).await[0]["status"], "pending");

    // A user of the connector approves, as its web interface does.
    let until = (time::OffsetDateTime::now_utc() + time::Duration::minutes(60))
        .replace_nanosecond(0)
        .unwrap();
    access::approve(
        agent.dir.path(),
        &agent.site.journal,
        &request,
        until,
        Changer::Web {
            user: "carol".into(),
        },
    )
    .unwrap();
    agent.site.gate.reload();
    agent.site.requests.answer(Answer {
        id: request.id,
        approved: true,
        by: "carol".into(),
        until: until
            .format(&time::format_description::well_known::Rfc3339)
            .ok(),
    });
    wait_for(
        || state.connectors.is_online(id),
        "the connector to come online",
    )
    .await;
    for _ in 0..50 {
        if requests(&app, &token).await[0]["status"] == "approved" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let listed = requests(&app, &token).await;
    assert_eq!(listed[0]["status"], "approved", "{listed}");
    assert_eq!(listed[0]["answered_by"], "carol");
    assert!(listed[0]["until"].is_string());
    // Answered, it leaves the connector's list.
    wait_for(
        || agent.site.requests.answers().is_empty(),
        "remotehub to take the answer",
    )
    .await;

    assert!(
        state
            .connectors
            .stream(id, &router, None, STREAM_TIMEOUT)
            .await
            .is_ok()
    );
    assert!(
        state
            .connectors
            .stream(id, &other, None, STREAM_TIMEOUT)
            .await
            .is_err()
    );

    let audit = send(&app, get("/api/audit", Some(&token))).await.json();
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(
        actions.contains(&"connector.access_requested"),
        "{actions:?}"
    );
    assert!(
        actions.contains(&"connector.access_approved"),
        "{actions:?}"
    );
}

/// Only who may connect asks, for what the connector reaches, with a reason
/// that reads as what it is; a folder asks for its devices behind its
/// connector.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_request_is_checked_before_it_reaches_the_customer(pool: PgPool) {
    let (_state, app, token, folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let router = device(&app, &token, &folder, "router", "10.0.0.1:22", id).await;
    let direct = create(
        &app,
        &token,
        "/api/devices",
        json!({ "folder_id": folder, "name": "direct", "protocol": "ssh", "host": "10.0.0.2",
                "port": 22, "auth_mode": "ask", "credential_id": null }),
    )
    .await;
    let post = |body: Value, token: String| {
        let app = app.clone();
        async move {
            send(
                &app,
                authed("POST", "/api/connector-requests", Some(body), &token),
            )
            .await
        }
    };
    for (body, field) in [
        (ask("device", &router, 5, "ERP"), "minutes"),
        (ask("device", &router, 2000, "ERP"), "minutes"),
        (ask("device", &router, 60, "  "), "reason"),
        (ask("device", &router, 60, "ERP\u{202E}gnp.exe"), "reason"),
        (ask("device", &router, 60, &"x".repeat(501)), "reason"),
        (ask("device", &direct, 60, "ERP"), "object"),
    ] {
        let refused = post(body.clone(), token.clone()).await;
        assert_eq!(refused.status, 400, "{body}");
        assert_eq!(refused.json()["params"]["field"], field, "{body}");
    }

    // bob sees none of it.
    let bob = send(&app, sign_in_request("bob", "right"))
        .await
        .session_token()
        .unwrap();
    let hidden = post(ask("device", &router, 60, "ERP"), bob.clone()).await;
    assert_eq!(hidden.status, 404);

    // The folder asks for what is behind its connector: the router, not the
    // direct device.
    let patched = send(
        &app,
        authed(
            "PATCH",
            &format!("/api/folders/{folder}"),
            Some(json!({ "connector_id": id })),
            &token,
        ),
    )
    .await;
    assert_eq!(patched.status, 204);
    let asked = post(ask("folder", &folder, 120, "ERP update"), token.clone()).await;
    assert_eq!(asked.status, 201, "{}", asked.json());
    let listed = requests(&app, &token).await;
    let names: Vec<&str> = listed[0]["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    // `direct` names no connector of its own and so takes the folder's now.
    assert_eq!(names, ["direct", "router"]);

    // Only its requester withdraws it; then the connector no longer sees it.
    let request = listed[0]["id"].as_str().unwrap().to_owned();
    let path = format!("/api/connector-requests/{request}");
    assert_eq!(
        send(&app, authed("DELETE", &path, None, &bob)).await.status,
        404
    );
    assert_eq!(
        send(&app, authed("DELETE", &path, None, &token))
            .await
            .status,
        204
    );
    assert_eq!(requests(&app, &token).await[0]["status"], "cancelled");
    let answered = send(&app, report(&secret, json!([]))).await;
    assert_eq!(answered.json()["requests"], json!([]));
}

/// A state report of the connector with `secret`, carrying `answers`.
fn report(secret: &str, answers: Value) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::post("/api/connectors/state")
        .header("authorization", format!("Bearer {secret}"))
        .header(protocol::HEADER, protocol::VERSION)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "open": false, "until": null, "answers": answers }).to_string(),
        ))
        .unwrap()
}

/// The connector's answers are bounded, and count only for its own pending
/// requests.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn answers_count_only_for_the_connectors_own_requests(pool: PgPool) {
    let (_state, app, token, folder) = setup(pool).await;
    let (id, secret) = new_connector(&app, &token, "lab").await;
    let (_, other_secret) = new_connector(&app, &token, "other").await;
    let router = device(&app, &token, &folder, "router", "10.0.0.1:22", id).await;
    let asked = send(
        &app,
        authed(
            "POST",
            "/api/connector-requests",
            Some(ask("device", &router, 60, "ERP")),
            &token,
        ),
    )
    .await;
    let request = asked.json()["id"].as_str().unwrap().to_owned();
    let approval = |by: &str, until: Value| json!([{ "id": request, "approved": true, "by": by, "until": until }]);

    // The pending request comes back as data.
    let answered = send(&app, report(&secret, json!([]))).await;
    assert_eq!(answered.status, 200);
    assert_eq!(answered.json()["requests"][0]["id"], request.as_str());
    assert_eq!(answered.json()["requests"][0]["requester"], "alice");
    // Another connector neither sees nor answers it.
    let foreign = send(
        &app,
        report(
            &other_secret,
            approval("mallory", json!("2099-01-01T00:00:00Z")),
        ),
    )
    .await;
    assert_eq!(foreign.status, 200);
    assert_eq!(foreign.json()["requests"], json!([]));
    assert_eq!(requests(&app, &token).await[0]["status"], "pending");

    for answers in [
        approval("carol", json!(null)),
        approval("carol", json!("soon")),
        approval("car\u{0}ol", json!("2099-01-01T00:00:00Z")),
        json!([{ "id": request, "approved": false, "by": "carol", "until": "2099-01-01T00:00:00Z" }]),
    ] {
        let refused = send(&app, report(&secret, answers.clone())).await;
        assert_eq!(refused.status, 400, "{answers}");
        assert_eq!(refused.json()["params"]["field"], "answers");
    }
    assert_eq!(requests(&app, &token).await[0]["status"], "pending");

    let taken = send(
        &app,
        report(
            &secret,
            approval("carol", json!("2099-01-01T02:00:00+02:00")),
        ),
    )
    .await;
    assert_eq!(taken.json()["requests"], json!([]));
    let listed = requests(&app, &token).await;
    assert_eq!(listed[0]["status"], "approved");
    assert_eq!(listed[0]["until"], "2099-01-01T00:00:00Z");
    // A second answer changes nothing.
    send(
        &app,
        report(
            &secret,
            json!([{ "id": request, "approved": false, "by": "dave", "until": null }]),
        ),
    )
    .await;
    assert_eq!(requests(&app, &token).await[0]["answered_by"], "carol");
}
