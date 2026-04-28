use std::time::Duration;

use reqwest::{Client, Url};
use tokio::time::sleep;
use tracing::warn;

use super::{Alert, AlertDispatcher, DispatchError};

const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);
const WEBHOOK_MAX_ATTEMPTS: usize = 3;
const WEBHOOK_RETRY_DELAY: Duration = Duration::from_millis(250);

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
        for attempt in 1..=WEBHOOK_MAX_ATTEMPTS {
            match self
                .client
                .post(self.url.as_str())
                .timeout(WEBHOOK_TIMEOUT)
                .json(alert)
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
                            url = %self.url,
                            "Retrying webhook after transient status"
                        );
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }

                    warn!(
                        status = %status,
                        url = %self.url,
                        "Webhook returned non-success status"
                    );
                    return Err(DispatchError::UnexpectedStatus(status));
                }
                Err(err) => {
                    if attempt < WEBHOOK_MAX_ATTEMPTS && is_retryable_error(&err) {
                        warn!(
                            attempt = attempt,
                            error = %err,
                            url = %self.url,
                            "Retrying webhook after transient transport failure"
                        );
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }

                    return Err(DispatchError::Http(err));
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
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use crate::alert::{Alert, AlertDispatcher, AlertType};

    use super::WebhookDispatcher;

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
}
