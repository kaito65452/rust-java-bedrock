use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::state::StudioState;

/// PTY output chunk emitted asynchronously via `"pty://data"`.
#[derive(Debug, Clone, Serialize)]
pub struct PtyDataEvent {
    pub session_id: u32,
    pub data: String,
}

/// PTY exit notification emitted via `"pty://exit"`.
#[derive(Debug, Clone, Serialize)]
pub struct PtyExitEvent {
    pub session_id: u32,
}

/// Active pseudo-terminal session managed by `StudioState`.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

/// Determines the default interactive shell executable for the host OS.
#[must_use]
pub fn default_shell_command() -> String {
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| String::from("powershell.exe"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty() && std::path::Path::new(s).exists())
            .unwrap_or_else(|| {
                if std::path::Path::new("/bin/bash").exists() {
                    String::from("/bin/bash")
                } else {
                    String::from("/bin/sh")
                }
            })
    }
}

/// Tauri Command: Spawn an interactive PTY terminal session (`$SHELL` or custom command like `yazi`).
#[tauri::command]
pub fn pty_spawn(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    cmd_opt: Option<String>,
) -> Result<u32, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.max(4),
            cols: cols.max(10),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed to open PTY: {e}"))?;

    let resolved_cwd = cwd
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| {
            state
                .active_pack
                .lock()
                .ok()
                .and_then(|g| g.as_ref().and_then(|p| p.source_path.clone()))
                .map(|p| {
                    if p.is_dir() {
                        p
                    } else {
                        p.parent()
                            .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf)
                    }
                })
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut cmd = match cmd_opt.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some("yazi") => {
            let mut c = CommandBuilder::new("yazi");
            c.arg(&resolved_cwd);
            c
        }
        Some(custom) => {
            let mut parts = custom.split_whitespace();
            let bin = parts.next().unwrap_or("sh");
            let mut c = CommandBuilder::new(bin);
            for arg in parts {
                c.arg(arg);
            }
            c
        }
        None => CommandBuilder::new(default_shell_command()),
    };

    cmd.cwd(&resolved_cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to spawn command in PTY: {e}"))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("failed cloning PTY reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("failed taking PTY writer: {e}"))?;

    let session_id = state.next_pty_id.fetch_add(1, Ordering::Relaxed);
    {
        let mut sessions = state
            .pty_sessions
            .lock()
            .map_err(|_| String::from("PTY sessions mutex poisoned"))?;
        sessions.insert(
            session_id,
            PtySession {
                master: pair.master,
                writer,
                child,
            },
        );
    }

    // Stream PTY output asynchronously to xterm.js via Tauri events
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    // Decode complete UTF-8 prefix so multi-byte glyphs never split across chunks
                    match std::str::from_utf8(&pending) {
                        Ok(valid) => {
                            let _ = app.emit(
                                "pty://data",
                                PtyDataEvent {
                                    session_id,
                                    data: valid.to_string(),
                                },
                            );
                            pending.clear();
                        }
                        Err(err) => {
                            let valid_up_to = err.valid_up_to();
                            if valid_up_to > 0 {
                                let valid = String::from_utf8_lossy(&pending[..valid_up_to])
                                    .into_owned();
                                let _ = app.emit(
                                    "pty://data",
                                    PtyDataEvent {
                                        session_id,
                                        data: valid,
                                    },
                                );
                                pending.drain(..valid_up_to);
                            } else if err.error_len().is_some() {
                                let lossy = String::from_utf8_lossy(&pending).into_owned();
                                let _ = app.emit(
                                    "pty://data",
                                    PtyDataEvent {
                                        session_id,
                                        data: lossy,
                                    },
                                );
                                pending.clear();
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let _ = app.emit("pty://exit", PtyExitEvent { session_id });
    });

    Ok(session_id)
}

/// Tauri Command: Write raw keyboard / command input into an active PTY session.
#[tauri::command]
pub fn pty_write(
    state: tauri::State<'_, StudioState>,
    session_id: u32,
    data: String,
) -> Result<(), String> {
    let mut sessions = state
        .pty_sessions
        .lock()
        .map_err(|_| String::from("PTY sessions mutex poisoned"))?;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| format!("PTY session {session_id} not found"))?;
    session
        .writer
        .write_all(data.as_bytes())
        .and_then(|()| session.writer.flush())
        .map_err(|e| format!("failed writing to PTY {session_id}: {e}"))
}

/// Tauri Command: Resize an active PTY session to match `xterm-addon-fit` dimensions.
#[tauri::command]
pub fn pty_resize(
    state: tauri::State<'_, StudioState>,
    session_id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = state
        .pty_sessions
        .lock()
        .map_err(|_| String::from("PTY sessions mutex poisoned"))?;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| format!("PTY session {session_id} not found"))?;
    session
        .master
        .resize(PtySize {
            rows: rows.max(4),
            cols: cols.max(10),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed resizing PTY {session_id}: {e}"))
}

/// Tauri Command: Terminate and clean up an active PTY session.
#[tauri::command]
pub fn pty_kill(state: tauri::State<'_, StudioState>, session_id: u32) -> Result<(), String> {
    let mut sessions = state
        .pty_sessions
        .lock()
        .map_err(|_| String::from("PTY sessions mutex poisoned"))?;
    if let Some(mut session) = sessions.remove(&session_id) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
    Ok(())
}
