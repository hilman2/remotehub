//! The Access page (#178): users with groups and roles, groups with members,
//! and the grants of one user or group. alice administers through the
//! directory group RH Admins, olaf is in RH Operators, bob has no groups.

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{ADMINS_SID, BOB_SID, OPS_SID, authed, get, send, sign_in_request};
use crate::terminal::setup;

async fn overview(app: &Router, token: &str) -> Value {
    let response = send(app, get("/api/access", Some(token))).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json());
    response.json()
}

fn user<'a>(overview: &'a Value, username: &str) -> &'a Value {
    overview["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == username)
        .unwrap_or_else(|| panic!("{username} is not listed"))
}

fn group<'a>(overview: &'a Value, sid: &str) -> Option<&'a Value> {
    overview["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["sid"] == sid)
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn users_come_with_their_groups_and_roles(pool: PgPool) {
    let (state, app, alice, folder) = setup(pool).await;
    // Signed in once, olaf's directory groups are known.
    send(&app, sign_in_request("olaf", "right")).await;
    let bob = send(&app, sign_in_request("bob", "right"))
        .await
        .session_token()
        .unwrap();
    // Not an auditor: nothing.
    assert_eq!(
        send(&app, get("/api/access", Some(&bob))).await.status,
        StatusCode::FORBIDDEN
    );

    // An own group with bob himself and the directory group RH Operators.
    let created = send(
        &app,
        authed(
            "POST",
            "/api/groups",
            Some(json!({ "name": "On-call" })),
            &alice,
        ),
    )
    .await;
    let on_call = created.json()["id"].as_str().unwrap().to_owned();
    for (sid, kind) in [(BOB_SID, "user"), (OPS_SID, "group")] {
        let added = send(
            &app,
            authed(
                "PUT",
                &format!("/api/groups/{on_call}/members/{sid}"),
                Some(json!({ "principal_kind": kind, "principal_name": "someone" })),
                &alice,
            ),
        )
        .await;
        assert_eq!(added.status, StatusCode::NO_CONTENT, "{sid}");
    }

    // Sign-in remembers the directory groups' names beside it.
    let mut named = false;
    for _ in 0..50 {
        let name: Option<String> =
            sqlx::query_scalar("SELECT name FROM directory_groups WHERE sid = $1")
                .bind(OPS_SID)
                .fetch_optional(&state.db)
                .await
                .unwrap();
        if name.as_deref() == Some("RH Operators") {
            named = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(named, "the name of RH Operators was not remembered");

    let seen = overview(&app, &alice).await;
    let alice_row = user(&seen, "alice");
    assert_eq!(
        alice_row["groups"],
        json!([{ "sid": ADMINS_SID, "name": "RH Admins", "source": "directory", "via": null }])
    );
    assert_eq!(
        alice_row["roles"],
        json!([{ "role": "administrator", "via": { "sid": ADMINS_SID, "name": "RH Admins" } }])
    );
    let own = format!("group:{on_call}");
    assert_eq!(
        user(&seen, "bob")["groups"],
        json!([{ "sid": own, "name": "On-call", "source": "own", "via": null }])
    );
    // olaf is in On-call through RH Operators.
    let olaf_groups = &user(&seen, "olaf")["groups"];
    assert_eq!(olaf_groups[1]["sid"], own.as_str());
    assert_eq!(olaf_groups[1]["via"]["name"], "RH Operators");

    let on_call_group = group(&seen, &own).unwrap();
    assert_eq!(on_call_group["source"], "own");
    let members: Vec<&str> = on_call_group["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["sid"].as_str().unwrap())
        .collect();
    assert_eq!(members.len(), 2);
    assert!(members.contains(&BOB_SID) && members.contains(&OPS_SID));
    let operators = group(&seen, OPS_SID).unwrap();
    assert_eq!(operators["source"], "directory");
    assert_eq!(operators["name"], "RH Operators");
    assert_eq!(operators["members"][0]["name"], "Olaf Operator");
    assert_eq!(
        operators["member_of"],
        json!([{ "sid": own, "name": "On-call" }])
    );
    let admins = group(&seen, ADMINS_SID).unwrap();
    assert_eq!(admins["roles"], json!(["administrator"]));

    // A grant with an end stands beside one without, and ends by itself.
    let grant = |role: &str, expires_at: Value| {
        json!({ "object": { "kind": "folder", "id": folder }, "principal_kind": "user",
                "principal_sid": BOB_SID, "principal_name": "Bob Helpdesk", "role": role,
                "expires_at": expires_at })
    };
    for (body, status) in [
        (
            grant("reveal", json!("2020-01-01T00:00:00Z")),
            StatusCode::BAD_REQUEST,
        ),
        (grant("reveal", json!("soon")), StatusCode::BAD_REQUEST),
        (grant("connect", json!(null)), StatusCode::NO_CONTENT),
        (
            grant("reveal", json!("2099-06-30T22:00:00+02:00")),
            StatusCode::NO_CONTENT,
        ),
    ] {
        let response = send(
            &app,
            authed("POST", "/api/grants", Some(body.clone()), &alice),
        )
        .await;
        assert_eq!(response.status, status, "{body}");
    }
    let listed = send(
        &app,
        get(
            &format!("/api/access/grants?principal={BOB_SID}"),
            Some(&alice),
        ),
    )
    .await
    .json();
    let roles: Vec<(&str, Option<&str>)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|g| (g["role"].as_str().unwrap(), g["expires_at"].as_str()))
        .collect();
    assert_eq!(roles.len(), 2, "{listed}");
    assert!(roles.contains(&("connect", None)));
    assert!(roles.contains(&("reveal", Some("2099-06-30T20:00:00Z"))));
    assert_eq!(
        listed[0]["object"],
        json!({ "kind": "folder", "id": folder })
    );
    assert_eq!(listed[0]["name"], "Lab");
    assert_eq!(
        send(
            &app,
            get(
                &format!("/api/access/grants?principal={BOB_SID}"),
                Some(&bob)
            )
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}
