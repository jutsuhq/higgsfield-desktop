//! Where the app listens. One function (`AgentEndpoint::from_environment`) decides it, and both the app and the helper
//! call it.
//!
//! - Linux and macOS: `agent.sock` in `$XDG_RUNTIME_DIR/jutsu-studio/`, else in the app data folder; the socket is
//!   mode 0600 in a folder of mode 0700.
//! - Windows: the named pipe `\\.\pipe\jutsu-studio-agent-<user name>`, open to its owner only.
//!
//! `JUTSU_AGENT_SOCKET` names another socket (a path, or on Windows a pipe name), for development; the helper then
//! never starts the app.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub const FILE_NAME: &str = "agent.sock";
/// Another socket for the helper and a development build of the app to use.
pub const VARIABLE: &str = "JUTSU_AGENT_SOCKET";
/// The app's identifier, which names its data folder (`identifier` in the app's Tauri configuration).
pub const APP_IDENTIFIER: &str = "com.jutsustudio.desktop";
/// The folder in `$XDG_RUNTIME_DIR` that holds the socket.
pub const RUNTIME_FOLDER: &str = "jutsu-studio";
/// The named pipe's name before the user's.
pub const PIPE_PREFIX: &str = "jutsu-studio-agent-";
/// Where Windows keeps named pipes.
pub const PIPE_NAMESPACE: &str = r"\\.\pipe\";

/// The longest socket path, in bytes: what a `sockaddr_un` holds before its NUL (108 bytes on Linux, 104 elsewhere),
/// and on Windows the longest pipe name.
#[cfg(target_os = "linux")]
pub const MAX_PATH_LENGTH: usize = 107;
#[cfg(all(unix, not(target_os = "linux")))]
pub const MAX_PATH_LENGTH: usize = 103;
#[cfg(windows)]
pub const MAX_PATH_LENGTH: usize = 256;

/// The folder that holds the inbox, beside the socket.
pub const INBOX_FOLDER: &str = "inbox";

/// The socket (or pipe) the app listens on, and the folder of its inbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentEndpoint {
    /// The socket file's path, or on Windows the pipe's full name.
    pub name: PathBuf,
    /// The folder that holds `inbox/`: the socket's own on Linux and macOS.
    pub folder: PathBuf,
    /// Named by `JUTSU_AGENT_SOCKET`: the helper never starts the app for it.
    pub is_override: bool,
    /// The folder is the app's own, made private (0700) when the listener makes it; never so for an override, which
    /// may name a shared folder such as the temporary one.
    pub owns_folder: bool,
}

impl AgentEndpoint {
    /// The endpoint this process's environment names.
    pub fn current() -> Option<Self> {
        Self::from_environment(|name| std::env::var_os(name))
    }

    /// The endpoint the environment `variable` reads names; `None` when no home or app data folder is known.
    pub fn from_environment(variable: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let set = |name: &str| variable(name).filter(|value| !value.is_empty());
        if let Some(path) = set(VARIABLE) {
            return Some(Self::overridden(&path, &set));
        }
        Self::default_for(&set)
    }

    /// A socket at `path`, its inbox beside it: for an override, and for tests.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let name = path.into();
        let folder = name.parent().map(Path::to_path_buf).unwrap_or_default();
        Self {
            name,
            folder,
            is_override: true,
            owns_folder: false,
        }
    }

    /// The inbox folder.
    pub fn inbox(&self) -> PathBuf {
        self.folder.join(INBOX_FOLDER)
    }

    /// Whether the name fits a local socket address.
    pub fn fits(&self) -> bool {
        self.name.as_os_str().len() <= MAX_PATH_LENGTH
    }

    #[cfg(unix)]
    fn overridden(path: &OsString, set: &impl Fn(&str) -> Option<OsString>) -> Self {
        let text = path.to_string_lossy();
        let name = match (text.strip_prefix("~/"), set("HOME")) {
            (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
            _ => PathBuf::from(path),
        };
        Self::at(name)
    }

    #[cfg(windows)]
    fn overridden(path: &OsString, set: &impl Fn(&str) -> Option<OsString>) -> Self {
        let text = path.to_string_lossy();
        let name = if text.starts_with(PIPE_NAMESPACE) {
            text.into_owned()
        } else {
            format!("{PIPE_NAMESPACE}{text}")
        };
        let folder = app_data(set).unwrap_or_else(std::env::temp_dir).join("agent");
        Self {
            name: PathBuf::from(name),
            folder,
            is_override: true,
            owns_folder: false,
        }
    }

    #[cfg(unix)]
    fn default_for(set: &impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let folder = match set("XDG_RUNTIME_DIR") {
            Some(runtime) => PathBuf::from(runtime).join(RUNTIME_FOLDER),
            None => app_data(set)?,
        };
        Some(Self {
            name: folder.join(FILE_NAME),
            folder,
            is_override: false,
            owns_folder: true,
        })
    }

    #[cfg(windows)]
    fn default_for(set: &impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let user = set("USERNAME")
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!("{PIPE_NAMESPACE}{PIPE_PREFIX}{}", pipe_safe(&user));
        let folder = app_data(set)?.join("agent");
        Some(Self {
            name: PathBuf::from(name),
            folder,
            is_override: false,
            owns_folder: true,
        })
    }
}

/// The app's data folder, as Tauri's `app_data_dir` places it.
fn app_data(set: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(set("LOCALAPPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(set("HOME")?).join("Library/Application Support")
    } else {
        match set("XDG_DATA_HOME") {
            Some(data) => PathBuf::from(data),
            None => PathBuf::from(set("HOME")?).join(".local/share"),
        }
    };
    Some(base.join(APP_IDENTIFIER))
}

/// A user name as a pipe name takes it: letters, digits, `.`, `_` and `-`, others as `_`; `user` when empty.
#[cfg_attr(not(windows), allow(dead_code, reason = "named pipes are Windows only"))]
pub(crate) fn pipe_safe(user: &str) -> String {
    let safe: String = user
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() { "user".to_owned() } else { safe }
}

#[cfg(test)]
mod tests {
    use super::{AgentEndpoint, FILE_NAME, MAX_PATH_LENGTH, pipe_safe};
    use std::{collections::HashMap, ffi::OsString, path::PathBuf};

    fn endpoint(variables: &[(&str, &str)]) -> Option<AgentEndpoint> {
        let variables: HashMap<String, OsString> = variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        AgentEndpoint::from_environment(|name| variables.get(name).cloned())
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_is_in_the_runtime_folder_else_the_app_data_folder() {
        let runtime =
            endpoint(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("HOME", "/home/mara")]).expect("endpoint");
        assert_eq!(
            runtime.name,
            PathBuf::from("/run/user/1000/jutsu-studio").join(FILE_NAME)
        );
        assert_eq!(
            runtime.inbox(),
            PathBuf::from("/run/user/1000/jutsu-studio/inbox")
        );
        assert!(!runtime.is_override && runtime.owns_folder);
        let data = endpoint(&[("HOME", "/home/mara")]).expect("endpoint");
        let expected = if cfg!(target_os = "macos") {
            "/home/mara/Library/Application Support/com.jutsustudio.desktop/agent.sock"
        } else {
            "/home/mara/.local/share/com.jutsustudio.desktop/agent.sock"
        };
        assert_eq!(data.name, PathBuf::from(expected));
        assert!(endpoint(&[]).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_variable_overrides_it() {
        let named =
            endpoint(&[(super::VARIABLE, "~/dev/agent.sock"), ("HOME", "/home/mara")]).expect("endpoint");
        assert_eq!(named.name, PathBuf::from("/home/mara/dev/agent.sock"));
        assert_eq!(named.folder, PathBuf::from("/home/mara/dev"));
        assert!(named.is_override && !named.owns_folder);
        assert!(!AgentEndpoint::at(format!("/{}", "a".repeat(MAX_PATH_LENGTH))).fits());
        assert!(AgentEndpoint::at(format!("/{}", "a".repeat(MAX_PATH_LENGTH - 1))).fits());
    }

    #[test]
    fn a_user_name_is_safe_in_a_pipe_name() {
        assert_eq!(pipe_safe("Mara Lind"), "Mara_Lind");
        assert_eq!(pipe_safe(""), "user");
    }
}
