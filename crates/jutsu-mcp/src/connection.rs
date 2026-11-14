//! `McpServer` over one client's stream of lines, each request answered in a task of its own, so a call that waits on
//! the app (`jobs` with `wait_seconds`) doesn't hold up the calls sent after it.
//!
//! `initialize` and everything that isn't a request are answered in turn, before the next line is taken, so the
//! requests after the handshake see its session. `notifications/cancelled` cancels its request's task, and a
//! cancelled request is answered with nothing, as MCP asks; `initialize` is never cancelled. Each reply is written
//! whole, one at a time.

use crate::{Error, McpServer};
use jutsu_agent::json;
use serde_json::Value;
use std::{
    collections::HashMap,
    mem,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt},
    task::{AbortHandle, JoinHandle},
};

/// Where replies go: one whole line at a time, never two at once.
pub trait Write: Send + Sync + 'static {
    fn write(&self, line: &[u8]);
}

struct State {
    server: McpServer,
    /// The requests being answered, by their JSON-RPC ID; the token tells a request from a later one sent with its
    /// ID.
    running: HashMap<String, (u64, AbortHandle)>,
    tasks: Vec<JoinHandle<()>>,
    next_token: u64,
}

#[derive(Clone)]
pub struct McpConnection {
    state: Arc<Mutex<State>>,
    write: Arc<dyn Write>,
}

impl McpConnection {
    pub fn new(server: McpServer, write: Arc<dyn Write>) -> Self {
        let state = State {
            server,
            running: HashMap::new(),
            tasks: Vec::new(),
            next_token: 0,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            write,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes one line from the client. Must be called inside a tokio runtime, one line at a time.
    pub async fn receive(&self, line: &[u8]) {
        let message = serde_json::from_slice::<Value>(line).ok();
        let field = |key: &str| message.as_ref().and_then(|message| message.get(key));
        let method = field("method").and_then(Value::as_str);
        if method == Some("notifications/cancelled") && field("id").is_none() {
            if let Some(id) = field("params").and_then(|params| params.get("requestId")) {
                self.cancel(id);
            }
            return;
        }
        let (Some(id), Some(method)) = (field("id"), method) else {
            return self.answer_in_turn(line).await;
        };
        if method == "initialize" {
            return self.answer_in_turn(line).await;
        }
        let key = json::write(id);
        let mut state = self.state();
        let token = state.next_token;
        state.next_token += 1;
        let mut server = state.server.clone();
        let line = line.to_vec();
        let connection = self.clone();
        let finish_key = key.clone();
        let task = tokio::spawn(async move {
            let reply = server.reply(&line).await;
            connection.finish(&finish_key, token, reply.as_deref());
        });
        state.running.insert(key, (token, task.abort_handle()));
        state.tasks.push(task);
    }

    /// Answered at once, without waiting on the app; `initialize` keeps the session it sets up.
    async fn answer_in_turn(&self, line: &[u8]) {
        let mut server = self.state().server.clone();
        let reply = server.reply(line).await;
        let mut state = self.state();
        state.server = server;
        if let Some(reply) = reply {
            self.write.write(&reply);
        }
        drop(state);
    }

    /// Waits for the requests still being answered, as when the client has closed its end.
    pub async fn drain(&self) {
        loop {
            let tasks = mem::take(&mut self.state().tasks);
            if tasks.is_empty() {
                return;
            }
            for task in tasks {
                // A cancelled request's task ends aborted, which is no failure here.
                let _ = task.await;
            }
        }
    }

    fn cancel(&self, id: &Value) {
        if let Some((_, task)) = self.state().running.remove(&json::write(id)) {
            task.abort();
        }
    }

    /// Writes the reply unless the request was cancelled meanwhile.
    fn finish(&self, key: &str, token: u64, reply: Option<&[u8]>) {
        let mut state = self.state();
        if state
            .running
            .get(key)
            .is_none_or(|(running, _)| *running != token)
        {
            return;
        }
        state.running.remove(key);
        // Written under the lock, so two replies never interleave.
        if let Some(reply) = reply {
            self.write.write(reply);
        }
    }
}

/// Serves the client's lines from `input` until it closes its end, then waits for the answers still coming. Empty
/// lines are skipped.
pub async fn serve<R: AsyncBufRead + Unpin>(connection: &McpConnection, mut input: R) -> Result<(), Error> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line).await? == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if !line.is_empty() {
            connection.receive(&line).await;
        }
    }
    connection.drain().await;
    Ok(())
}
