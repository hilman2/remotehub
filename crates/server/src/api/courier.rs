//! `POST /courier`, on a port of its own that only the compose network
//! reaches (#145): Kratos hands over the mails it would send, and remotehub
//! sends them through the mail server of its settings, in the language of
//! the request that started the flow.
//!
//! Kratos' HTTP courier (`courier.delivery_strategy: http`) posts what
//! `deploy/ops/kratos/courier.jsonnet` makes of each message, with the token
//! from `REMOTEHUB_COURIER_TOKEN` as bearer. An answer other than 2xx makes
//! Kratos try again later (`courier.message_retries`).

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::post;
use remotehub_i18n::{Locale, Message};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::mail::{self, Outgoing, Step};

/// A message of Kratos, as `courier.jsonnet` shapes it.
#[derive(Debug, Deserialize)]
pub struct Delivery {
    recipient: String,
    template_type: String,
    #[serde(default)]
    template_data: Value,
    /// Headers of the request that started the flow, each as one value or
    /// a list.
    #[serde(default)]
    request_headers: HashMap<String, Value>,
    /// Kratos' own rendering, in English, for messages remotehub has no
    /// text of its own for.
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
}

impl Delivery {
    /// The language the person asked in, from `Accept-Language`.
    fn locale(&self) -> Locale {
        let header = self
            .request_headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("accept-language"))
            .map(|(_, value)| match value {
                Value::String(text) => text.clone(),
                Value::Array(values) => values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(","),
                _ => String::new(),
            })
            .unwrap_or_default();
        Locale::from_accept_language(&header)
    }

    /// The mail to send, or none for a message without text.
    fn outgoing(&self) -> Option<Outgoing> {
        let locale = self.locale();
        match (
            self.template_type.as_str(),
            self.template_data["recovery_code"].as_str(),
        ) {
            ("recovery_code_valid", Some(code)) => Some(Outgoing::new(
                &self.recipient,
                locale,
                &Message::MailRecoverySubject {},
                &Message::MailRecoveryBody {
                    code: code.to_owned(),
                },
            )),
            _ if self.subject.is_empty() => None,
            _ => Some(Outgoing {
                to: self.recipient.clone(),
                subject: self.subject.clone(),
                text: self.body.clone(),
            }),
        }
    }
}

/// The router of the courier's port; `token` is what Kratos must present.
pub fn router(state: AppState, token: SecretString) -> Router {
    let expected: [u8; 32] = Sha256::digest(format!("Bearer {}", token.expose_secret())).into();
    Router::new()
        .route("/courier", post(deliver))
        .with_state((state, Arc::new(expected)))
}

async fn deliver(
    State((state, expected)): State<(AppState, Arc<[u8; 32]>)>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // The token first: without it, the body is not even read. Comparing
    // hashes keeps the time the same whatever the token shares.
    let presented: [u8; 32] = Sha256::digest(
        headers
            .get(header::AUTHORIZATION)
            .map(|v| v.as_bytes())
            .unwrap_or_default(),
    )
    .into();
    if presented != *expected {
        tracing::warn!("courier request without the right token");
        return StatusCode::UNAUTHORIZED;
    }
    let delivery: Delivery = match serde_json::from_slice(&body) {
        Ok(delivery) => delivery,
        Err(error) => {
            tracing::warn!(%error, "courier message Kratos sent is unreadable");
            return StatusCode::BAD_REQUEST;
        }
    };
    let Some(outgoing) = delivery.outgoing() else {
        tracing::info!(template = %delivery.template_type, "courier message without text, dropped");
        return StatusCode::NO_CONTENT;
    };
    match mail::send(&state.db, &state.vault, outgoing).await {
        Ok(()) => {
            tracing::info!(template = %delivery.template_type, "mail sent for Kratos");
            StatusCode::NO_CONTENT
        }
        Err(failure) => {
            tracing::warn!(template = %delivery.template_type, step = ?failure.step, detail = %failure.detail, "mail for Kratos failed");
            if failure.step == Step::NotConfigured {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::BAD_GATEWAY
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn delivery(value: Value) -> Delivery {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_recovery_code_goes_out_in_the_asked_language() {
        let german = delivery(json!({
            "recipient": "ada@example.com",
            "template_type": "recovery_code_valid",
            "template_data": { "recovery_code": "123456" },
            "request_headers": { "Accept-Language": ["de-DE,de;q=0.9,en;q=0.8"] },
            "subject": "Recover access to your account",
            "body": "Hi, …",
        }))
        .outgoing()
        .unwrap();
        assert_eq!(german.to, "ada@example.com");
        assert_eq!(german.subject, "Code für remotehub");
        assert!(german.text.contains("123456"));

        let english = delivery(json!({
            "recipient": "ada@example.com",
            "template_type": "recovery_code_valid",
            "template_data": { "recovery_code": "123456" },
            "request_headers": { "accept-language": "fr" },
        }))
        .outgoing()
        .unwrap();
        assert_eq!(english.subject, "Code for remotehub");
    }

    #[test]
    fn other_messages_keep_the_text_of_kratos() {
        let other = delivery(json!({
            "recipient": "ada@example.com",
            "template_type": "verification_code_valid",
            "subject": "Please verify",
            "body": "Code 42",
        }))
        .outgoing()
        .unwrap();
        assert_eq!(
            (other.subject.as_str(), other.text.as_str()),
            ("Please verify", "Code 42")
        );
        let empty = delivery(json!({ "recipient": "x@example.com", "template_type": "x" }));
        assert!(empty.outgoing().is_none());
    }
}
