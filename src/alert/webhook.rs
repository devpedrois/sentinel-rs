use std::time::Duration;

use reqwest::{Client, Url};
use serde::Serialize;
use tokio::time::sleep;
use tracing::warn;

use super::{Alert, AlertDispatcher, AlertType, DispatchError};

const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);
const WEBHOOK_MAX_ATTEMPTS: usize = 3;
const WEBHOOK_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum WebhookRequestBody {
    Slack { text: String },
    Discord { content: String },
    Generic(Alert),
}

impl WebhookRequestBody {
    fn from_alert(url: &Url, alert: &Alert) -> Self {
        let message = format_alert_message(alert);

        if is_slack_webhook(url) {
            return Self::Slack { text: message };
        }

        if is_discord_webhook(url) {
            return Self::Discord { content: message };
        }

        Self::Generic(alert.clone())
    }
}

pub struct WebhookDispatcher {
    client: Client,
    url: Url,
}

impl WebhookDispatcher {
    pub fn new(url: Url) -> Self {
        Self {
            client: Client::new(),
            url,
        }
    }
}

impl AlertDispatcher for WebhookDispatcher {
    async fn dispatch(&self, alert: &Alert) -> Result<(), DispatchError> {
        let payload = WebhookRequestBody::from_alert(&self.url, alert);
        let redacted_url = redact_webhook_url(&self.url);

        for attempt in 1..=WEBHOOK_MAX_ATTEMPTS {
            match self
                .client
                .post(self.url.as_str())
                .timeout(WEBHOOK_TIMEOUT)
                .json(&payload)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    let status = response.status();

                    if attempt < WEBHOOK_MAX_ATTEMPTS && is_retryable_status(status) {
                        warn!(
                            attempt = attempt,
                            status = %status,
                            endpoint = %redacted_url,
                            "Retrying webhook after transient status"
                        );
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }

                    warn!(
                        status = %status,
                        endpoint = %redacted_url,
                        "Webhook returned non-success status"
                    );
                    return Err(DispatchError::UnexpectedStatus(status));
                }
                Err(err) => {
                    let sanitized = sanitize_reqwest_error(err);

                    if attempt < WEBHOOK_MAX_ATTEMPTS && is_retryable_error(&sanitized) {
                        warn!(
                            attempt = attempt,
                            error = %sanitized,
                            endpoint = %redacted_url,
                            "Retrying webhook after transient transport failure"
                        );
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }

                    return Err(DispatchError::Http(sanitized));
                }
            }
        }

        unreachable!("webhook retry loop should return on success or terminal failure")
    }
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request()
}

fn retry_delay(attempt: usize) -> Duration {
    WEBHOOK_RETRY_DELAY.saturating_mul(attempt as u32)
}

fn sanitize_reqwest_error(error: reqwest::Error) -> reqwest::Error {
    error.without_url()
}

fn is_slack_webhook(url: &Url) -> bool {
    matches!(url.host_str(), Some("hooks.slack.com"))
}

fn is_discord_webhook(url: &Url) -> bool {
    matches!(url.host_str(), Some("discord.com" | "discordapp.com"))
        && url.path().contains("/api/webhooks/")
}

fn redact_webhook_url(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{}://{}:{port}/...", url.scheme(), host),
        (Some(host), None) => format!("{}://{host}/...", url.scheme()),
        (None, _) => "<redacted-webhook-url>".to_string(),
    }
}

fn format_alert_message(alert: &Alert) -> String {
    match alert.alert_type {
        AlertType::Cpu => format!(
            "Sentinel alert: CPU at {:.1}% exceeded {:.1}% for {} consecutive readings at {}.",
            alert.current_value, alert.threshold, alert.consecutive_readings, alert.timestamp
        ),
        AlertType::Ram => format!(
            "Sentinel alert: RAM at {:.1}% exceeded {:.1}% for {} consecutive readings at {}.",
            alert.current_value, alert.threshold, alert.consecutive_readings, alert.timestamp
        ),
        AlertType::Collector => format!(
            "Sentinel alert: collector failed {} time(s) as of {}. {}",
            alert.consecutive_readings,
            alert.timestamp,
            alert
                .message
                .as_deref()
                .unwrap_or("system metric collection failed")
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use reqwest::Url;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use crate::alert::{Alert, AlertDispatcher, AlertType};

    use super::{redact_webhook_url, WebhookDispatcher, WebhookRequestBody};

    fn sample_alert() -> Alert {
        Alert {
            alert_type: AlertType::Cpu,
            current_value: 97.5,
            threshold: 90.0,
            consecutive_readings: 3,
            timestamp: "2026-04-28T00:00:00Z".to_string(),
            message: None,
        }
    }

    async fn spawn_status_server(status_line: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("listener should expose addr");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept should succeed");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            stream
                .write_all(
                    format!("{status_line}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .expect("response should be written");
        });

        addr
    }

    async fn spawn_sequenced_status_server(
        responses: Vec<&'static str>,
        requests: Arc<AtomicUsize>,
    ) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("listener should expose addr");

        tokio::spawn(async move {
            for status_line in responses {
                let (mut stream, _) = listener.accept().await.expect("accept should succeed");
                requests.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer).await;
                stream
                    .write_all(
                        format!("{status_line}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                            .as_bytes(),
                    )
                    .await
                    .expect("response should be written");
            }
        });

        addr
    }

    #[tokio::test]
    async fn returns_error_when_webhook_status_is_not_success() {
        let addr = spawn_status_server("HTTP/1.1 500 Internal Server Error").await;
        let dispatcher = WebhookDispatcher::new(
            Url::parse(&format!("http://{addr}/alert")).expect("URL should be valid"),
        );

        let result = dispatcher.dispatch(&sample_alert()).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retries_transient_status_failures_until_success() {
        let requests = Arc::new(AtomicUsize::new(0));
        let addr = spawn_sequenced_status_server(
            vec![
                "HTTP/1.1 503 Service Unavailable",
                "HTTP/1.1 429 Too Many Requests",
                "HTTP/1.1 204 No Content",
            ],
            Arc::clone(&requests),
        )
        .await;
        let dispatcher = WebhookDispatcher::new(
            Url::parse(&format!("http://{addr}/alert")).expect("URL should be valid"),
        );

        let result = dispatcher.dispatch(&sample_alert()).await;

        assert!(result.is_ok());
        assert_eq!(requests.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn transport_errors_do_not_expose_webhook_secrets() {
        let dispatcher = WebhookDispatcher::new(
            Url::parse("http://127.0.0.1:1/discord/very-secret-token")
                .expect("URL should be valid"),
        );

        let result = dispatcher.dispatch(&sample_alert()).await;

        let error_text = format!(
            "{}",
            result.expect_err("dispatch should fail against a closed port")
        );
        assert!(
            !error_text.contains("very-secret-token"),
            "transport errors must not expose secret webhook segments"
        );
    }

    #[test]
    fn builds_slack_compatible_payload_for_slack_webhooks() {
        let url = Url::parse("https://hooks.slack.com/services/T000/B000/secret")
            .expect("URL should be valid");

        let payload = WebhookRequestBody::from_alert(&url, &sample_alert());

        assert_eq!(
            serde_json::to_value(&payload).expect("payload should serialize"),
            json!({
                "text": "Sentinel alert: CPU at 97.5% exceeded 90.0% for 3 consecutive readings at 2026-04-28T00:00:00Z."
            })
        );
    }

    #[test]
    fn builds_discord_compatible_payload_for_discord_webhooks() {
        let url =
            Url::parse("https://discord.com/api/webhooks/123/secret").expect("URL should be valid");

        let payload = WebhookRequestBody::from_alert(&url, &sample_alert());

        assert_eq!(
            serde_json::to_value(&payload).expect("payload should serialize"),
            json!({
                "content": "Sentinel alert: CPU at 97.5% exceeded 90.0% for 3 consecutive readings at 2026-04-28T00:00:00Z."
            })
        );
    }

    #[test]
    fn keeps_structured_payload_for_generic_webhooks() {
        let url = Url::parse("https://example.com/hooks/sentinel").expect("URL should be valid");

        let payload = WebhookRequestBody::from_alert(&url, &sample_alert());

        assert_eq!(
            serde_json::to_value(&payload).expect("payload should serialize"),
            json!({
                "alert_type": "Cpu",
                "current_value": 97.5,
                "threshold": 90.0,
                "consecutive_readings": 3,
                "timestamp": "2026-04-28T00:00:00Z",
                "message": null
            })
        );
    }

    #[test]
    fn redacts_secret_segments_from_logged_webhook_urls() {
        let url = Url::parse("https://discord.com/api/webhooks/123/very-secret-token")
            .expect("URL should be valid");

        let redacted = redact_webhook_url(&url);

        assert_eq!(redacted, "https://discord.com/...");
        assert!(
            !redacted.contains("very-secret-token"),
            "redacted URL must not expose secret path segments"
        );
    }
}
