//! The password generator's settings (#194): the organisation's default,
//! set by administrators and audited, and a default of each user's own,
//! which is nobody else's business.

use axum::Router;
use axum::http::StatusCode;
use remotehub_server::app;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{Response, authed, send, sign_in_request, state};

async fn sign_in(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

async fn call(app: &Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
    send(app, authed(method, uri, body, token)).await
}

/// What the generator makes before anyone sets anything.
fn built_in() -> Value {
    json!({
        "kind": "password", "length": 20, "lower": true, "upper": true, "digits": true,
        "symbols": true, "look_alikes": false, "words": 6, "separator": "-",
    })
}

/// The built-in settings with the keys of `changes` replaced.
fn with(changes: Value) -> Value {
    let mut settings = built_in();
    settings
        .as_object_mut()
        .unwrap()
        .extend(changes.as_object().unwrap().clone());
    settings
}

/// `details` of every `generator.changed` in the audit log, newest first.
async fn audited(app: &Router, token: &str) -> Vec<Value> {
    let log = call(app, token, "GET", "/api/audit?limit=500", None)
        .await
        .json();
    log.as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["action"] == "generator.changed")
        .map(|entry| entry["details"].clone())
        .collect()
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_organisation_sets_a_default_and_each_user_may_keep_their_own(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = sign_in(&app, "alice").await;
    let bob = sign_in(&app, "bob").await;
    let read = |token: &str| {
        let (app, token) = (&app, token.to_owned());
        async move {
            call(app, &token, "GET", "/api/generator", None)
                .await
                .json()
        }
    };
    let status = |response: Response| (response.status, response.code());

    // Built in until somebody sets something.
    assert_eq!(
        read(&bob).await,
        json!({ "organisation": built_in(), "own": null })
    );

    // Only an administrator sets the organisation's, and it is audited.
    let phrase = with(json!({ "kind": "passphrase", "words": 5, "separator": "." }));
    let org = "/api/settings/generator";
    let refused = call(&app, &bob, "PUT", org, Some(phrase.clone())).await;
    assert_eq!(
        status(refused),
        (StatusCode::FORBIDDEN, "forbidden".to_owned())
    );
    assert_eq!(read(&bob).await["organisation"], built_in());
    let saved = call(&app, &alice, "PUT", org, Some(phrase.clone())).await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.json());
    assert_eq!(
        read(&bob).await,
        json!({ "organisation": phrase, "own": null })
    );
    // A second change replaces the first.
    let longer = with(json!({ "length": 32 }));
    call(&app, &alice, "PUT", org, Some(longer.clone())).await;
    assert_eq!(read(&bob).await["organisation"], longer);
    assert_eq!(audited(&app, &alice).await, [longer.clone(), phrase]);

    // bob's own is his alone, replaced by the next, removed on request;
    // a preference, so not audited.
    let own = "/api/generator/own";
    for settings in [
        with(json!({ "symbols": false, "look_alikes": true })),
        with(json!({ "length": 64 })),
    ] {
        let saved = call(&app, &bob, "PUT", own, Some(settings.clone())).await;
        assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.json());
        assert_eq!(
            read(&bob).await,
            json!({ "organisation": longer, "own": settings })
        );
        assert_eq!(read(&alice).await["own"], Value::Null);
    }
    let removed = call(&app, &bob, "DELETE", own, None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        read(&bob).await,
        json!({ "organisation": longer, "own": null })
    );
    assert_eq!(audited(&app, &alice).await.len(), 2);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn settings_the_generator_cannot_follow_are_refused(pool: PgPool) {
    let app = app(state(pool.clone()), None);
    let alice = sign_in(&app, "alice").await;
    let targets = ["/api/generator/own", "/api/settings/generator"];

    let no_characters =
        json!({ "lower": false, "upper": false, "digits": false, "symbols": false });
    for (field, changes) in [
        ("kind", json!({ "kind": "pin" })),
        ("length", json!({ "length": 7 })),
        ("length", json!({ "length": 129 })),
        ("characters", no_characters),
        ("words", json!({ "words": 2 })),
        ("words", json!({ "words": 21 })),
        ("separator", json!({ "separator": "----" })),
        ("separator", json!({ "separator": "\t" })),
    ] {
        for target in targets {
            let refused = call(&app, &alice, "PUT", target, Some(with(changes.clone()))).await;
            assert_eq!(
                (refused.code(), &refused.json()["params"]["field"]),
                ("invalid_request".to_owned(), &json!(field)),
                "{target} {changes}"
            );
        }
    }
    // Settings with a key missing are no settings.
    let mut partial = built_in();
    partial.as_object_mut().unwrap().remove("separator");
    for target in targets {
        let refused = call(&app, &alice, "PUT", target, Some(partial.clone())).await;
        assert_eq!(refused.code(), "invalid_request", "{target}");
    }
    let stored = call(&app, &alice, "GET", "/api/generator", None)
        .await
        .json();
    assert_eq!(stored, json!({ "organisation": built_in(), "own": null }));

    // The bounds themselves are fine; a separator counts characters.
    for changes in [
        json!({ "length": 8, "words": 3, "separator": "" }),
        json!({ "length": 128, "words": 20, "separator": "·–·" }),
        json!({ "lower": false, "upper": false, "digits": true, "symbols": false }),
    ] {
        for target in targets {
            let saved = call(&app, &alice, "PUT", target, Some(with(changes.clone()))).await;
            assert_eq!(
                saved.status,
                StatusCode::NO_CONTENT,
                "{target} {changes}: {}",
                saved.json()
            );
        }
    }
}
