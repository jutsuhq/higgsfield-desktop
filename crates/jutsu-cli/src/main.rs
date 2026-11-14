//! The `jutsu` helper binary bundled next to the app. `jutsu mcp` runs the MCP server over stdio: stdout carries MCP
//! messages only, and anything for a person goes to stderr.

mod launcher;

use jutsu_agent::{AgentEndpoint, socket};
use jutsu_mcp::{AppConnection, Location, McpConnection, McpServer, Write, serve};
use launcher::ProcessLauncher;
use std::{
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

/// The version of the app this helper is bundled in: the workspace's, which the app shares.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the command line asks for: its first argument, as the macOS helper reads it.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Mcp,
    Help,
    Version,
    /// Anything else: the usage on stderr, and `EX_USAGE`.
    Usage,
}

fn parse(args: &[String]) -> Command {
    match args.first().map(String::as_str) {
        Some("mcp") => Command::Mcp,
        Some("-h" | "--help") => Command::Help,
        Some("--version") => Command::Version,
        _ => Command::Usage,
    }
}

/// `EX_USAGE` of `sysexits.h`: the command line was wrong.
const EX_USAGE: u8 = 64;

/// The help, with the command that adds this very helper to Claude Code, as Settings › Agents offers it.
fn usage(helper: &Path) -> String {
    format!(
        "Usage: jutsu mcp
       jutsu --help | --version

Lets an AI agent drive Jutsu Studio on this computer.

  mcp         A Model Context Protocol server on stdin and stdout. Add it to
              Claude Code with:
                claude mcp add jutsu -- \"{}\" mcp
  --version   The version of the app this helper is bundled in.
  -h, --help  Show this help.

The app answers only while Settings › Agents › Allow agents is on. When it isn't
running, the first call starts it in the background.

Environment:
  {}  The socket to use instead of the app's, for
                      development. The app is never started for it.",
        helper.display(),
        socket::VARIABLE
    )
}

/// The one writer of stdout: each MCP message, whole, then flushed.
struct Stdout;

impl Write for Stdout {
    fn write(&self, line: &[u8]) {
        let mut stdout = io::stdout().lock();
        if let Err(error) = stdout.write_all(line).and_then(|()| stdout.flush()) {
            eprintln!("jutsu: couldn't write to stdout: {error}");
        }
    }
}

fn mcp(executable: &Path) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("jutsu: {error}");
            return ExitCode::FAILURE;
        }
    };
    let location = Location::locate(executable, AgentEndpoint::current());
    let connection = AppConnection::new(location, Arc::new(ProcessLauncher));
    let server = McpServer::new(VERSION, Arc::new(connection));
    runtime.block_on(async {
        let session = McpConnection::new(server, Arc::new(Stdout));
        // One JSON-RPC message per line until the client closes stdin, each request answered as soon as it can be.
        match serve(&session, tokio::io::BufReader::new(tokio::io::stdin())).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("jutsu: {error}");
                ExitCode::FAILURE
            }
        }
    })
}

/// `--help` and `--version` answer on stdout, as a command asked for them does.
fn answer(text: &str) -> ExitCode {
    match writeln!(io::stdout().lock(), "{text}") {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let executable = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from(std::env::args_os().next().unwrap_or_default()));
    match parse(&args) {
        Command::Mcp => mcp(&executable),
        Command::Help => answer(&usage(&executable)),
        Command::Version => answer(VERSION),
        Command::Usage => {
            eprintln!("{}", usage(&executable));
            ExitCode::from(EX_USAGE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    fn parsed(args: &[&str]) -> Command {
        parse(&args.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>())
    }

    /// The first argument decides, as the macOS helper's `switch` on it does.
    #[test]
    fn the_first_argument_picks_the_command() {
        assert_eq!(parsed(&["mcp"]), Command::Mcp);
        assert_eq!(parsed(&["mcp", "extra"]), Command::Mcp);
        assert_eq!(parsed(&["-h"]), Command::Help);
        assert_eq!(parsed(&["--help"]), Command::Help);
        assert_eq!(parsed(&["--version"]), Command::Version);
        assert_eq!(parsed(&[]), Command::Usage);
        assert_eq!(parsed(&["serve"]), Command::Usage);
        assert_eq!(parsed(&["--mcp"]), Command::Usage);
    }
}
