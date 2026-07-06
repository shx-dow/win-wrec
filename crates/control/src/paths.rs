use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const SOCKET_NAME: &str = "wrec.sock";
const DAEMON_LOG_NAME: &str = "daemon.log";
const JOB_EVENTS_NAME: &str = "job-events.jsonl";

pub fn wrec_home() -> PathBuf {
    std::env::var_os("WREC_HOME")
        .map(PathBuf::from)
        .or_else(default_wrec_home)
        .unwrap_or_else(|| PathBuf::from(".wrec"))
}

pub fn socket_path() -> PathBuf {
    wrec_home().join(SOCKET_NAME)
}

pub fn daemon_log_path() -> PathBuf {
    wrec_home().join(DAEMON_LOG_NAME)
}

pub fn job_events_path() -> PathBuf {
    wrec_home().join(JOB_EVENTS_NAME)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "windows")]
fn default_wrec_home() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|dir| dir.join("Wrec"))
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|home| home.join(".wrec"))
        })
}

#[cfg(not(target_os = "windows"))]
fn default_wrec_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".wrec"))
}
