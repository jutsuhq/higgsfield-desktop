//! `text/event-stream` bodies of the Streamable HTTP transport, read as their bytes arrive.

use futures::{Stream, StreamExt};

/// The most a single MCP response may carry; anything larger is refused rather than buffered.
pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// A body grew past its limit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResponseTooLarge;

/// A body that broke off, or grew past its limit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadError {
    TooLarge,
    Transport,
}

impl From<ResponseTooLarge> for ReadError {
    fn from(_: ResponseTooLarge) -> Self {
        Self::TooLarge
    }
}

/// Splits a `text/event-stream` body into the `data` payload of each event as its bytes arrive.
/// Multi-line data is joined with `\n`; other fields (`event`, `id`, `retry`) and comments are ignored.
#[derive(Default)]
pub(crate) struct ServerSentEventParser {
    line: Vec<u8>,
    data: Vec<String>,
}

impl ServerSentEventParser {
    /// Feeds the next byte; returns the event's data when that byte ends an event.
    pub fn consume(&mut self, byte: u8) -> Option<String> {
        if byte != b'\n' {
            self.line.push(byte);
            return None;
        }
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
        let line = std::mem::take(&mut self.line);
        if line.is_empty() {
            if self.data.is_empty() {
                return None;
            }
            return Some(std::mem::take(&mut self.data).join("\n"));
        }
        if let Some(value) = line.strip_prefix(b"data:") {
            let value = value.strip_prefix(b" ").unwrap_or(value);
            self.data.push(String::from_utf8_lossy(value).into_owned());
        }
        None
    }

    /// The data of an event the body ended without a blank line after, if any.
    pub fn finish(&mut self) -> Option<String> {
        if !self.line.is_empty() {
            let _ = self.consume(b'\n');
        }
        self.consume(b'\n')
    }
}

/// The `data` payload of each event in a `text/event-stream` body.
pub(crate) fn server_sent_event_data(body: &str) -> Vec<String> {
    let mut parser = ServerSentEventParser::default();
    let mut events: Vec<String> = body.bytes().filter_map(|byte| parser.consume(byte)).collect();
    events.extend(parser.finish());
    events
}

/// Reads an event stream until `is_complete` accepts the events received so far, then stops. A Streamable
/// HTTP server may keep the stream open after it sent the reply, so waiting for the end of the body hangs.
/// Fails with `TooLarge` once the body exceeds `limit` bytes.
pub(crate) async fn read_event_stream<S, B, E>(
    mut chunks: S,
    limit: usize,
    mut is_complete: impl FnMut(&[String]) -> bool,
) -> Result<Vec<u8>, ReadError>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut body = Vec::new();
    let mut parser = ServerSentEventParser::default();
    let mut events = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| ReadError::Transport)?;
        for &byte in chunk.as_ref() {
            if body.len() >= limit {
                return Err(ReadError::TooLarge);
            }
            body.push(byte);
            let Some(event) = parser.consume(byte) else {
                continue;
            };
            events.push(event);
            if is_complete(&events) {
                return Ok(body);
            }
        }
    }
    Ok(body)
}

/// Collects a body, failing with `TooLarge` once it exceeds `limit`.
pub(crate) async fn read_body<S, B, E>(mut chunks: S, limit: usize) -> Result<Vec<u8>, ReadError>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut data = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| ReadError::Transport)?;
        if data.len() + chunk.as_ref().len() > limit {
            return Err(ReadError::TooLarge);
        }
        data.extend_from_slice(chunk.as_ref());
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use futures::stream;

    use super::*;
    use crate::mcp::client::is_reply;

    type Chunk = Result<Vec<u8>, Infallible>;

    /// Bytes of `text`, then the stream stays open like a server that keeps the connection.
    fn open_stream(text: &str) -> impl Stream<Item = Chunk> + Unpin + use<> {
        let bytes: Vec<Chunk> = text.bytes().map(|byte| Ok(vec![byte])).collect();
        stream::iter(bytes).chain(stream::pending())
    }

    fn closed_stream(text: &str) -> impl Stream<Item = Chunk> + Unpin + use<> {
        stream::iter(text.bytes().map(|byte| Ok(vec![byte])).collect::<Vec<Chunk>>())
    }

    fn text(body: &[u8]) -> String {
        String::from_utf8_lossy(body).into_owned()
    }

    #[tokio::test]
    async fn handles_carriage_return_line_ends() {
        let stream = open_stream("data: {\"id\":1}\r\n\r\n");
        let body = read_event_stream(stream, MAX_RESPONSE_BYTES, |events| {
            events.iter().any(|event| event == "{\"id\":1}")
        })
        .await
        .unwrap();
        assert_eq!(text(&body), "data: {\"id\":1}\r\n\r\n");
    }

    #[tokio::test]
    async fn reads_to_the_end_when_no_event_completes() {
        let body = read_event_stream(
            closed_stream("data: a\n\ndata: b\n\n"),
            MAX_RESPONSE_BYTES,
            |_| false,
        )
        .await
        .unwrap();
        assert_eq!(server_sent_event_data(&text(&body)), ["a", "b"]);
    }

    #[tokio::test]
    async fn checks_completion_once_per_event_as_it_arrives() {
        let stream = open_stream("data: a\n\nevent: x\ndata: b\ndata: c\n\n: comment\n\ndata: d\r\n\r\n");
        let mut seen: Vec<Vec<String>> = Vec::new();
        read_event_stream(stream, MAX_RESPONSE_BYTES, |events| {
            seen.push(events.to_vec());
            events.len() == 3
        })
        .await
        .unwrap();
        assert_eq!(seen, [vec!["a"], vec!["a", "b\nc"], vec!["a", "b\nc", "d"]]);
    }

    #[test]
    fn reply_check_reads_only_the_newest_event() {
        let reply = r#"{"jsonrpc":"2.0","id":7,"result":{}}"#.to_owned();
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/progress"}"#.to_owned();
        assert!(is_reply(7, &[notification.clone(), reply.clone()]));
        assert!(!is_reply(7, &[reply, notification]));
        assert!(!is_reply(
            7,
            &[r#"{"jsonrpc":"2.0","id":8,"result":{}}"#.to_owned()]
        ));
        assert!(!is_reply(7, &[]));
    }

    #[tokio::test]
    async fn many_events_before_the_reply_are_checked_once_each() {
        let notification = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n";
        let stream = open_stream(
            &(notification.repeat(2_000) + "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n\n"),
        );
        let mut checks = 0;
        let body = read_event_stream(stream, MAX_RESPONSE_BYTES, |events| {
            checks += 1;
            is_reply(7, events)
        })
        .await
        .unwrap();
        assert_eq!(checks, 2_001);
        assert_eq!(server_sent_event_data(&text(&body)).len(), 2_001);
    }

    #[test]
    fn parses_events_split_across_chunks() {
        let mut parser = ServerSentEventParser::default();
        let mut events = Vec::new();
        for chunk in ["da", "ta: {\"id\"", ":1}\r", "\n", "\r\nda", "ta: two\n", "\n"] {
            events.extend(chunk.bytes().filter_map(|byte| parser.consume(byte)));
        }
        assert_eq!(events, ["{\"id\":1}", "two"]);
        assert_eq!(parser.finish(), None);
    }

    #[test]
    fn keeps_an_event_the_body_ends_without_a_blank_line() {
        assert_eq!(server_sent_event_data("data: a\n\ndata: b"), ["a", "b"]);
    }

    #[tokio::test]
    async fn refuses_an_event_stream_over_the_limit() {
        let stream = open_stream(&format!("data: {}\n\n", "x".repeat(64)));
        assert_eq!(
            read_event_stream(stream, 32, |_| false).await,
            Err(ReadError::TooLarge)
        );
    }

    #[tokio::test]
    async fn refuses_a_body_over_the_limit() {
        assert_eq!(
            read_body(closed_stream(&"x".repeat(33)), 32).await,
            Err(ReadError::TooLarge)
        );
    }

    #[tokio::test]
    async fn reads_a_body_at_the_limit() {
        assert_eq!(
            read_body(closed_stream(&"x".repeat(32)), 32).await.unwrap().len(),
            32
        );
        assert_eq!(MAX_RESPONSE_BYTES, 16 * 1024 * 1024);
    }
}
