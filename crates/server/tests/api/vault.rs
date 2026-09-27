//! The vault like KeePass (#193): an entry carries tags and the day its
//! password runs out, may keep a one-time password whose codes the server
//! computes, and goes into a recycle bin before it goes for good.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use remotehub_server::app;
use remotehub_totp::{Params, unix_now};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{BOB_SID, ORIGIN, Response, authed, collection, send, sign_in_request, state};

/// A one-time password as a service hands it out in a QR code, with all
/// that can differ from the usual: SHA-256, eight digits, a minute a code.
const LINK: &str = "otpauth://totp/Shop:alice?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Shop&algorithm=SHA256&digits=8&period=60";

/// A bare secret as people copy it from a service: lower case, in groups,
/// and 80 bits, the least a secret may have.
const SECRET: &str = "jbsw y3dp ehpk 3pxp";

/// Pieces of [`LINK`] and [`SECRET`], in lower case: none of them may show
/// anywhere but in an export.
const SECRET_TRACES: [&str; 4] = ["gezdgnbvgy3tqojq", "jbswy3dp", "jbsw y3dp", "otpauth"];

async fn sign_in(app: &Router, user: &str) -> String {
    send(app, sign_in_request(user, "right"))
        .await
        .session_token()
        .unwrap()
}

/// Adds the keys of the JSON object `extra` to the object `body`, replacing
/// those it has.
fn merge(body: &mut Value, extra: Value) {
    let Value::Object(extra) = extra else {
        panic!("not a JSON object")
    };
    body.as_object_mut().unwrap().extend(extra);
}

/// Whether `value` is a time in UTC as RFC 3339 writes it.
fn is_utc_time(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.ends_with('Z')
            && time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
                .is_ok()
    })
}

/// Checks that `response` carries the current code of the one-time password
/// `text`, allowing one step of clock drift, and nothing but the code and how
/// long it holds.
fn assert_code(response: &Response, text: &str) {
    assert_eq!(response.status, StatusCode::OK, "{}", response.json());
    assert_eq!(response.headers["cache-control"], "no-store");
    let body = response.json();
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["code", "period", "remaining"], "{body}");
    let params = Params::parse(text).unwrap();
    let now = unix_now();
    let expected: Vec<String> = [now - params.period, now, now + params.period]
        .into_iter()
        .map(|at| params.code_at(at).0)
        .collect();
    assert!(
        expected.iter().any(|code| body["code"] == code.as_str()),
        "{body}, expected one of {expected:?}"
    );
    assert_eq!(body["period"], params.period);
    let remaining = body["remaining"].as_u64().unwrap();
    assert!((1..=params.period).contains(&remaining), "{body}");
}

/// The collection "Vault" at the top level, made by alice, who administers.
struct Vault {
    app: Router,
    pool: PgPool,
    alice: String,
    collection: String,
}

async fn vault(pool: PgPool) -> Vault {
    let app = app(state(pool.clone()), None);
    let alice = sign_in(&app, "alice").await;
    let collection = collection(&app, &alice, None, "Vault").await;
    Vault {
        app,
        pool,
        alice,
        collection,
    }
}

impl Vault {
    async fn call(&self, token: &str, method: &str, uri: &str, body: Option<Value>) -> Response {
        send(&self.app, authed(method, uri, body, token)).await
    }

    /// `POST /api/credentials` as alice: an entry `name` in the vault with a
    /// user name and a password, and what `extra` adds or replaces.
    async fn add(&self, name: &str, extra: Value) -> Response {
        let mut body = json!({
            "collection_id": self.collection, "name": name, "username": "admin",
            "password": "S3cret!",
        });
        merge(&mut body, extra);
        self.call(&self.alice, "POST", "/api/credentials", Some(body))
            .await
    }

    /// Adds an entry as [`Vault::add`] does. Returns its id.
    async fn create(&self, name: &str, extra: Value) -> String {
        let response = self.add(name, extra).await;
        assert_eq!(
            response.status,
            StatusCode::CREATED,
            "{name}: {}",
            response.json()
        );
        response.json()["id"].as_str().unwrap().to_owned()
    }

    /// `PUT /api/credentials/{id}` as alice: the entry `name` in the vault
    /// with the user name `admin`, and what `extra` adds. Without a password
    /// or a one-time password in `extra`, the sealed ones stay.
    async fn change(&self, id: &str, name: &str, extra: Value) -> Response {
        let mut body =
            json!({ "collection_id": self.collection, "name": name, "username": "admin" });
        merge(&mut body, extra);
        let uri = format!("/api/credentials/{id}");
        self.call(&self.alice, "PUT", &uri, Some(body)).await
    }

    /// The entry `id` as the tree lists it for `token`'s user; None if the
    /// tree does not list it.
    async fn listed(&self, token: &str, id: &str) -> Option<Value> {
        let tree = self.call(token, "GET", "/api/tree", None).await.json();
        tree["credentials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .cloned()
    }

    /// The ids of the files kept with the entry `id`.
    async fn files(&self, id: &str) -> Vec<String> {
        self.listed(&self.alice, id).await.unwrap()["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["id"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn code(&self, token: &str, id: &str, purpose: &str) -> Response {
        let uri = format!("/api/credentials/{id}/code");
        self.call(token, "POST", &uri, Some(json!({ "purpose": purpose })))
            .await
    }

    /// What alice sees when she reveals the entry `id` with `body`.
    async fn reveal(&self, id: &str, body: Value) -> Value {
        let uri = format!("/api/credentials/{id}/reveal");
        let response = self.call(&self.alice, "POST", &uri, Some(body)).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.json());
        response.json()
    }

    /// As alice: `DELETE /api/credentials/{id}`, into the recycle bin.
    async fn delete(&self, id: &str) {
        let uri = format!("/api/credentials/{id}");
        let deleted = self.call(&self.alice, "DELETE", &uri, None).await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.json());
    }

    /// As alice: `DELETE /api/credentials/{id}?purge=true`, for good.
    async fn purge(&self, id: &str) {
        let uri = format!("/api/credentials/{id}?purge=true");
        let purged = self.call(&self.alice, "DELETE", &uri, None).await;
        assert_eq!(purged.status, StatusCode::NO_CONTENT, "{}", purged.json());
    }

    /// Keeps a file with the entry `id`, sent as the browser sends it.
    async fn attach(&self, id: &str, name: &str, content: &[u8]) {
        let request = Request::post(format!("/api/credentials/{id}/attachments?name={name}"))
            .header("cookie", format!("__Host-remotehub-session={}", self.alice))
            .header("origin", ORIGIN)
            .header("content-type", "application/octet-stream")
            .body(Body::from(content.to_vec()))
            .unwrap();
        let sent = send(&self.app, request).await;
        assert_eq!(sent.status, StatusCode::NO_CONTENT, "{}", sent.json());
    }

    /// Grants bob `role` on the collection or credential `id`; granting
    /// again replaces the role.
    async fn grant_bob(&self, kind: &str, id: &str, role: &str) {
        let body = json!({
            "object": { "kind": kind, "id": id },
            "principal_kind": "user", "principal_sid": BOB_SID, "principal_name": "Bob",
            "role": role,
        });
        let response = self
            .call(&self.alice, "POST", "/api/grants", Some(body))
            .await;
        assert_eq!(
            response.status,
            StatusCode::NO_CONTENT,
            "{}",
            response.json()
        );
    }

    /// The audit log, newest first.
    async fn audit(&self) -> Vec<Value> {
        let log = self
            .call(&self.alice, "GET", "/api/audit?limit=500", None)
            .await
            .json();
        log.as_array().unwrap().clone()
    }

    /// `(action, details)` of each audit entry about `object`, newest first.
    async fn audited(&self, object: &str) -> Vec<(String, Value)> {
        self.audit()
            .await
            .into_iter()
            .filter(|entry| entry["object_id"] == object)
            .map(|entry| {
                (
                    entry["action"].as_str().unwrap().to_owned(),
                    entry["details"].clone(),
                )
            })
            .collect()
    }

    /// The versions in which `field` of `owner` is sealed, oldest first.
    async fn sealed(&self, owner: &str, field: &str) -> Vec<i32> {
        sqlx::query_scalar(
            "SELECT version FROM secret_fields WHERE owner_id = $1::uuid AND field = $2
             ORDER BY version",
        )
        .bind(owner)
        .bind(field)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    /// What the database still holds of the entry `id` whose files were
    /// `files`: `(entries, files, sealed values)`, the sealed values being
    /// those of the entry and of its files.
    async fn remains(&self, id: &str, files: &[String]) -> (i64, i64, i64) {
        sqlx::query_as(
            "SELECT (SELECT count(*) FROM credentials WHERE id = $1::uuid),
                    (SELECT count(*) FROM credential_attachments WHERE credential_id = $1::uuid),
                    (SELECT count(*) FROM secret_fields
                     WHERE owner_id = $1::uuid OR owner_id::text = ANY($2))",
        )
        .bind(id)
        .bind(files)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

// ── Tags and expiry ─────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn tags_and_an_expiry_day_are_kept_and_listed(pool: PgPool) {
    let v = vault(pool).await;
    // Tags are trimmed, and one that differs from an earlier one only in
    // case is the same tag.
    let id = v
        .create(
            "router",
            json!({ "tags": [" web ", "Prod", "WEB", "prod", "Übung"], "expires_on": "2028-02-29" }),
        )
        .await;
    let row = v.listed(&v.alice, &id).await.unwrap();
    assert_eq!(row["tags"], json!(["web", "Prod", "Übung"]));
    assert_eq!(row["expires_on"], "2028-02-29");
    assert_eq!(row["has_totp"], false);
    assert!(row["deleted_at"].is_null(), "{row}");
    assert!(is_utc_time(&row["updated_at"]), "{row}");

    // Neither is a secret: changing them makes no new version.
    let changed = v
        .change(
            &id,
            "router",
            json!({ "tags": ["lan"], "expires_on": "2027-01-31" }),
        )
        .await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.json());
    let row = v.listed(&v.alice, &id).await.unwrap();
    assert_eq!(
        (&row["tags"], &row["expires_on"], &row["version"]),
        (&json!(["lan"]), &json!("2027-01-31"), &json!(1))
    );

    // An empty day, or none at all, means it never runs out.
    for never in [json!(""), json!("  "), Value::Null] {
        v.change(&id, "router", json!({ "expires_on": "2027-01-31" }))
            .await;
        let row = v.listed(&v.alice, &id).await.unwrap();
        assert_eq!(row["expires_on"], "2027-01-31");
        let changed = v
            .change(&id, "router", json!({ "expires_on": never.clone() }))
            .await;
        assert_eq!(changed.status, StatusCode::NO_CONTENT, "{never}");
        let row = v.listed(&v.alice, &id).await.unwrap();
        assert!(row["expires_on"].is_null(), "{never}: {row}");
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn tags_and_expiry_days_are_checked(pool: PgPool) {
    let v = vault(pool).await;
    // Twenty tags at most, counted after "T0" turned out to be "t0"; fifty
    // characters each, however many bytes they take.
    let mut tags: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
    tags[1] = "ä".repeat(50);
    tags.push("T0".to_owned());
    let id = v.create("many", json!({ "tags": tags })).await;
    let tags_of = |row: Option<Value>| row.unwrap()["tags"].as_array().unwrap().len();
    assert_eq!(tags_of(v.listed(&v.alice, &id).await), 20);

    let too_many: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
    for (field, value) in [
        ("tags", json!(too_many)),
        ("tags", json!(["x".repeat(51)])),
        ("tags", json!([""])),
        ("tags", json!(["  "])),
        ("tags", json!(["a\tb"])),
        ("expires_on", json!("2026-02-30")),
        ("expires_on", json!("2027-02-29")),
        ("expires_on", json!("2026-13-01")),
        ("expires_on", json!("2026-12-00")),
        ("expires_on", json!("31.12.2026")),
        ("expires_on", json!("2026-12-31T00:00:00Z")),
        ("expires_on", json!("never")),
    ] {
        let mut extra = json!({});
        extra[field] = value.clone();
        let added = v.add("refused", extra.clone()).await;
        let changed = v.change(&id, "many", extra).await;
        for refused in [added, changed] {
            assert_eq!(
                (refused.code().as_str(), &refused.json()["params"]["field"]),
                ("invalid_request", &json!(field)),
                "{field}: {value}"
            );
        }
    }
    // Nothing refused was stored.
    let tree = v.call(&v.alice, "GET", "/api/tree", None).await.json();
    assert_eq!(tree["credentials"].as_array().unwrap().len(), 1, "{tree}");
    assert_eq!(tags_of(v.listed(&v.alice, &id).await), 20);
}

/// PostgreSQL knows no year 0: a day in it is refused like any other day
/// that does not exist, instead of failing in the database.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_day_outside_the_years_1_to_9999_is_refused(pool: PgPool) {
    let v = vault(pool).await;
    let id = v.create("ancient", json!({})).await;
    let mut answers = Vec::new();
    // PostgreSQL has no year 0.
    for day in ["0000-01-01", "10000-01-01"] {
        let day = json!({ "expires_on": day });
        answers.push(v.add("older", day.clone()).await);
        answers.push(v.change(&id, "ancient", day).await);
    }
    for refused in answers {
        assert_eq!(
            (
                refused.status,
                refused.code().as_str(),
                &refused.json()["params"]["field"]
            ),
            (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                &json!("expires_on")
            ),
            "{}",
            refused.json()
        );
    }
}

/// KeePass keeps "DOMAIN\user" in the user name, and so does the vault: its
/// entries have no domain of their own.
#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_domain_stays_in_the_user_name(pool: PgPool) {
    let v = vault(pool).await;
    let id = v.create("dc", json!({ "username": "LAB\\admin" })).await;
    let row = v.listed(&v.alice, &id).await.unwrap();
    assert_eq!(row["username"], "LAB\\admin");
    assert!(row.get("domain").is_none(), "{row}");
    let revealed = v.reveal(&id, json!({ "purpose": "show" })).await;
    assert_eq!(
        (&revealed["username"], &revealed["domain"]),
        (&json!("LAB\\admin"), &json!(""))
    );
}

// ── One-time passwords ──────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_server_computes_the_one_time_codes(pool: PgPool) {
    let v = vault(pool).await;
    let linked = v.create("shop", json!({ "totp": LINK })).await;
    let bare = v.create("mail", json!({ "totp": SECRET })).await;
    for (id, text) in [(&linked, LINK), (&bare, SECRET)] {
        assert_eq!(v.listed(&v.alice, id).await.unwrap()["has_totp"], true);
        for purpose in ["show", "copy"] {
            assert_code(&v.code(&v.alice, id, purpose).await, text);
        }
    }

    // Without one, or with an empty one, there is no code.
    for (name, extra) in [
        ("none", json!({})),
        ("empty", json!({ "totp": "" })),
        ("null", json!({ "totp": null })),
    ] {
        let id = v.create(name, extra).await;
        assert_eq!(v.listed(&v.alice, &id).await.unwrap()["has_totp"], false);
        let none = v.code(&v.alice, &id, "show").await;
        assert_eq!(
            (none.status, none.code().as_str()),
            (StatusCode::NOT_FOUND, "not_found"),
            "{name}"
        );
    }
    // A code is shown or copied; an export takes the secret instead.
    let exported = v.code(&v.alice, &linked, "export").await;
    assert_eq!(exported.json()["params"]["field"], "purpose");

    // Anything that is no TOTP is refused.
    for odd in [
        "JBSWY3DP",
        "not base32!",
        "otpauth://hotp/Shop?secret=JBSWY3DPEHPK3PXP&counter=1",
    ] {
        let refused = v.add("odd", json!({ "totp": odd })).await;
        assert_eq!(
            (refused.code().as_str(), &refused.json()["params"]["field"]),
            ("invalid_request", &json!("totp")),
            "{odd}"
        );
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_one_time_code_takes_reveal_and_is_audited(pool: PgPool) {
    let v = vault(pool).await;
    let id = v.create("shop", json!({ "totp": LINK })).await;
    let bob = sign_in(&v.app, "bob").await;
    let hidden = v.code(&bob, &id, "show").await;
    assert_eq!(
        (hidden.status, hidden.code().as_str()),
        (StatusCode::NOT_FOUND, "not_found")
    );
    for role in ["list", "connect"] {
        v.grant_bob("collection", &v.collection, role).await;
        let refused = v.code(&bob, &id, "show").await;
        assert_eq!(
            (refused.status, refused.code().as_str()),
            (StatusCode::FORBIDDEN, "forbidden"),
            "{role}"
        );
    }
    v.grant_bob("collection", &v.collection, "reveal").await;
    assert_code(&v.code(&bob, &id, "show").await, LINK);
    assert_code(&v.code(&bob, &id, "copy").await, LINK);

    // Each code shown is logged, the refused ones are not.
    let log = v.audit().await;
    let shown: Vec<(&str, &str, &Value)> = log
        .iter()
        .filter(|entry| entry["action"] == "credential.code_shown")
        .map(|entry| {
            (
                entry["actor_name"].as_str().unwrap(),
                entry["object_id"].as_str().unwrap(),
                &entry["details"],
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("bob", id.as_str(), &json!({ "purpose": "copy" })),
            ("bob", id.as_str(), &json!({ "purpose": "show" })),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_one_time_secret_leaves_only_in_an_export(pool: PgPool) {
    let v = vault(pool.clone()).await;
    let id = v.create("shop", json!({ "totp": LINK })).await;
    let other = v.create("mail", json!({ "totp": SECRET })).await;
    // Its audit entry is checked below, with the others.
    assert_code(&v.code(&v.alice, &id, "show").await, LINK);

    for purpose in ["show", "copy"] {
        let revealed = v.reveal(&id, json!({ "purpose": purpose })).await;
        assert_eq!(revealed["password"], "S3cret!");
        assert!(revealed.get("totp").is_none(), "{purpose}: {revealed}");
    }
    // Into a KeePass file, the secret goes as it was given.
    for (id, text) in [(&id, LINK), (&other, SECRET)] {
        let exported = v.reveal(id, json!({ "purpose": "export" })).await;
        assert_eq!(exported["totp"], text);
    }

    let tree = v
        .call(&v.alice, "GET", "/api/tree", None)
        .await
        .json()
        .to_string()
        .to_lowercase();
    let log = serde_json::to_string(&v.audit().await)
        .unwrap()
        .to_lowercase();
    for trace in SECRET_TRACES {
        assert!(!tree.contains(trace), "tree: {trace}");
        assert!(!log.contains(trace), "audit log: {trace}");
    }
    let sealed: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT ciphertext FROM secret_fields WHERE field = 'totp'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(sealed.len(), 2);
    for ciphertext in sealed {
        let text = String::from_utf8_lossy(&ciphertext).to_lowercase();
        assert!(
            SECRET_TRACES.iter().all(|trace| !text.contains(trace)),
            "{text}"
        );
    }
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_change_keeps_the_one_time_password_unless_it_names_another(pool: PgPool) {
    let v = vault(pool).await;
    let id = v.create("shop", json!({ "totp": SECRET })).await;
    let version = |row: Option<Value>| {
        let row = row.unwrap();
        (row["version"].as_i64().unwrap(), row["has_totp"].clone())
    };

    // A change without it keeps it, in the same version …
    let kept = v.change(&id, "shop", json!({ "notes": "2FA" })).await;
    assert_eq!(kept.status, StatusCode::NO_CONTENT, "{}", kept.json());
    assert_eq!(version(v.listed(&v.alice, &id).await), (1, json!(true)));
    assert_code(&v.code(&v.alice, &id, "show").await, SECRET);
    // … and into the next one when the password changes.
    v.change(&id, "shop", json!({ "password": "N3w-S3cret!" }))
        .await;
    assert_eq!(version(v.listed(&v.alice, &id).await), (2, json!(true)));
    assert_eq!(v.sealed(&id, "totp").await, [1, 2]);
    assert_code(&v.code(&v.alice, &id, "show").await, SECRET);

    // Anything that is no TOTP is refused and changes nothing.
    for odd in ["JBSWY3DP", "otpauth://totp/Shop?issuer=Shop"] {
        let refused = v.change(&id, "shop", json!({ "totp": odd })).await;
        assert_eq!(
            (refused.code().as_str(), &refused.json()["params"]["field"]),
            ("invalid_request", &json!("totp")),
            "{odd}"
        );
    }
    assert_eq!(version(v.listed(&v.alice, &id).await), (2, json!(true)));

    // Another one makes a new version, like a new password, and the
    // password comes along.
    v.change(&id, "shop", json!({ "totp": LINK })).await;
    assert_eq!(version(v.listed(&v.alice, &id).await), (3, json!(true)));
    assert_code(&v.code(&v.alice, &id, "show").await, LINK);

    // An empty one removes it from the next version on; the version before
    // keeps it, as it keeps its password.
    v.change(&id, "shop", json!({ "totp": "" })).await;
    assert_eq!(version(v.listed(&v.alice, &id).await), (4, json!(false)));
    assert_eq!(v.sealed(&id, "totp").await, [1, 2, 3]);
    assert_eq!(v.sealed(&id, "password").await, [1, 2, 3, 4]);
    let gone = v.code(&v.alice, &id, "show").await;
    assert_eq!(
        (gone.status, gone.code().as_str()),
        (StatusCode::NOT_FOUND, "not_found")
    );
    let now = v.reveal(&id, json!({ "purpose": "export" })).await;
    assert_eq!(
        (&now["password"], now.get("totp")),
        (&json!("N3w-S3cret!"), None)
    );
    let before = v
        .reveal(&id, json!({ "purpose": "export", "version": 3 }))
        .await;
    assert_eq!(before["totp"], LINK);

    // Removing what is not there is no change.
    v.change(&id, "shop", json!({ "totp": "" })).await;
    assert_eq!(version(v.listed(&v.alice, &id).await), (4, json!(false)));
}

// ── Recycle bin ─────────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_deleted_entry_waits_in_the_recycle_bin_until_it_is_deleted_again(pool: PgPool) {
    let v = vault(pool).await;
    let id = v
        .create(
            "router",
            json!({
                "fields": [{ "name": "PIN", "protected": true, "value": "1234" }],
                "totp": SECRET,
            }),
        )
        .await;
    v.attach(&id, "cert.pem", b"-----BEGIN-----").await;
    let files = v.files(&id).await;
    let other = v.create("switch", json!({})).await;
    let restore = format!("/api/credentials/{id}/restore");

    // Only what is in the bin can be restored.
    let odd = v.call(&v.alice, "POST", &restore, None).await;
    assert_eq!(
        (odd.code().as_str(), &odd.json()["params"]["field"]),
        ("invalid_request", &json!("id"))
    );

    // Deleted once, it is still listed, marked, and all it keeps still opens.
    v.delete(&id).await;
    let row = v.listed(&v.alice, &id).await.expect("listed in the bin");
    assert!(is_utc_time(&row["deleted_at"]), "{row}");
    assert!(v.listed(&v.alice, &other).await.unwrap()["deleted_at"].is_null());
    let revealed = v.reveal(&id, json!({ "purpose": "show" })).await;
    assert_eq!(
        (&revealed["password"], &revealed["fields"]),
        (
            &json!("S3cret!"),
            &json!([{ "name": "PIN", "value": "1234" }])
        )
    );
    let uri = format!("/api/credentials/{id}/attachments/{}", files[0]);
    let file = v.call(&v.alice, "GET", &uri, None).await;
    assert_eq!(file.body, b"-----BEGIN-----");

    // Restored, it is back in its collection.
    let restored = v.call(&v.alice, "POST", &restore, None).await;
    assert_eq!(
        restored.status,
        StatusCode::NO_CONTENT,
        "{}",
        restored.json()
    );
    assert!(v.listed(&v.alice, &id).await.unwrap()["deleted_at"].is_null());

    // Deleted twice, as a double click would, it stays in the bin with all
    // it keeps.
    let (entries, kept_files, sealed) = v.remains(&id, &files).await;
    assert_eq!((entries, kept_files), (1, 1));
    assert!(sealed >= 4, "password, PIN, TOTP and the file: {sealed}");
    v.delete(&id).await;
    v.delete(&id).await;
    assert!(is_utc_time(
        &v.listed(&v.alice, &id).await.unwrap()["deleted_at"]
    ));
    assert_eq!(v.remains(&id, &files).await, (1, 1, sealed));
    let odd = format!("/api/credentials/{id}?purge=maybe");
    let odd = v.call(&v.alice, "DELETE", &odd, None).await;
    assert_eq!(odd.code(), "invalid_request");

    // Purged, it is gone for good, with its sealed values and files.
    v.purge(&id).await;
    assert!(v.listed(&v.alice, &id).await.is_none());
    assert_eq!(v.remains(&id, &files).await, (0, 0, 0));
    for (method, uri) in [
        ("DELETE", format!("/api/credentials/{id}")),
        ("DELETE", format!("/api/credentials/{id}?purge=true")),
        ("POST", restore.clone()),
    ] {
        let gone = v.call(&v.alice, method, &uri, None).await;
        assert_eq!(gone.code(), "not_found", "{method} {uri}");
    }
    // The entry next to it is untouched.
    assert!(v.listed(&v.alice, &other).await.is_some());
    assert_eq!(v.sealed(&other, "password").await, [1]);

    let bin: Vec<(String, Value)> = v
        .audited(&id)
        .await
        .into_iter()
        .filter(|(action, _)| action == "credential.deleted" || action == "credential.restored")
        .collect();
    let entry = |action: &str, details: Value| (action.to_owned(), details);
    assert_eq!(
        bin,
        [
            entry("credential.deleted", json!({ "purged": true })),
            entry("credential.deleted", json!({ "purged": false })),
            entry("credential.restored", json!({})),
            entry("credential.deleted", json!({ "purged": false })),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn the_recycle_bin_takes_edit(pool: PgPool) {
    let v = vault(pool).await;
    let id = v.create("router", json!({})).await;
    let bob = sign_in(&v.app, "bob").await;
    let uri = format!("/api/credentials/{id}");
    let restore = format!("/api/credentials/{id}/restore");
    let binned = || {
        let (v, id) = (&v, &id);
        async move { !v.listed(&v.alice, id).await.unwrap()["deleted_at"].is_null() }
    };

    // For bob, it does not exist.
    for (method, target) in [("DELETE", &uri), ("POST", &restore)] {
        let hidden = v.call(&bob, method, target, None).await;
        assert_eq!(
            (hidden.status, hidden.code().as_str()),
            (StatusCode::NOT_FOUND, "not_found"),
            "{method} {target}"
        );
    }
    // reveal is not edit: bob neither bins it, nor restores it, nor
    // deletes it for good.
    v.grant_bob("collection", &v.collection, "reveal").await;
    let refused = v.call(&bob, "DELETE", &uri, None).await;
    assert_eq!(refused.code(), "forbidden");
    assert!(!binned().await);
    v.delete(&id).await;
    let purge = format!("{uri}?purge=true");
    for (method, target) in [("POST", &restore), ("DELETE", &uri), ("DELETE", &purge)] {
        let refused = v.call(&bob, method, target, None).await;
        assert_eq!(refused.code(), "forbidden", "{method} {target}");
    }
    assert!(binned().await);

    // edit does all three, on the entry alone as well; a purge needs no
    // stop in the bin first.
    v.grant_bob("credential", &id, "edit").await;
    let restored = v.call(&bob, "POST", &restore, None).await;
    assert_eq!(
        restored.status,
        StatusCode::NO_CONTENT,
        "{}",
        restored.json()
    );
    assert!(!binned().await);
    let binning = v.call(&bob, "DELETE", &uri, None).await;
    assert_eq!(binning.status, StatusCode::NO_CONTENT, "{}", binning.json());
    assert!(binned().await);
    v.call(&bob, "POST", &restore, None).await;
    let purged = v.call(&bob, "DELETE", &purge, None).await;
    assert_eq!(purged.status, StatusCode::NO_CONTENT, "{}", purged.json());
    assert!(v.listed(&v.alice, &id).await.is_none());
}

#[sqlx::test(migrations = "../../migrations", fixtures("set_up"))]
async fn a_collection_goes_with_its_recycle_bin_but_not_with_a_live_entry(pool: PgPool) {
    let v = vault(pool).await;
    let team = collection(&v.app, &v.alice, Some(&v.collection), "Team").await;
    let a = v
        .create("a", json!({ "collection_id": team, "totp": SECRET }))
        .await;
    v.attach(&a, "a.pem", b"a").await;
    let a_files = v.files(&a).await;
    let b = v.create("b", json!({ "collection_id": team })).await;
    let c = v.create("c", json!({})).await;
    v.delete(&a).await;
    v.delete(&c).await;
    let delete = |id: &str| {
        let uri = format!("/api/collections/{id}");
        let v = &v;
        async move { v.call(&v.alice, "DELETE", &uri, None).await }
    };

    // b is not in the bin: Team stays, and so does a, in the bin with all
    // it keeps. Vault holds Team: it stays, and so does c.
    for (collection, binned, files) in [(&team, &a, &a_files), (&v.collection, &c, &Vec::new())] {
        let kept = delete(collection).await;
        assert_eq!(
            (kept.status, kept.code().as_str()),
            (StatusCode::CONFLICT, "folder_not_empty"),
            "{collection}"
        );
        let row = v.listed(&v.alice, binned).await.unwrap();
        assert!(is_utc_time(&row["deleted_at"]), "{row}");
        let (entries, kept_files, sealed) = v.remains(binned, files).await;
        assert_eq!((entries, kept_files), (1, files.len() as i64), "{binned}");
        assert!(sealed > 0, "{binned}");
    }

    // With b in the bin as well, Team goes and takes a and b along; then
    // Vault goes and takes c.
    v.delete(&b).await;
    for (collection, purged) in [(&team, 2), (&v.collection, 1)] {
        let gone = delete(collection).await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.json());
        assert_eq!(
            v.audited(collection).await[0],
            ("collection.deleted".to_owned(), json!({ "purged": purged }))
        );
    }
    for (id, files) in [(&a, &a_files), (&b, &Vec::new()), (&c, &Vec::new())] {
        assert_eq!(v.remains(id, files).await, (0, 0, 0), "{id}");
    }
}
