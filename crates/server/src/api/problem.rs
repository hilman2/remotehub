//! API errors as RFC 9457 problem responses.
//!
//! Every error carries a stable [`ErrorCode`]. The UI turns the code into
//! text with its message `error_<code>` (ADR 0002); the English `title` is
//! only for API consumers and logs and is never shown in the UI.

use std::collections::BTreeMap;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Defines the error codes once: variant, wire name, HTTP status and English
/// title. The macro also lists every variant in [`ErrorCode::ALL`], so the
/// generated TypeScript and the translation guards can never miss one.
macro_rules! error_codes {
    ($($variant:ident = ($code:literal, $status:ident, $title:literal)),+ $(,)?) => {
        /// Stable error codes of the API; never rename one, add a new one instead.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        pub enum ErrorCode {
            $(#[serde(rename = $code)] $variant,)+
        }

        impl ErrorCode {
            pub const ALL: &[ErrorCode] = &[$(ErrorCode::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(ErrorCode::$variant => $code,)+ }
            }

            pub fn status(self) -> StatusCode {
                match self { $(ErrorCode::$variant => StatusCode::$status,)+ }
            }

            pub fn title(self) -> &'static str {
                match self { $(ErrorCode::$variant => $title,)+ }
            }
        }
    };
}

error_codes! {
    NotFound = ("not_found", NOT_FOUND, "Not found"),
    InvalidRequest = ("invalid_request", BAD_REQUEST, "Invalid request"),
    Unauthenticated = ("unauthenticated", UNAUTHORIZED, "Not signed in"),
    Forbidden = ("forbidden", FORBIDDEN, "Not permitted"),
    InvalidCredentials = ("invalid_credentials", UNAUTHORIZED, "Invalid user name or password"),
    AccountDisabled = ("account_disabled", FORBIDDEN, "Account disabled"),
    AccountLocked = ("account_locked", FORBIDDEN, "Account locked out"),
    AccountExpired = ("account_expired", FORBIDDEN, "Account expired"),
    PasswordChangeRequired = ("password_change_required", FORBIDDEN, "Password must be changed"),
    TooManyAttempts = ("too_many_attempts", TOO_MANY_REQUESTS, "Too many failed sign-ins"),
    ForbiddenOrigin = ("forbidden_origin", FORBIDDEN, "Request from a foreign origin"),
    NameTaken = ("name_taken", CONFLICT, "The name is already taken here"),
    FolderNotEmpty = ("folder_not_empty", CONFLICT, "The folder is not empty"),
    HostKeyChanged = ("host_key_changed", CONFLICT, "The target's host key changed"),
    CertificateChanged = ("certificate_changed", CONFLICT, "The target's certificate changed"),
    TlsRequired = ("tls_required", BAD_GATEWAY, "The target offers RDP only without TLS"),
    TargetUnreachable = ("target_unreachable", BAD_GATEWAY, "The target is not reachable"),
    TargetAuthFailed = ("target_auth_failed", BAD_GATEWAY, "The target refused the credentials"),
    ConnectionFailed = ("connection_failed", BAD_GATEWAY, "The connection failed"),
    OwnAccountUnavailable = ("own_account_unavailable", CONFLICT, "The own sign-in password is not available in this session"),
    SshCaUnavailable = ("ssh_ca_unavailable", CONFLICT, "No SSH certificate authority is configured"),
    LapsUnavailable = ("laps_unavailable", CONFLICT, "No readable LAPS password for this device"),
    RequestPending = ("request_pending", CONFLICT, "The same request is already waiting for a decision"),
    RequestDecided = ("request_decided", CONFLICT, "The request has been decided already"),
    OwnRequest = ("own_request", FORBIDDEN, "Nobody approves their own request"),
    LastUnlock = ("last_unlock", CONFLICT, "The last way to unlock the personal vault cannot be removed"),
    VaultNotCovered = ("vault_not_covered", CONFLICT, "The vault is not wrapped for the organisation recovery key"),
    RecoveryKeyInUse = ("recovery_key_in_use", CONFLICT, "Vaults still depend on this recovery key"),
    RecoveryOpen = ("recovery_open", CONFLICT, "A recovery of this vault is open already"),
    RecoveryNotReady = ("recovery_not_ready", CONFLICT, "The recovery is not approved, or its day has passed"),
    ConnectorInUse = ("connector_in_use", CONFLICT, "Devices are still reached through this connector"),
    ConnectorOffline = ("connector_offline", BAD_GATEWAY, "The device's connector is not connected"),
    PurposeRequired = ("purpose_required", BAD_REQUEST, "State the purpose of the connection"),
    AccountsUnavailable = ("accounts_unavailable", SERVICE_UNAVAILABLE, "The service for local accounts is not reachable"),
    SecondFactorRequired = ("second_factor_required", FORBIDDEN, "Confirm the sign-in with the second factor"),
    SecondFactorSetupRequired = ("second_factor_setup_required", FORBIDDEN, "Set up a second factor first"),
    SecondFactorInvalid = ("second_factor_invalid", UNAUTHORIZED, "The code is not right"),
    SetupPending = ("setup_pending", CONFLICT, "remotehub is not set up yet"),
    SetupCodeInvalid = ("setup_code_invalid", FORBIDDEN, "The setup code is not valid"),
    SetupDone = ("setup_done", CONFLICT, "remotehub is set up already"),
    BreakGlassExists = ("break_glass_exists", CONFLICT, "A break-glass account exists already"),
    LastAdministrator = ("last_administrator", CONFLICT, "The last administrator keeps the role"),
    DirectoryUnavailable = ("directory_unavailable", SERVICE_UNAVAILABLE, "Directory unavailable"),
    DatabaseUnavailable = ("database_unavailable", SERVICE_UNAVAILABLE, "Database unavailable"),
    Internal = ("internal", INTERNAL_SERVER_ERROR, "Internal server error"),
}

/// A problem response: `application/problem+json` with the code and optional
/// parameters for the UI's message.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: &'static str,
    pub status: u16,
    pub code: ErrorCode,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
}

impl Problem {
    pub fn new(code: ErrorCode) -> Self {
        Problem {
            kind: format!("urn:remotehub:error:{}", code.as_str()),
            title: code.title(),
            status: code.status().as_u16(),
            code,
            params: BTreeMap::new(),
        }
    }

    /// Adds a parameter the UI message can use, e.g. `{ name }`.
    pub fn param(mut self, name: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}

impl From<ErrorCode> for Problem {
    fn from(code: ErrorCode) -> Self {
        Problem::new(code)
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = self.code.status();
        let mut response = (status, Json(self)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

/// Database errors never reach the client in detail: they are logged here
/// and become `database_unavailable` or `internal`.
impl From<sqlx::Error> for Problem {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
                tracing::warn!(%error, "database unavailable");
                Problem::new(ErrorCode::DatabaseUnavailable)
            }
            error => {
                tracing::error!(%error, "database error");
                Problem::new(ErrorCode::Internal)
            }
        }
    }
}

/// TypeScript for the UI: the list of codes and the problem shape. Checked
/// in at `web/src/lib/api/generated/problem.ts`; a test fails when it is stale.
pub fn typescript() -> String {
    let codes = ErrorCode::ALL
        .iter()
        .map(|code| format!("\t'{}'", code.as_str()))
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "// Generated from crates/server/src/api/problem.rs — do not edit.\n\
         // Regenerate: REMOTEHUB_BLESS=1 cargo nextest run -p remotehub-server generated\n\
         \n\
         export const ERROR_CODES = [\n{codes}\n] as const;\n\
         \n\
         export type ErrorCode = (typeof ERROR_CODES)[number];\n\
         \n\
         /** RFC 9457 problem response of the API. */\n\
         export interface Problem {{\n\
         \ttype: string;\n\
         \ttitle: string;\n\
         \tstatus: number;\n\
         \tcode: ErrorCode;\n\
         \tparams?: Record<string, unknown>;\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn codes_are_unique_snake_case_and_match_their_serde_name() {
        let mut seen = HashSet::new();
        for code in ErrorCode::ALL {
            let name = code.as_str();
            assert!(seen.insert(name), "duplicate code {name}");
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name} is not snake_case"
            );
            assert_eq!(serde_json::to_value(code).unwrap(), name);
            assert!(code.status().is_client_error() || code.status().is_server_error());
        }
    }

    #[test]
    fn serialises_as_rfc_9457_problem() {
        let problem = Problem::new(ErrorCode::NotFound).param("path", "/api/x");
        assert_eq!(
            serde_json::to_value(&problem).unwrap(),
            serde_json::json!({
                "type": "urn:remotehub:error:not_found",
                "title": "Not found",
                "status": 404,
                "code": "not_found",
                "params": { "path": "/api/x" }
            })
        );
        let plain = serde_json::to_value(Problem::new(ErrorCode::Internal)).unwrap();
        assert!(plain.get("params").is_none());
    }

    #[test]
    fn responds_with_the_problem_content_type_and_status() {
        let response = Problem::new(ErrorCode::DatabaseUnavailable).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
    }

    #[test]
    fn hides_database_details() {
        let problem = Problem::from(sqlx::Error::RowNotFound);
        assert_eq!(problem.code, ErrorCode::Internal);
        assert!(problem.params.is_empty());
        assert_eq!(
            Problem::from(sqlx::Error::PoolTimedOut).code,
            ErrorCode::DatabaseUnavailable
        );
    }
}
