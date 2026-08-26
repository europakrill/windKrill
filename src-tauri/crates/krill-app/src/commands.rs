//! Tauri command layer: bridges the GUI frontend to krill sessions.
//!
//! One [`SessionHandle`] per tab, keyed by an opaque id. The frontend polls
//! `session_screen` for snapshots; push events land in M4 with the event
//! channel hardening.

use krill_core::{validate_screen_size, Screen, SnapshotDto};
use krill_session::{SessionHandle, SessionStatus};
#[cfg(not(windows))]
use krill_transport::local::LoopbackTransport;
use krill_transport::local::{ShellKind, SpawnOptions};
use krill_transport::Transport;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::State;

#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub id: u64,
    pub cols: u16,
    pub rows: u16,
    pub shell: String,
}

#[derive(Debug, Serialize)]
pub struct StatusDto {
    pub state: String,
    pub detail: Option<String>,
}

impl From<&SessionStatus> for StatusDto {
    fn from(status: &SessionStatus) -> Self {
        match status {
            SessionStatus::Running => Self {
                state: "running".into(),
                detail: None,
            },
            SessionStatus::Eof => Self {
                state: "eof".into(),
                detail: None,
            },
            SessionStatus::Closed => Self {
                state: "closed".into(),
                detail: None,
            },
            SessionStatus::Failed(message) => Self {
                state: "failed".into(),
                detail: Some(message.clone()),
            },
        }
    }
}

#[derive(Default)]
pub struct SessionRegistry {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<u64, SessionEntry>>,
}

struct SessionEntry {
    handle: SessionHandle,
    cols: u16,
    rows: u16,
    shell: String,
}

impl SessionRegistry {
    fn insert(&self, entry: SessionEntry) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.sessions.lock().unwrap().insert(id, entry);
        id
    }

    fn get(&self, id: u64) -> Result<SessionEntryClone, String> {
        let map = self.sessions.lock().unwrap();
        let entry = map.get(&id).ok_or_else(|| "no such session".to_string())?;
        Ok(SessionEntryClone {
            handle: entry.handle.clone(),
            cols: entry.cols,
            rows: entry.rows,
            shell: entry.shell.clone(),
        })
    }

    fn remove(&self, id: u64) -> Result<(), String> {
        self.sessions
            .lock()
            .unwrap()
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| "no such session".to_string())
    }
}

/// A cheap clone of the registry entry for use outside the lock.
struct SessionEntryClone {
    handle: SessionHandle,
    cols: u16,
    rows: u16,
    shell: String,
}

impl SessionEntryClone {
    fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
}

/// Spawn a new terminal session (one tab).
///
/// Windows hosts a real shell through ConPTY; other platforms fall back to
/// the loopback transport until their PTY adapter lands.
#[tauri::command]
pub async fn session_create(
    registry: State<'_, SessionRegistry>,
    cols: u16,
    rows: u16,
) -> Result<SessionInfo, String> {
    validate_screen_size(cols, rows).map_err(|error| error.to_string())?;
    let options = SpawnOptions {
        shell: ShellKind::Default,
        initial_cols: cols,
        initial_rows: rows,
    };

    #[cfg(windows)]
    let (transport, shell_name) = {
        use krill_transport::ConPtyTransport;
        let transport = ConPtyTransport::spawn(&options)
            .await
            .map_err(|error| error.to_string())?;
        (
            Box::new(transport) as Box<dyn Transport>,
            "conpty".to_string(),
        )
    };
    #[cfg(not(windows))]
    let (transport, shell_name) = (
        Box::new(LoopbackTransport::new(&options)) as Box<dyn Transport>,
        "loopback".to_string(),
    );

    let screen = Screen::try_new(cols, rows).map_err(|error| error.to_string())?;
    let handle = krill_session::spawn_session(transport, screen);
    let info = SessionInfo {
        id: 0,
        cols,
        rows,
        shell: shell_name.clone(),
    };
    let id = registry.insert(SessionEntry {
        handle,
        cols,
        rows,
        shell: shell_name,
    });
    Ok(SessionInfo { id, ..info })
}

/// Send user input (keystrokes or a paste) to a session.
#[tauri::command]
pub async fn session_input(
    registry: State<'_, SessionRegistry>,
    id: u64,
    data: Vec<u8>,
) -> Result<usize, String> {
    let session = registry.get(id)?;
    session
        .handle
        .send_input(&data)
        .await
        .map_err(|error| error.to_string())
}

/// Resize a session's PTY and screen.
#[tauri::command]
pub async fn session_resize(
    registry: State<'_, SessionRegistry>,
    id: u64,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let session = registry.get(id)?;
    session
        .handle
        .resize(cols, rows)
        .await
        .map_err(|error| error.to_string())
}

/// Fetch a serializable snapshot of the current screen plus session metadata.
#[tauri::command]
pub async fn session_screen(
    registry: State<'_, SessionRegistry>,
    id: u64,
) -> Result<ScreenEnvelope, String> {
    let session = registry.get(id)?;
    let (cols, rows) = session.size();
    let screen = session.handle.screen();
    Ok(ScreenEnvelope {
        id,
        cols,
        rows,
        shell: session.shell,
        snapshot: SnapshotDto::from_screen(&screen),
    })
}

#[derive(Debug, Serialize)]
pub struct ScreenEnvelope {
    pub id: u64,
    pub cols: u16,
    pub rows: u16,
    pub shell: String,
    pub snapshot: SnapshotDto,
}

/// Query a session's lifecycle status.
#[tauri::command]
pub async fn session_status(
    registry: State<'_, SessionRegistry>,
    id: u64,
) -> Result<StatusDto, String> {
    let session = registry.get(id)?;
    Ok(StatusDto::from(&session.handle.status()))
}

/// Close a session and drop it from the registry.
#[tauri::command]
pub async fn session_close(registry: State<'_, SessionRegistry>, id: u64) -> Result<(), String> {
    let session = registry.get(id)?;
    let result = session
        .handle
        .close()
        .await
        .map_err(|error| error.to_string());
    registry.remove(id)?;
    result
}

/// Register everything on the Tauri builder.
pub trait BuilderExt {
    fn register_krill_commands(self) -> Self;
}

impl BuilderExt for tauri::Builder<tauri::Wry> {
    fn register_krill_commands(self) -> Self {
        self.manage(SessionRegistry::default())
            .invoke_handler(tauri::generate_handler![
                session_create,
                session_input,
                session_resize,
                session_screen,
                session_status,
                session_close,
            ])
    }
}
