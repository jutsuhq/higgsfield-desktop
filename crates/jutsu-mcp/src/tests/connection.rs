//! Requests served at once over one client's lines (`McpConnection`).

use crate::{App, McpConnection, McpServer, Write};
use async_trait::async_trait;
use jutsu_agent::{AgentCapabilities, AgentCommand, AgentResponse, Connection, ConnectionKind};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// What the connection wrote, and what the fake app saw, from any thread.
#[derive(Default)]
struct Record {
    lines: Mutex<Vec<Vec<u8>>>,
    cancelled: Mutex<Vec<String>>,
    clients: Mutex<Vec<Option<String>>>,
}

impl Record {
    /// The ID of each reply, in the order they were written; every reply is one whole line.
    fn ids(&self) -> Vec<Value> {
        let lines = self.lines.lock().expect("lines");
        lines
            .iter()
            .filter(|line| line.last() == Some(&b'\n') && !line[..line.len() - 1].contains(&b'\n'))
            .filter_map(|line| {
                serde_json::from_slice::<Value>(line)
                    .ok()
                    .map(|reply| reply["id"].clone())
            })
            .collect()
    }

    fn cancels(&self) -> Vec<String> {
        self.cancelled.lock().expect("cancelled").clone()
    }
}

struct Writer(Arc<Record>);

impl Write for Writer {
    fn write(&self, line: &[u8]) {
        self.0.lines.lock().expect("lines").push(line.to_vec());
    }
}

/// Records a cancel when the work it guards is dropped before it finished: an aborted task.
struct CancelGuard {
    record: Arc<Record>,
    finished: bool,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.record
                .cancelled
                .lock()
                .expect("cancelled")
                .push("jobs".to_owned());
        }
    }
}

/// A fake app that answers `jobs` only after 30 s, or never once its task is cancelled.
struct SlowApp(Arc<Record>);

#[async_trait]
impl App for SlowApp {
    async fn send(&self, command: AgentCommand, client: Option<String>) -> AgentResponse {
        self.0.clients.lock().expect("clients").push(client);
        if matches!(command, AgentCommand::Jobs(_)) {
            let mut guard = CancelGuard {
                record: self.0.clone(),
                finished: false,
            };
            tokio::time::sleep(Duration::from_secs(30)).await;
            guard.finished = true;
        }
        let capabilities = AgentCapabilities::new(
            "0.2.0",
            Connection {
                kind: ConnectionKind::None,
                plan: None,
                credits: None,
            },
            None,
        );
        AgentResponse {
            capabilities: Some(capabilities),
            ..AgentResponse::default()
        }
    }
}

fn connection(record: &Arc<Record>) -> McpConnection {
    let server = McpServer::new("0.2.0", Arc::new(SlowApp(record.clone())));
    McpConnection::new(server, Arc::new(Writer(record.clone())))
}

fn line(method: &str, params: Option<Value>, id: Option<Value>) -> Vec<u8> {
    let mut message = json!({ "jsonrpc": "2.0", "method": method });
    if let Some(id) = id {
        message["id"] = id;
    }
    if let Some(params) = params {
        message["params"] = params;
    }
    serde_json::to_vec(&message).expect("line")
}

async fn eventually(condition: impl Fn() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never came true");
}

#[tokio::test]
async fn a_waiting_call_holds_up_nothing_and_a_cancelled_one_is_never_answered() {
    let record = Arc::new(Record::default());
    let connection = connection(&record);
    let initialize = json!({ "protocolVersion": "2025-06-18", "clientInfo": { "name": "agent" } });
    connection
        .receive(&line("initialize", Some(initialize), Some(json!(0))))
        .await;
    let jobs = json!({ "name": "jobs", "arguments": { "wait_seconds": 50 } });
    connection
        .receive(&line("tools/call", Some(jobs), Some(json!(1))))
        .await;
    connection
        .receive(&line(
            "tools/call",
            Some(json!({ "name": "capabilities" })),
            Some(json!(2)),
        ))
        .await;
    eventually(|| record.ids() == [json!(0), json!(2)]).await;

    let cancel = json!({ "requestId": 1, "reason": "user" });
    connection
        .receive(&line("notifications/cancelled", Some(cancel), None))
        .await;
    eventually(|| record.cancels() == ["jobs"]).await;
    connection.drain().await;

    assert_eq!(record.ids(), [json!(0), json!(2)]);
    // Both requests ran in the session the handshake set up.
    let clients = record.clients.lock().expect("clients").clone();
    assert_eq!(clients, [Some("agent".to_owned()), Some("agent".to_owned())]);
}

#[tokio::test]
async fn stateless_requests_and_notifications_work_as_before() {
    let record = Arc::new(Record::default());
    let connection = connection(&record);
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    connection
        .receive(&line("notifications/initialized", None, None))
        .await;
    let call = json!({ "_meta": meta, "name": "capabilities" });
    connection
        .receive(&line("tools/call", Some(call), Some(json!("a"))))
        .await;
    connection
        .receive(&line(
            "notifications/cancelled",
            Some(json!({ "requestId": "unknown" })),
            None,
        ))
        .await;
    connection.receive(b"not json").await;
    connection.drain().await;

    // The parse error is answered in turn, the request in a task of its own: either may come first.
    let ids = record.ids();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&json!("a")) && ids.contains(&Value::Null));
}

/// The stdio loop: lines in, replies out, empty lines skipped, and the answers still coming waited for at the end.
#[tokio::test]
async fn serve_reads_lines_until_the_client_closes_its_end() {
    let record = Arc::new(Record::default());
    let connection = connection(&record);
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}";
    crate::serve(&connection, &input[..]).await.expect("served");
    let mut ids = record.ids();
    ids.sort_by_key(Value::to_string);
    assert_eq!(ids, [json!(1), json!(2)]);
}
