//! Directory changes reach running sessions (#108), with a directory whose
//! answers the tests change while users are signed in.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use remotehub_directory::laps::{LapsError, LapsPassword};
use remotehub_directory::{AuthError, Identity, IdentityProvider, Principal, Sid};
use remotehub_server::{AppState, app, refresh, session};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{
    BOB_SID, FakeDirectory, OLAF_SID, OPS_SID, authed, get, send, settings, sign_in_request, vault,
};

/// What the directory says about a user now.
#[derive(Clone)]
enum Answer {
    Groups(Vec<&'static str>),
    Disabled,
    Expired,
    Gone,
    Down,
}

/// The fake directory, except for the users `answers` names. Notes whom it
/// was asked about.
#[derive(Clone, Default)]
struct Changing {
    answers: Arc<Mutex<HashMap<&'static str, Answer>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Changing {
    fn set(&self, sid: &'static str, answer: Answer) {
        self.answers.lock().unwrap().insert(sid, answer);
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

impl IdentityProvider for Changing {
    async fn authenticate(&self, u: &str, p: &SecretString) -> Result<Identity, AuthError> {
        FakeDirectory.authenticate(u, p).await
    }

    async fn search(&self, query: &str, limit: i32) -> Result<Vec<Principal>, AuthError> {
        FakeDirectory.search(query, limit).await
    }

    async fn refresh(&self, sid: &Sid) -> Result<Vec<Sid>, AuthError> {
        self.asked.lock().unwrap().push(sid.to_string());
        let answer = self.answers.lock().unwrap().get(sid.as_str()).cloned();
        match answer {
            None => FakeDirectory.refresh(sid).await,
            Some(Answer::Groups(groups)) => Ok(groups.iter().map(|g| g.parse().unwrap()).collect()),
            Some(Answer::Disabled) => Err(AuthError::AccountDisabled),
            Some(Answer::Expired) => Err(AuthError::AccountExpired),
            Some(Answer::Gone) => Err(AuthError::InvalidCredentials),
            Some(Answer::Down) => Err(AuthError::Unavailable("connection refused".into())),
        }
    }

    async fn laps_password(&self, host: &str) -> Result<LapsPassword, LapsError> {
        FakeDirectory.laps_password(host).await
    }
}

struct Lab {
    state: AppState,
    app: axum::Router,
    directory: Changing,
    pool: PgPool,
}

async fn lab(pool: PgPool) -> Lab {
    let directory = Changing::default();
    let state = AppState::new(
        pool.clone(),
        Some(Arc::new(directory.clone())),
        settings(),
        vault(),
    );
    Lab {
        app: app(state.clone(), None),
        state,
        directory,
        pool,
    }
}

impl Lab {
    async fn sign_in(&self, user: &str) -> String {
        send(&self.app, sign_in_request(user, "right"))
            .await
            .session_token()
            .unwrap()
    }

    async fn signed_in(&self, token: &str) -> bool {
        send(&self.app, get("/api/session", Some(token)))
            .await
            .status
            == 200
    }

    /// Makes every session look as if it was checked long ago.
    async fn age(&self) {
        sqlx::query("UPDATE sessions SET checked_at = now() - interval '6 minutes'")
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn folders(&self, token: &str) -> Vec<String> {
        send(&self.app, get("/api/tree", Some(token))).await.json()["folders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn audited(&self, admin: &str) -> Vec<Value> {
        send(&self.app, get("/api/audit", Some(admin)))
            .await
            .json()
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["action"] == "session.ended_by_directory")
            .map(|e| e["details"].clone())
            .collect()
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn accounts_that_may_no_longer_sign_in_lose_their_sessions(pool: PgPool) {
    let lab = lab(pool).await;
    let alice = lab.sign_in("alice").await;
    let bob = lab.sign_in("bob").await;
    let olaf = lab.sign_in("olaf").await;
    lab.directory.set(BOB_SID, Answer::Disabled);
    lab.directory.set(OLAF_SID, Answer::Gone);

    // Sessions checked a moment ago wait for their turn.
    refresh::due(&lab.state).await.unwrap();
    assert!(lab.directory.asked().is_empty());
    assert!(lab.signed_in(&bob).await);

    lab.age().await;
    refresh::due(&lab.state).await.unwrap();
    assert!(lab.signed_in(&alice).await);
    assert!(!lab.signed_in(&bob).await);
    assert!(!lab.signed_in(&olaf).await);
    assert_eq!(lab.directory.asked().len(), 3);

    let mut ended = lab.audited(&alice).await;
    ended.sort_by_key(|d| d["sid"].as_str().unwrap().to_owned());
    assert_eq!(
        ended,
        [
            json!({ "reason": "account_disabled", "sid": BOB_SID, "sessions_ended": 1 }),
            json!({ "reason": "invalid_credentials", "sid": OLAF_SID, "sessions_ended": 1 }),
        ]
    );

    // Checked, alice waits for the next turn again.
    refresh::due(&lab.state).await.unwrap();
    assert!(lab.directory.asked().is_empty());

    lab.directory.set(BOB_SID, Answer::Expired);
    let bob = lab.sign_in("bob").await;
    lab.age().await;
    refresh::due(&lab.state).await.unwrap();
    assert!(!lab.signed_in(&bob).await);
    assert_eq!(lab.audited(&alice).await[0]["reason"], "account_expired");
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn new_groups_reach_open_sessions(pool: PgPool) {
    let lab = lab(pool).await;
    let alice = lab.sign_in("alice").await;
    let olaf = lab.sign_in("olaf").await;
    let bob = lab.sign_in("bob").await;
    let folder = send(
        &lab.app,
        authed(
            "POST",
            "/api/folders",
            Some(json!({ "parent_id": null, "name": "Ops" })),
            &alice,
        ),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let grant = json!({
        "object": { "kind": "folder", "id": folder },
        "principal_kind": "group", "principal_sid": OPS_SID, "principal_name": "RH Operators",
        "role": "list",
    });
    send(&lab.app, authed("POST", "/api/grants", Some(grant), &alice)).await;
    assert_eq!(lab.folders(&olaf).await, ["Ops"]);
    assert!(lab.folders(&bob).await.is_empty());

    // olaf left RH Operators, bob joined it.
    lab.directory.set(OLAF_SID, Answer::Groups(vec![]));
    lab.directory.set(BOB_SID, Answer::Groups(vec![OPS_SID]));
    lab.age().await;
    refresh::due(&lab.state).await.unwrap();
    assert!(lab.folders(&olaf).await.is_empty());
    assert_eq!(lab.folders(&bob).await, ["Ops"]);
    assert!(lab.signed_in(&olaf).await);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn an_unreachable_directory_changes_nothing(pool: PgPool) {
    let lab = lab(pool.clone()).await;
    let olaf = lab.sign_in("olaf").await;
    lab.directory.set(OLAF_SID, Answer::Down);
    lab.age().await;
    refresh::due(&lab.state).await.unwrap();
    assert!(lab.signed_in(&olaf).await);
    let groups: Vec<String> = sqlx::query_scalar("SELECT unnest(groups) FROM sessions")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(groups, [OPS_SID]);
    // Still due: the next round asks again.
    lab.directory.asked();
    refresh::due(&lab.state).await.unwrap();
    assert_eq!(lab.directory.asked(), [OLAF_SID]);
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn every_connection_asks_the_directory_first(pool: PgPool) {
    let lab = lab(pool).await;
    let olaf = lab.sign_in("olaf").await;
    let idle = lab.state.settings.session.idle;
    let current = || async {
        session::lookup(&lab.state.db, &olaf, idle)
            .await
            .unwrap()
            .unwrap()
    };

    // Checked a moment ago, and asked all the same.
    lab.directory.set(OLAF_SID, Answer::Groups(vec![]));
    let refreshed = refresh::before_connecting(&lab.state, current().await)
        .await
        .unwrap();
    assert!(refreshed.groups.is_empty());
    assert_eq!(lab.directory.asked(), [OLAF_SID]);

    lab.directory.set(OLAF_SID, Answer::Disabled);
    assert!(
        refresh::before_connecting(&lab.state, current().await)
            .await
            .is_err()
    );
    assert!(!lab.signed_in(&olaf).await);
}
