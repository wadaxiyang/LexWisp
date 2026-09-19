use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Duration,
};

use futures_util::StreamExt as _;
use lexwisp_core::{AiMessage, AiRole, ProviderConfig, ProviderError};
use reqwest::{Client, Proxy, StatusCode};
use serde::Serialize;
use serde_json::Value;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use url::Url;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct AiService {
    clients: Arc<RwLock<HashMap<String, Client>>>,
}

#[derive(Serialize)]
struct RequestMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<RequestMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

impl AiService {
    pub fn new() -> Result<Self, ProviderError> {
        Ok(Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn client_for(&self, provider: &ProviderConfig) -> Result<Client, ProviderError> {
        let key = format!(
            "{}|{}",
            provider.proxy_url().unwrap_or_default(),
            provider.connect_timeout_seconds()
        );
        if let Some(client) = self
            .clients
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .cloned()
        {
            return Ok(client);
        }
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(provider.connect_timeout_seconds()))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > 5 {
                    return attempt.error("too many redirects");
                }
                if let Some(previous) = attempt.previous().last()
                    && !same_origin(previous, attempt.url())
                {
                    return attempt.error("cross-origin redirects are not allowed");
                }
                attempt.follow()
            }));
        if let Some(proxy) = provider.proxy_url() {
            builder = builder.proxy(Proxy::all(proxy).map_err(|error| {
                ProviderError::InvalidConfiguration(format!("proxy URL is invalid: {error}"))
            })?);
        }
        let client = builder
            .build()
            .map_err(|error| ProviderError::Http(error.to_string()))?;
        self.clients
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, client.clone());
        Ok(client)
    }

    pub async fn chat(
        &self,
        provider: &ProviderConfig,
        model_id: &str,
        credential: Option<&str>,
        messages: &[AiMessage],
        cancellation: &CancellationToken,
        mut on_delta: impl FnMut(String) -> Result<(), ProviderError>,
    ) -> Result<String, ProviderError> {
        let endpoint = completion_endpoint(provider.base_url())?;
        let request_messages = messages
            .iter()
            .map(|message| RequestMessage {
                role: match message.role {
                    AiRole::System => "system",
                    AiRole::User => "user",
                    AiRole::Assistant => "assistant",
                },
                content: &message.content,
            })
            .collect();
        let body = ChatRequest {
            model: model_id,
            messages: request_messages,
            stream: provider.stream(),
            temperature: provider
                .temperature_milli()
                .map(|value| f64::from(value) / 1_000.0),
            max_tokens: provider.max_output_tokens(),
        };
        let mut request = self
            .client_for(provider)?
            .post(endpoint)
            .timeout(Duration::from_secs(provider.total_timeout_seconds()))
            .json(&body);
        if let Some(credential) = credential {
            request = request.bearer_auth(credential);
        }
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            response = request.send() => response.map_err(map_request_error)?,
        };
        if !response.status().is_success() {
            return Err(http_status_error(&response));
        }
        if provider.stream() {
            read_stream(
                response,
                cancellation,
                &mut on_delta,
                Duration::from_secs(provider.event_timeout_seconds()),
            )
            .await
        } else {
            let bytes = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                bytes = response.bytes() => bytes.map_err(map_request_error)?,
            };
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err(ProviderError::Protocol("response exceeded 2 MiB".into()));
            }
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|error| ProviderError::Protocol(error.to_string()))?;
            let content = response_content(&value)?;
            on_delta(content.clone())?;
            Ok(content)
        }
    }
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

async fn read_stream(
    response: reqwest::Response,
    cancellation: &CancellationToken,
    on_delta: &mut impl FnMut(String) -> Result<(), ProviderError>,
    event_timeout: Duration,
) -> Result<String, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut decoder = SseDecoder::default();
    let mut output = String::new();
    let mut observed_completion = false;
    let mut total_bytes = 0_usize;
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            next = timeout(event_timeout, stream.next()) => {
                next.map_err(|_| ProviderError::Timeout(if output.is_empty() { "the first event" } else { "the next stream event" }))?
            }
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(map_request_error)?;
        total_bytes = total_bytes.saturating_add(chunk.len());
        if total_bytes > MAX_RESPONSE_BYTES {
            return Err(ProviderError::Protocol("stream exceeded 2 MiB".into()));
        }
        for data in decoder.push(&chunk)? {
            if data.trim() == "[DONE]" {
                observed_completion = true;
                continue;
            }
            let value: Value = serde_json::from_str(&data)
                .map_err(|error| ProviderError::Protocol(error.to_string()))?;
            if let Some(choices) = value.get("choices").and_then(Value::as_array) {
                if choices.is_empty() {
                    continue;
                }
                for choice in choices {
                    if choice
                        .get("finish_reason")
                        .is_some_and(|reason| !reason.is_null())
                    {
                        observed_completion = true;
                    }
                    if let Some(delta) = choice
                        .get("delta")
                        .and_then(|delta| delta.get("content"))
                        .and_then(Value::as_str)
                    {
                        output.push_str(delta);
                        on_delta(delta.to_owned())?;
                    }
                }
            }
        }
    }
    decoder.finish()?;
    if !observed_completion {
        return Err(ProviderError::Protocol(
            "stream ended before [DONE] or a finish reason; partial text was preserved".into(),
        ));
    }
    Ok(output)
}

fn http_status_error(response: &reqwest::Response) -> ProviderError {
    let status = response.status();
    let category = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "authentication failed",
        StatusCode::NOT_FOUND => "endpoint or model was not found",
        StatusCode::TOO_MANY_REQUESTS => "rate limit exceeded",
        status if status.is_server_error() => "provider server error",
        _ => "HTTP request failed",
    };
    ProviderError::Http(format!("{category} ({status})"))
}

fn map_request_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout("the request")
    } else {
        ProviderError::Http(error.to_string())
    }
}

pub(crate) fn completion_endpoint(base_url: &str) -> Result<Url, ProviderError> {
    let mut url = Url::parse(base_url)
        .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?;
    if url.username() != "" || url.password().is_some() || url.fragment().is_some() {
        return Err(ProviderError::InvalidConfiguration(
            "Base URL cannot contain credentials or a fragment".into(),
        ));
    }
    let is_loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    match url.scheme() {
        "https" => {}
        "http" if is_loopback => {}
        "http" => {
            return Err(ProviderError::InvalidConfiguration(
                "plain HTTP is allowed only for loopback providers".into(),
            ));
        }
        _ => {
            return Err(ProviderError::InvalidConfiguration(
                "Base URL must use HTTPS or loopback HTTP".into(),
            ));
        }
    }
    if url.path().ends_with("/chat/completions") {
        return Err(ProviderError::InvalidConfiguration(
            "enter the API base path, not the full /chat/completions endpoint".into(),
        ));
    }
    url.set_query(None);
    let mut path = url.path().trim_end_matches('/').to_owned();
    path.push_str("/chat/completions");
    url.set_path(&path);
    Ok(url)
}

fn response_content(value: &Value) -> Result<String, ProviderError> {
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"));
    if let Some(content) = content.and_then(Value::as_str) {
        return Ok(content.to_owned());
    }
    if let Some(parts) = content.and_then(Value::as_array) {
        let text = parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<String>();
        if !text.is_empty() {
            return Ok(text);
        }
    }
    Err(ProviderError::Protocol(
        "response did not contain choices[0].message.content".into(),
    ))
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, ProviderError> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((end, delimiter_len)) = find_event_boundary(&self.buffer) {
            let event = self.buffer.drain(..end).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_len);
            let event = std::str::from_utf8(&event)
                .map_err(|error| ProviderError::Protocol(error.to_string()))?;
            let data = event
                .lines()
                .filter_map(|line| {
                    let line = line.trim_end_matches('\r');
                    (!line.starts_with(':'))
                        .then(|| line.strip_prefix("data:").map(str::trim_start))
                        .flatten()
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                events.push(data);
            }
        }
        Ok(events)
    }

    fn finish(&self) -> Result<(), ProviderError> {
        if self.buffer.iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            Err(ProviderError::Protocol(
                "stream ended in the middle of an SSE event".into(),
            ))
        }
    }
}

fn find_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4))
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| (position, 2))
        })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    use super::*;

    fn provider(base_url: String, stream: bool) -> ProviderConfig {
        ProviderConfig::new(
            lexwisp_core::ProviderId::parse("fixture").expect("provider ID"),
            "Fixture",
            base_url,
            None,
            vec!["fixture".into()],
            stream,
        )
    }

    fn serve_once(
        status: &str,
        content_type: &str,
        parts: Vec<Vec<u8>>,
        delay: Duration,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock listener binds");
        let address = listener.local_addr().expect("mock address");
        let status = status.to_owned();
        let content_type = content_type.to_owned();
        let content_length = parts.iter().map(Vec::len).sum::<usize>();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("mock accepts request");
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
            )
            .expect("mock writes headers");
            for part in parts {
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
                if stream.write_all(&part).is_err() {
                    break;
                }
                let _ = stream.flush();
            }
        });
        (format!("http://{address}/v1"), worker)
    }

    #[test]
    fn endpoint_preserves_a_base_path_prefix() {
        assert_eq!(
            completion_endpoint("https://example.com/custom/v1")
                .expect("valid endpoint")
                .as_str(),
            "https://example.com/custom/v1/chat/completions"
        );
    }

    #[test]
    fn sse_decoder_handles_fragmented_utf8_json_and_multiline_data() {
        let source =
            "data: {\"choices\":[{\"delta\":{\"content\":\"中\"}}]}\r\n\r\ndata: [DONE]\n\n";
        let bytes = source.as_bytes();
        let split = source.find('中').expect("fixture contains unicode") + 1;
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(&bytes[..split])
                .expect("first fragment")
                .is_empty()
        );
        let events = decoder.push(&bytes[split..]).expect("second fragment");
        assert_eq!(events.len(), 2);
        assert!(events[0].contains('中'));
        assert_eq!(events[1], "[DONE]");
        decoder.finish().expect("decoder is complete");
    }

    #[test]
    fn truncated_sse_event_is_rejected() {
        let mut decoder = SseDecoder::default();
        decoder
            .push(b"data: {\"choices\":[")
            .expect("buffering succeeds");
        assert!(decoder.finish().is_err());
    }

    #[tokio::test]
    async fn http_authentication_and_rate_limit_errors_are_distinct() {
        for (status, expected) in [
            ("401 Unauthorized", "authentication failed"),
            ("429 Too Many Requests", "rate limit exceeded"),
        ] {
            let (base_url, worker) = serve_once(
                status,
                "application/json",
                vec![br#"{"error":"fixture"}"#.to_vec()],
                Duration::ZERO,
            );
            let result = AiService::new()
                .expect("AI service")
                .chat(
                    &provider(base_url, false),
                    "fixture",
                    None,
                    &[AiMessage {
                        role: AiRole::User,
                        content: "hello".into(),
                    }],
                    &CancellationToken::new(),
                    |_| Ok(()),
                )
                .await;
            worker.join().expect("mock exits");
            assert!(
                result
                    .expect_err("status must fail")
                    .to_string()
                    .contains(expected)
            );
        }
    }

    #[tokio::test]
    async fn empty_choices_are_rejected() {
        let (base_url, worker) = serve_once(
            "200 OK",
            "application/json",
            vec![br#"{"choices":[]}"#.to_vec()],
            Duration::ZERO,
        );
        let result = AiService::new()
            .expect("AI service")
            .chat(
                &provider(base_url, false),
                "fixture",
                None,
                &[AiMessage {
                    role: AiRole::User,
                    content: "hello".into(),
                }],
                &CancellationToken::new(),
                |_| Ok(()),
            )
            .await;
        worker.join().expect("mock exits");
        assert!(matches!(result, Err(ProviderError::Protocol(_))));
    }

    #[tokio::test]
    async fn interrupted_stream_preserves_emitted_partial_text() {
        let body = br#"data: {"choices":[{"delta":{"content":"partial"}}]}

"#;
        let (base_url, worker) = serve_once(
            "200 OK",
            "text/event-stream",
            vec![body.to_vec()],
            Duration::ZERO,
        );
        let partial = Arc::new(Mutex::new(String::new()));
        let observed = partial.clone();
        let result = AiService::new()
            .expect("AI service")
            .chat(
                &provider(base_url, true),
                "fixture",
                None,
                &[AiMessage {
                    role: AiRole::User,
                    content: "hello".into(),
                }],
                &CancellationToken::new(),
                move |delta| {
                    observed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push_str(&delta);
                    Ok(())
                },
            )
            .await;
        worker.join().expect("mock exits");
        assert!(matches!(result, Err(ProviderError::Protocol(_))));
        assert_eq!(
            partial
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_str(),
            "partial"
        );
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_slow_stream() {
        let body = b"data: [DONE]\n\n".to_vec();
        let (base_url, worker) = serve_once(
            "200 OK",
            "text/event-stream",
            vec![body],
            Duration::from_millis(250),
        );
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        });
        let result = AiService::new()
            .expect("AI service")
            .chat(
                &provider(base_url, true),
                "fixture",
                None,
                &[AiMessage {
                    role: AiRole::User,
                    content: "hello".into(),
                }],
                &cancellation,
                |_| Ok(()),
            )
            .await;
        worker.join().expect("mock exits");
        assert_eq!(result, Err(ProviderError::Cancelled));
    }
}
