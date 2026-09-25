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
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use uuid::Uuid;

use crate::config::KratosConfig;

/// Kratos answers on the internal network; a request that takes longer is
/// as good as lost.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Request headers a browser's request keeps on its way to Kratos: what it
/// accepts and sends, and the cookies of Kratos' session and CSRF token.
const REQUEST_HEADERS: [HeaderName; 4] = [
    header::ACCEPT,
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

/// What the identity schema (`deploy/ops/kratos/identity.schema.json`) keeps.
#[derive(Debug, Clone, Deserialize)]
pub struct Traits {
    pub email: String,
    #[serde(default)]
    pub name: String,
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
        let mut invitation: Invitation = self
            .admin(
                Method::POST,
                "/admin/recovery/code",
                Some(serde_json::json!({
                    "identity_id": created.id,
                    "expires_in": format!("{}s", valid_for.as_secs()),
                })),
            )
            .await?;
        invitation.identity_id = created.id;
        Ok(invitation)
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
