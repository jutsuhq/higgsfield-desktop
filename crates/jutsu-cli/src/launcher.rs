//! Starts the app beside the helper as its own process, detached from the helper so it outlives it and never shares
//! its stdio, which carries MCP messages.

use async_trait::async_trait;
use jutsu_mcp::{AppLauncher, RunningApp};
use std::{
    io,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Mutex, PoisonError},
    time::Instant,
};

pub struct ProcessLauncher;

#[async_trait]
impl AppLauncher for ProcessLauncher {
    async fn start(&self, app: &Path, arguments: &[&str]) -> io::Result<RunningApp> {
        let mut command = Command::new(app);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut command);
        let child = Mutex::new(command.spawn()?);
        Ok(RunningApp {
            launched: Instant::now(),
            is_terminated: Box::new(move || has_quit(&child)),
        })
    }
}

fn has_quit(child: &Mutex<Child>) -> bool {
    let mut child = child.lock().unwrap_or_else(PoisonError::into_inner);
    // A process the helper can no longer ask about is as good as gone.
    !matches!(child.try_wait(), Ok(None))
}

/// Its own process group, so a signal to the helper's (a client stopping it) doesn't reach the app.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// No console of its own and a process group of its own: `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut Command) {}
