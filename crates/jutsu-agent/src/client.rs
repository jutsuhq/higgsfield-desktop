//! One connection to the app's socket: requests are sent and answered one at a time.

use crate::{AgentEndpoint, AgentLineBuffer, AgentRequest, AgentResponse, Error, UploadCommand, wire};
use interprocess::local_socket::{
    GenericFilePath, ToFsName,
    tokio::{Stream, prelude::*},
};
use std::{io, path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The most of a file read and sent at once.
const UPLOAD_CHUNK: u64 = 1 << 20;

pub struct AgentClient {
    stream: Stream,
    buffer: AgentLineBuffer,
    timeout: Duration,
}

impl AgentClient {
    /// Connects to `endpoint`. `timeout` is how long a write or a wait for the answer may take before the connection
    /// counts as closed, so a hung app can't hang the agent too.
    pub async fn connect(endpoint: &AgentEndpoint, timeout: Duration) -> Result<Self, Error> {
        if !endpoint.fits() {
            return Err(Error::PathTooLong);
        }
        let name = endpoint
            .name
            .as_path()
            .to_fs_name::<GenericFilePath>()
            .map_err(Error::NotListening)?;
        let stream = match tokio::time::timeout(timeout, Stream::connect(name)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => return Err(Error::NotListening(error)),
            Err(_) => return Err(Error::TimedOut),
        };
        Ok(Self {
            stream,
            buffer: AgentLineBuffer::new(),
            timeout: timeout.max(Duration::from_secs(1)),
        })
    }

    pub async fn send(&mut self, request: &AgentRequest) -> Result<AgentResponse, Error> {
        let line = wire::line(request).map_err(Error::Malformed)?;
        self.send_line(&line).await
    }

    /// A line as given, newline included: also for tests of what the app does with malformed input.
    pub async fn send_line(&mut self, line: &[u8]) -> Result<AgentResponse, Error> {
        self.write(line).await?;
        self.answer().await
    }

    /// Sends `file`'s bytes, `command.size` of them, after the request's line, and returns the app's answer. The app
    /// may refuse before reading them and close the connection; its answer is read all the same.
    pub async fn upload(
        &mut self,
        command: &UploadCommand,
        file: &Path,
        id: Option<&str>,
        client: Option<&str>,
    ) -> Result<AgentResponse, Error> {
        let mut request = AgentRequest::new(crate::AgentCommand::Upload(command.clone()));
        request.id = id.map(str::to_owned);
        request.client = client.map(str::to_owned);
        self.write(&wire::line(&request).map_err(Error::Malformed)?)
            .await?;
        let mut source = tokio::fs::File::open(file).await.map_err(Error::File)?;
        let size = u64::try_from(command.size).unwrap_or(0);
        let mut sent = 0;
        let mut chunk = vec![0; usize::try_from(UPLOAD_CHUNK).unwrap_or(1 << 20)];
        while sent < size {
            let wanted = usize::try_from((size - sent).min(UPLOAD_CHUNK)).unwrap_or(chunk.len());
            let count = source.read(&mut chunk[..wanted]).await.map_err(Error::File)?;
            // The app would wait for the rest; closing the connection tells it the upload failed.
            if count == 0 {
                return Err(Error::FileChanged);
            }
            match self.write(&chunk[..count]).await {
                Ok(()) => sent += count as u64,
                // Refused early: the answer says why.
                Err(Error::Closed) => break,
                Err(error) => return Err(error),
            }
        }
        self.answer().await
    }

    async fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        match tokio::time::timeout(self.timeout, self.stream.write_all(data)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(Error::Closed),
            Err(_) => Err(Error::TimedOut),
        }
    }

    async fn answer(&mut self) -> Result<AgentResponse, Error> {
        let line = self.read_line().await?;
        wire::decode(&line).map_err(Error::Malformed)
    }

    async fn read_line(&mut self) -> Result<Vec<u8>, Error> {
        let mut chunk = vec![0; 4096];
        loop {
            // Answers aren't capped like requests: a list of jobs grows with the queue.
            if let Some(line) = self.buffer.next_line() {
                return Ok(line);
            }
            let count = match tokio::time::timeout(self.timeout, self.stream.read(&mut chunk)).await {
                Ok(Ok(0)) => return Err(Error::Closed),
                Ok(Ok(count)) => count,
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(_)) => return Err(Error::Closed),
                Err(_) => return Err(Error::TimedOut),
            };
            self.buffer.push(&chunk[..count]);
        }
    }
}
