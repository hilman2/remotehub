//! Ory Kratos, where local accounts live (#103, ADR 0010).
//!
//! Browsers reach Kratos' public API only through remotehub, under
//! `/api/auth/`: its cookies and flows stay on remotehub's origin, and Kratos
//! itself stays on the internal network with its admin API. Kratos proves
//! who someone is; remotehub keeps its own session and decides what they may
//! do.

use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, Method, Request, Response, StatusCode, header};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::config::KratosConfig;
use crate::{totp, webauthn};

/// Adds an invited account to remotehub's users, so that administrators see
/// it before its first sign-in. Returns its user ID. The first sign-in fills
/// in the rest (`api::accounts`).
pub async fn add_invited<'e>(
    db: impl sqlx::PgExecutor<'e>,
    invitation: &Invitation,
    email: &str,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    let name = if name.is_empty() { email } else { name };
    sqlx::query_scalar(
        "INSERT INTO users (kind, identity_id, username, display_name, email)
         VALUES ('local', $1, $2, $3, $2)
         ON CONFLICT (identity_id) DO UPDATE SET identity_id = EXCLUDED.identity_id
         RETURNING id",
    )
    .bind(invitation.identity_id)
    .bind(email)
    .bind(name)
    .fetch_one(db)
    .await
}

/// Kratos answers on the internal network; a request that takes longer is
/// as good as lost.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Request headers a browser's request keeps on its way to Kratos: what it
/// accepts and sends, and the cookies of Kratos' session and CSRF token.
const REQUEST_HEADERS: [HeaderName; 5] = [
    header::ACCEPT,
    // Kratos keeps it with a message, so its mail goes out in the browser's
    // language (#145).
    header::ACCEPT_LANGUAGE,
    header::CONTENT_TYPE,
    header::COOKIE,
    header::USER_AGENT,
];

/// Response headers that go back to the browser.
const RESPONSE_HEADERS: [HeaderName; 4] = [
    header::CONTENT_TYPE,
    header::SET_COOKIE,
    header::LOCATION,
    header::CACHE_CONTROL,
];

#[derive(Debug, Error)]
pub enum KratosError {
    #[error("Kratos is not reachable: {0}")]
    Unreachable(String),
    #[error("Kratos took longer than {0:?}")]
    Timeout(Duration),
    #[error("Kratos answered {status}: {body}")]
    Unexpected { status: StatusCode, body: String },
}

#[derive(Clone)]
pub struct Kratos {
    public_url: String,
    admin_url: String,
    client: Client<HttpConnector, Body>,
}

/// What Kratos says about the session behind a browser's cookies.
#[derive(Debug, Clone)]
pub enum Whoami {
    /// No session, an expired one, or one of a disabled account.
    None,
    /// The account has a second factor this session has not used yet.
    SecondFactorPending,
    Session(KratosSession),
}

/// The Kratos session behind a browser's cookie, as far as remotehub uses it.
#[derive(Debug, Clone, Deserialize)]
pub struct KratosSession {
    pub id: Uuid,
    pub active: bool,
    /// `aal1` after the password, `aal2` after a second factor too.
    pub authenticator_assurance_level: String,
    pub identity: Identity,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Identity {
    pub id: Uuid,
    /// `active` or `inactive`; administrators disable an account this way.
    #[serde(default)]
    pub state: String,
    pub traits: Traits,
}

/// An invited account and the one-time code its owner starts with.
#[derive(Debug, Clone, Deserialize)]
pub struct Invitation {
    #[serde(default)]
    pub identity_id: Uuid,
    /// remotehub's page where the code is entered (`/sign-in/recovery`).
    pub recovery_link: String,
    pub recovery_code: String,
    pub expires_at: String,
}

/// An OpenID Connect provider as the sign-in page offers it (#109).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provider {
    /// The provider's `id` in Kratos' configuration.
    pub id: String,
    /// Its `label` there, e.g. "Microsoft"; the id without one.
    pub label: String,
}

/// The providers a login flow's `oidc` nodes name.
fn providers(flow: &serde_json::Value) -> Vec<Provider> {
    let nodes = flow["ui"]["nodes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    nodes
        .iter()
        .filter(|node| node["group"] == "oidc" && node["attributes"]["name"] == "provider")
        .filter_map(|node| {
            let id = node["attributes"]["value"].as_str()?.to_owned();
            let label = node["meta"]["label"]["context"]["provider"]
                .as_str()
                .unwrap_or(&id)
                .to_owned();
            Some(Provider { id, label })
        })
        .collect()
}

/// What the identity schema (`deploy/ops/kratos/identity.schema.json`) keeps.
#[derive(Debug, Clone, Deserialize)]
pub struct Traits {
    pub email: String,
    #[serde(default)]
    pub name: String,
}

/// A local account's second factors as Kratos keeps them. remotehub checks
/// them itself when someone confirms that it is them (#241): Kratos has no
/// flow that asks for the second factor alone.
#[derive(Debug, Default)]
pub struct SecondFactors {
    pub app: Option<totp::Params>,
    pub keys: Vec<KratosKey>,
}

/// A security key or passkey of a local account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KratosKey {
    pub credential_id: Vec<u8>,
    /// SEC1, uncompressed; keys of other kinds than ES256 are left out.
    pub public_key: Vec<u8>,
    /// The counter as Kratos last saw it.
    pub counter: u32,
}

/// What `GET /admin/identities/{id}?include_credential=…` answers, as far
/// as the second factors go. Go writes byte slices as standard base64.
#[derive(Deserialize)]
struct StoredIdentity {
    #[serde(default)]
    credentials: StoredCredentials,
}

#[derive(Default, Deserialize)]
struct StoredCredentials {
    totp: Option<Stored<StoredTotp>>,
    webauthn: Option<Stored<StoredKeys>>,
}

#[derive(Deserialize)]
struct Stored<T> {
    config: Option<T>,
}

#[derive(Deserialize)]
struct StoredTotp {
    /// `otpauth://totp/…` with the secret.
    totp_url: Option<SecretString>,
}

#[derive(Deserialize)]
struct StoredKeys {
    #[serde(default)]
    credentials: Vec<StoredKey>,
}

#[derive(Deserialize)]
struct StoredKey {
    id: String,
    /// A COSE key.
    public_key: String,
    authenticator: Option<StoredAuthenticator>,
}

#[derive(Deserialize)]
struct StoredAuthenticator {
    #[serde(default)]
    sign_count: u32,
}

impl StoredIdentity {
    fn second_factors(self) -> SecondFactors {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;
        let app = self
            .credentials
            .totp
            .and_then(|totp| totp.config?.totp_url)
            .and_then(|url| totp::Params::parse(url.expose_secret()));
        let keys = self
            .credentials
            .webauthn
            .and_then(|keys| keys.config)
            .map(|config| config.credentials)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|key| {
                Some(KratosKey {
                    credential_id: STANDARD.decode(&key.id).ok()?,
                    public_key: webauthn::cose_es256(&STANDARD.decode(&key.public_key).ok()?)?,
                    counter: key.authenticator.map_or(0, |a| a.sign_count),
                })
            })
            .collect();
        SecondFactors { app, keys }
    }
}

impl std::fmt::Debug for Kratos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kratos")
            .field("public_url", &self.public_url)
            .field("admin_url", &self.admin_url)
            .finish_non_exhaustive()
    }
}

impl Kratos {
    pub fn new(config: &KratosConfig) -> Self {
        Kratos {
            public_url: config.public_url.clone(),
            admin_url: config.admin_url.clone(),
            client: Client::builder(TokioExecutor::new()).build_http(),
        }
    }

    /// Forwards a browser's request to the public API, `path_and_query`
    /// being what follows `/api/auth` in remotehub's URL.
    pub async fn forward(
        &self,
        path_and_query: &str,
        request: Request<Body>,
    ) -> Result<Response<Body>, KratosError> {
        let (parts, body) = request.into_parts();
        let mut outgoing = Request::builder()
            .method(parts.method)
            .uri(format!("{}{path_and_query}", self.public_url));
        for name in REQUEST_HEADERS {
            for value in parts.headers.get_all(&name) {
                outgoing = outgoing.header(&name, value);
            }
        }
        let outgoing = outgoing
            .body(body)
            .map_err(|error| KratosError::Unreachable(error.to_string()))?;
        let answer = self.send(outgoing).await?;
        let (parts, body) = answer.into_parts();
        let mut response = Response::builder().status(parts.status);
        for name in RESPONSE_HEADERS {
            for value in parts.headers.get_all(&name) {
                response = response.header(&name, value);
            }
        }
        response
            .body(Body::new(body))
            .map_err(|error| KratosError::Unreachable(error.to_string()))
    }

    /// The Kratos session behind the browser's cookies.
    pub async fn whoami(&self, headers: &HeaderMap) -> Result<Whoami, KratosError> {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(format!("{}/sessions/whoami", self.public_url))
            .header(header::ACCEPT, "application/json");
        for value in headers.get_all(header::COOKIE) {
            request = request.header(header::COOKIE, value);
        }
        let request = request
            .body(Body::empty())
            .map_err(|error| KratosError::Unreachable(error.to_string()))?;
        let answer = self.send(request).await?;
        match answer.status() {
            StatusCode::OK => json(answer).await.map(Whoami::Session),
            StatusCode::UNAUTHORIZED => Ok(Whoami::None),
            // `session_aal2_required`: `session.whoami.required_aal` is
            // `highest_available` (deploy/ops/kratos/kratos.yml).
            StatusCode::FORBIDDEN => Ok(Whoami::SecondFactorPending),
            status => Err(unexpected(status, answer).await),
        }
    }

    /// The OpenID Connect providers Kratos offers for signing in (#109), in
    /// the order of its configuration. Read from a login flow for apps, which
    /// needs no cookies, so it answers the same whatever the browser holds.
    pub async fn providers(&self) -> Result<Vec<Provider>, KratosError> {
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("{}/self-service/login/api", self.public_url))
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .map_err(|error| KratosError::Unreachable(error.to_string()))?;
        let answer = self.send(request).await?;
        if !answer.status().is_success() {
            return Err(unexpected(answer.status(), answer).await);
        }
        Ok(providers(&json::<serde_json::Value>(answer).await?))
    }

    /// Creates an account for `email` and a one-time code that lets its
    /// owner set a password and a second factor within `valid_for`.
    pub async fn invite(
        &self,
        email: &str,
        name: &str,
        valid_for: Duration,
    ) -> Result<Invitation, KratosError> {
        #[derive(Deserialize)]
        struct Created {
            id: Uuid,
        }
        let created: Created = self
            .admin(
                Method::POST,
                "/admin/identities",
                Some(serde_json::json!({
                    "schema_id": "user",
                    "traits": { "email": email, "name": name },
                })),
            )
            .await?;
        self.recovery_code(created.id, valid_for).await
    }

    /// A one-time code with which the account's owner sets a new password
    /// (and second factor, if it has none) within `valid_for`.
    pub async fn recovery_code(
        &self,
        identity: Uuid,
        valid_for: Duration,
    ) -> Result<Invitation, KratosError> {
        let mut invitation: Invitation = self
            .admin(
                Method::POST,
                "/admin/recovery/code",
                Some(serde_json::json!({
                    "identity_id": identity,
                    "expires_in": format!("{}s", valid_for.as_secs()),
                })),
            )
            .await?;
        invitation.identity_id = identity;
        Ok(invitation)
    }

    /// Disables or enables an account; Kratos refuses the sessions of a
    /// disabled one at once.
    pub async fn set_active(&self, identity: Uuid, active: bool) -> Result<(), KratosError> {
        self.admin::<serde_json::Value>(
            Method::PATCH,
            &format!("/admin/identities/{identity}"),
            Some(serde_json::json!([{
                "op": "replace",
                "path": "/state",
                "value": if active { "active" } else { "inactive" },
            }])),
        )
        .await
        .map(|_| ())
    }

    /// Ends every Kratos session of the account.
    pub async fn revoke_all(&self, identity: Uuid) -> Result<(), KratosError> {
        self.admin::<serde_json::Value>(
            Method::DELETE,
            &format!("/admin/identities/{identity}/sessions"),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Removes the account's second factors, so that a recovery code lets
    /// its owner set up a new one. Kratos keeps a recovery going only
    /// without them.
    pub async fn remove_second_factors(&self, identity: Uuid) -> Result<(), KratosError> {
        for kind in ["totp", "lookup_secret", "webauthn"] {
            let removed = self
                .admin::<serde_json::Value>(
                    Method::DELETE,
                    &format!("/admin/identities/{identity}/credentials/{kind}"),
                    None,
                )
                .await;
            match removed {
                // One the account never had.
                Ok(_)
                | Err(KratosError::Unexpected {
                    status: StatusCode::NOT_FOUND,
                    ..
                }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// The account's authenticator app and keys, for a confirmation (#241).
    pub async fn second_factors(&self, identity: Uuid) -> Result<SecondFactors, KratosError> {
        let answer: StoredIdentity = self
            .admin(
                Method::GET,
                &format!(
                    "/admin/identities/{identity}?include_credential=totp&include_credential=webauthn"
                ),
                None,
            )
            .await?;
        Ok(answer.second_factors())
    }

    /// Deletes the account in Kratos.
    pub async fn delete(&self, identity: Uuid) -> Result<(), KratosError> {
        self.admin::<serde_json::Value>(
            Method::DELETE,
            &format!("/admin/identities/{identity}"),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Ends one Kratos session, as signing out of remotehub does.
    pub async fn revoke(&self, session: Uuid) -> Result<(), KratosError> {
        self.admin::<serde_json::Value>(Method::DELETE, &format!("/admin/sessions/{session}"), None)
            .await
            .map(|_| ())
    }

    /// A request to the admin API with a JSON body, answered with JSON
    /// (`null` for an answer without a body).
    pub async fn admin<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, KratosError> {
        let request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.admin_url))
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .map_err(|error| KratosError::Unreachable(error.to_string()))?;
        let answer = self.send(request).await?;
        match answer.status() {
            StatusCode::NO_CONTENT => {
                serde_json::from_value(serde_json::Value::Null).map_err(|e| {
                    KratosError::Unexpected {
                        status: StatusCode::NO_CONTENT,
                        body: e.to_string(),
                    }
                })
            }
            status if status.is_success() => json(answer).await,
            status => Err(unexpected(status, answer).await),
        }
    }

    async fn send(
        &self,
        request: Request<Body>,
    ) -> Result<Response<hyper::body::Incoming>, KratosError> {
        tokio::time::timeout(TIMEOUT, self.client.request(request))
            .await
            .map_err(|_| KratosError::Timeout(TIMEOUT))?
            .map_err(|error| KratosError::Unreachable(error.to_string()))
    }
}

async fn bytes(answer: Response<hyper::body::Incoming>) -> Result<Vec<u8>, KratosError> {
    use http_body_util::BodyExt;
    answer
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes().to_vec())
        .map_err(|error| KratosError::Unreachable(error.to_string()))
}

async fn json<T: DeserializeOwned>(
    answer: Response<hyper::body::Incoming>,
) -> Result<T, KratosError> {
    let status = answer.status();
    let body = bytes(answer).await?;
    serde_json::from_slice(&body).map_err(|error| KratosError::Unexpected {
        status,
        body: format!("not what remotehub expects: {error}"),
    })
}

async fn unexpected(status: StatusCode, answer: Response<hyper::body::Incoming>) -> KratosError {
    let body = bytes(answer).await.unwrap_or_default();
    KratosError::Unexpected {
        status,
        // Kratos' error JSON, cut short: enough for the log, never a secret.
        body: String::from_utf8_lossy(&body).chars().take(300).collect(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_the_providers_from_a_login_flow() {
        // As Kratos v26.2 answers GET /self-service/login/api.
        let flow = json!({ "ui": { "nodes": [
            { "group": "default", "attributes": { "name": "identifier", "value": "" } },
            { "group": "password", "attributes": { "name": "method", "value": "password" } },
            { "group": "oidc", "attributes": { "name": "provider", "value": "lab" },
              "meta": { "label": { "id": 1010002, "text": "Sign in with Lab",
                                   "context": { "provider": "Lab", "provider_id": "lab" } } } },
            { "group": "oidc", "attributes": { "name": "provider", "value": "github" },
              "meta": {} },
        ] } });
        assert_eq!(
            providers(&flow),
            [
                Provider {
                    id: "lab".into(),
                    label: "Lab".into()
                },
                Provider {
                    id: "github".into(),
                    label: "github".into()
                },
            ]
        );
        assert_eq!(providers(&json!({})), []);
    }

    #[test]
    fn reads_the_second_factors_of_an_identity() {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;
        // A P-256 key in COSE form: kty 2, alg -7, crv 1, x, y.
        let key = p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();
        let point = key.verifying_key().to_sec1_point(false);
        let mut cose = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        cose.extend(&point.as_bytes()[1..33]);
        cose.extend([0x22, 0x58, 0x20]);
        cose.extend(&point.as_bytes()[33..]);
        // As Kratos v26.2 answers the admin API, shortened.
        let identity: StoredIdentity = serde_json::from_value(json!({
            "id": "8f0e8a4e-6a4b-4d52-9d5a-0d7c2b1e4f10",
            "credentials": {
                "password": { "type": "password", "config": { "hashed_password": "$argon2id$…" } },
                "totp": { "type": "totp", "config": {
                    "totp_url": "otpauth://totp/remotehub:ada@example.com?algorithm=SHA1&digits=6&issuer=remotehub&period=30&secret=JBSWY3DPEHPK3PXP",
                } },
                "webauthn": { "type": "webauthn", "config": {
                    "credentials": [
                        { "id": STANDARD.encode([1, 2, 3]), "public_key": STANDARD.encode(&cose),
                          "attestation_type": "none", "display_name": "Laptop",
                          "authenticator": { "aaguid": "AAAAAAAAAAAAAAAAAAAAAA==", "sign_count": 4, "clone_warning": false },
                          "is_passwordless": false },
                        { "id": STANDARD.encode([4]), "public_key": STANDARD.encode([0xa1, 0x01, 0x01]) },
                    ],
                    "user_handle": "AQID",
                } },
            },
        }))
        .unwrap();
        let factors = identity.second_factors();
        assert_eq!(
            factors.app.unwrap().secret.as_slice(),
            b"Hello!\xde\xad\xbe\xef"
        );
        assert_eq!(
            factors.keys,
            [KratosKey {
                credential_id: vec![1, 2, 3],
                public_key: point.as_bytes().to_vec(),
                counter: 4,
            }]
        );

        let none: StoredIdentity = serde_json::from_value(json!({ "id": "x" })).unwrap();
        let none = none.second_factors();
        assert!(none.app.is_none() && none.keys.is_empty());
    }
}
