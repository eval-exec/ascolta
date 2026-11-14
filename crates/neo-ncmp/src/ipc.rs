//! Per-user Unix-socket IPC for app-specific commands.
//!
//! Every request is one JSON line; every response is one JSON line. The socket
//! lives in the user's runtime directory and is created with mode 0600, so only
//! the owning user can control the running instance.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

use crate::app::App;
use crate::mpd_worker::Msg;

#[derive(Debug, Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    value: Option<Value>,
}

/// Binds the socket and starts accepting connections.
///
/// Returns an error when another instance already owns the socket, but stale
/// sockets left by a crashed process are reclaimed.
pub fn spawn(path: PathBuf, tx: mpsc::UnboundedSender<Msg>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "another neo-ncmp instance owns the IPC socket",
                ));
            }
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let listener = UnixListener::bind(&path)?;
    set_private_permissions(&path)?;
    tokio::spawn(async move {
        let _guard = SocketGuard(path);
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(handle_connection(stream, tx.clone()));
                }
                Err(error) => tracing::warn!(%error, "IPC accept failed"),
            }
        }
    });
    Ok(())
}

/// Removes the socket on a clean shutdown.
pub fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Parses and executes one request line, returning the JSON response.
pub fn handle(line: &str, app: &mut App) -> String {
    let response = match serde_json::from_str::<Request>(line) {
        Ok(request) => dispatch(request, app),
        Err(parse_error) => error(format!("invalid request: {parse_error}")),
    };
    serde_json::to_string(&response)
        .unwrap_or_else(|_| r#"{"ok":false,"error":"could not encode response"}"#.into())
}

fn dispatch(request: Request, app: &mut App) -> Value {
    let command = request.command.to_ascii_lowercase();
    let result: Result<Value, String> = match command.as_str() {
        "ping" => Ok(Value::Null),
        "screen" => string_value(request.value)
            .and_then(|name| app.ipc_set_screen(&name))
            .map(|()| Value::Null),
        "search" => string_value(request.value)
            .and_then(|query| app.ipc_search(query))
            .map(|()| Value::Null),
        "action" => string_value(request.value)
            .and_then(|action| app.ipc_action(&action))
            .map(|()| Value::Null),
        "partition" => string_value(request.value)
            .and_then(|name| app.ipc_partition(&name))
            .map(|()| Value::Null),
        "playlist" => string_value(request.value)
            .and_then(|name| app.ipc_play_playlist(&name))
            .map(|()| Value::Null),
        "notify" => string_value(request.value).map(|message| {
            app.ipc_notify(message);
            Value::Null
        }),
        "volume" => number_value(request.value).and_then(|volume| {
            if !(0.0..=100.0).contains(&volume) {
                return Err("volume must be between 0 and 100".into());
            }
            app.ipc_volume(volume.round() as u8);
            Ok(Value::Null)
        }),
        "seek" => number_value(request.value).and_then(|seconds| {
            if !seconds.is_finite() || seconds < 0.0 {
                return Err("seek needs a non-negative number of seconds".into());
            }
            app.ipc_seek(seconds);
            Ok(Value::Null)
        }),
        "status" => Ok(json!({ "status": app.ipc_status() })),
        other => return error(format!("unknown IPC command {other:?}")),
    };

    match result {
        Ok(extra) => {
            let mut response = json!({ "ok": true });
            if let Value::Object(fields) = extra
                && let Value::Object(response_fields) = &mut response
            {
                response_fields.extend(fields);
            }
            response
        }
        Err(message) => error(message),
    }
}

fn string_value(value: Option<Value>) -> Result<String, String> {
    match value {
        Some(Value::String(value)) => Ok(value),
        Some(Value::Number(value)) => Ok(value.to_string()),
        Some(_) => Err("expected a string value".into()),
        None => Err("missing value".into()),
    }
}

fn number_value(value: Option<Value>) -> Result<f64, String> {
    match value {
        Some(Value::Number(value)) => value
            .as_f64()
            .ok_or_else(|| "expected a number".to_string()),
        Some(Value::String(value)) => value.parse().map_err(|_| "expected a number".to_string()),
        Some(_) => Err("expected a number".into()),
        None => Err("missing value".into()),
    }
}

fn error(message: impl Into<String>) -> Value {
    json!({ "ok": false, "error": message.into() })
}

async fn handle_connection(stream: UnixStream, tx: mpsc::UnboundedSender<Msg>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(%error, "IPC read failed");
                break;
            }
        };
        let (reply, response) = oneshot::channel();
        if tx.send(Msg::Ipc { line, reply }).is_err() {
            break;
        }
        let Ok(response) = response.await else {
            break;
        };
        if write.write_all(response.as_bytes()).await.is_err()
            || write.write_all(b"\n").await.is_err()
            || write.flush().await.is_err()
        {
            break;
        }
    }
}

fn set_private_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_socket() -> PathBuf {
        std::env::temp_dir().join(format!(
            "neo-ncmp-ipc-test-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[tokio::test]
    async fn serves_json_lines_over_a_private_socket() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_socket();
        let _ = std::fs::remove_file(&path);
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn(path.clone(), tx).expect("bind IPC socket");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let mut stream = UnixStream::connect(&path).await.unwrap();
        stream.write_all(b"{\"command\":\"ping\"}\n").await.unwrap();
        let Msg::Ipc { line, reply } = rx.recv().await.expect("request") else {
            panic!("expected an IPC message");
        };
        assert!(line.contains("ping"));
        reply.send(r#"{"ok":true}"#.into()).unwrap();

        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .await
            .unwrap();
        assert_eq!(response.trim(), r#"{"ok":true}"#);

        cleanup(&path);
    }

    #[tokio::test]
    async fn a_second_listener_refuses_to_steal_the_socket() {
        let path = temp_socket();
        let _ = std::fs::remove_file(&path);
        let (tx, _rx) = mpsc::unbounded_channel();
        spawn(path.clone(), tx.clone()).expect("bind IPC socket");
        let error = spawn(path.clone(), tx).expect_err("second bind must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        cleanup(&path);
    }
}
