use crate::paths::{daemon_addr, now_ms};
use crate::protocol::{
    generic_daemon_error, AgentError, IpcRequest, IpcResponse, JobEvent, JobSnapshot,
    JobStatus, StartRecordingParams,
};
use crate::PROTOCOL_VERSION;
use domain::{CaptureTarget, ScreenRecordingPermissionStatus};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(debug_assertions)]
const CARGO_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const IPC_READ_TIMEOUT: Duration = Duration::from_secs(10);
const IPC_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

#[allow(dead_code)]
struct DaemonLaunch {
    program: PathBuf,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    startup_timeout: Duration,
}

impl DaemonLaunch {
    fn executable(path: PathBuf) -> Self {
        Self {
            program: path,
            args: Vec::new(),
            envs: Vec::new(),
            startup_timeout: STARTUP_TIMEOUT,
        }
    }

    fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        for (key, value) in &self.envs {
            command.env(key, value);
        }
        command
    }
}

#[derive(Clone, Debug, Default)]
pub struct DaemonClient;

impl DaemonClient {
    pub fn new() -> Self {
        Self
    }

    pub fn ensure(&self) -> Result<(), AgentError> {
        ensure_daemon()
    }

    pub fn send_request(&self, method: &str, params: Value) -> Result<IpcResponse, AgentError> {
        let addr = daemon_addr();
        let mut stream = TcpStream::connect(&addr).map_err(|err| AgentError {
            code: "daemon_unreachable".into(),
            message: format!("Could not connect to daemon at {addr}: {err}"),
            recoverable: true,
            next: "Run `wrec daemon serve` or ensure the daemon is running.".into(),
        })?;
        stream
            .set_write_timeout(Some(IPC_WRITE_TIMEOUT))
            .map_err(|err| AgentError {
                code: "request_timeout_config_failed".into(),
                message: format!("Could not configure IPC write timeout: {err}"),
                recoverable: true,
                next: "Retry the command; if it repeats, restart the daemon.".into(),
            })?;
        stream
            .set_read_timeout(Some(IPC_READ_TIMEOUT))
            .map_err(|err| AgentError {
                code: "response_timeout_config_failed".into(),
                message: format!("Could not configure IPC read timeout: {err}"),
                recoverable: true,
                next: "Retry the command; if it repeats, restart the daemon.".into(),
            })?;

        let request = IpcRequest {
            id: now_ms(),
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&request).map_err(|err| AgentError {
            code: "request_encode_failed".into(),
            message: err.to_string(),
            recoverable: false,
            next: "Report this as a wrec IPC serialization bug.".into(),
        })?;
        stream
            .write_all(line.as_bytes())
            .map_err(|err| AgentError {
                code: "request_write_failed".into(),
                message: format!("Could not write IPC request: {err}"),
                recoverable: true,
                next: "Retry the command; if it repeats, run `wrec daemon status`.".into(),
            })?;
        stream.write_all(b"\n").map_err(|err| AgentError {
            code: "request_write_failed".into(),
            message: format!("Could not finish IPC request: {err}"),
            recoverable: true,
            next: "Retry the command; if it repeats, run `wrec daemon status`.".into(),
        })?;
        stream.flush().map_err(|err| AgentError {
            code: "request_write_failed".into(),
            message: format!("Could not flush IPC request: {err}"),
            recoverable: true,
            next: "Retry the command; if it repeats, run `wrec daemon status`.".into(),
        })?;

        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .map_err(|err| AgentError {
                code: "response_read_failed".into(),
                message: format!("Could not read IPC response: {err}"),
                recoverable: true,
                next: "Retry the command; if it repeats, restart the daemon.".into(),
            })?;
        if response.is_empty() {
            return Err(AgentError {
                code: "response_read_failed".into(),
                message: "Daemon closed the IPC stream without a response.".into(),
                recoverable: true,
                next: "Retry the command; if it repeats, restart the daemon.".into(),
            });
        }
        serde_json::from_str(&response).map_err(|err| AgentError {
            code: "response_decode_failed".into(),
            message: format!("Could not decode IPC response: {err}"),
            recoverable: false,
            next: "Inspect ~/.wrec/daemon.log and report this as a wrec IPC protocol bug.".into(),
        })
    }

    pub fn status(&self) -> Result<Value, AgentError> {
        self.request_result("daemon.status", json!({}))
    }

    pub fn stop_daemon(&self) -> Result<Value, AgentError> {
        self.request_result("daemon.stop", json!({}))
    }

    pub fn screen_recording_permission_status(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        let result = self.request_result("permission.status", json!({}))?;
        decode_result_field(result, "status", "permission_status_decode_failed")
    }

    pub fn request_screen_recording_permission(
        &self,
    ) -> Result<ScreenRecordingPermissionStatus, AgentError> {
        let result = self.request_result("permission.request", json!({}))?;
        decode_result_field(result, "status", "permission_status_decode_failed")
    }

    pub fn list_targets(&self) -> Result<Vec<CaptureTarget>, AgentError> {
        let result = self.request_result("targets.list", json!({}))?;
        serde_json::from_value(result.get("targets").cloned().unwrap_or_else(|| json!([])))
            .map_err(|err| protocol_mismatch("targets_decode_failed", err))
    }

    pub fn start_recording(&self, params: StartRecordingParams) -> Result<JobSnapshot, AgentError> {
        let value = serde_json::to_value(params).map_err(|err| AgentError {
            code: "record_request_encode_failed".into(),
            message: format!("Could not encode record.start request: {err}"),
            recoverable: false,
            next: "Report this as a wrec IPC serialization bug.".into(),
        })?;
        self.request_job("record.start", value)
    }

    pub fn list_jobs(&self) -> Result<Vec<JobSnapshot>, AgentError> {
        let result = self.request_result("jobs.list", json!({}))?;
        serde_json::from_value(result.get("jobs").cloned().unwrap_or_else(|| json!([])))
            .map_err(|err| protocol_mismatch("jobs_decode_failed", err))
    }

    pub fn show_job(&self, job_id: u64) -> Result<JobSnapshot, AgentError> {
        self.request_job("job.show", json!({ "job_id": job_id }))
    }

    pub fn job_logs(&self, job_id: u64) -> Result<Vec<JobEvent>, AgentError> {
        let result = self.request_result("job.logs", json!({ "job_id": job_id }))?;
        serde_json::from_value(result.get("events").cloned().unwrap_or_else(|| json!([])))
            .map_err(|err| protocol_mismatch("job_logs_decode_failed", err))
    }

    pub fn pause_job(&self, job_id: u64) -> Result<JobSnapshot, AgentError> {
        self.request_job("job.pause", json!({ "job_id": job_id }))
    }

    pub fn resume_job(&self, job_id: u64) -> Result<JobSnapshot, AgentError> {
        self.request_job("job.resume", json!({ "job_id": job_id }))
    }

    pub fn stop_job(&self, job_id: u64) -> Result<JobSnapshot, AgentError> {
        self.request_job("job.stop", json!({ "job_id": job_id }))
    }

    pub fn cancel_job(&self, job_id: u64) -> Result<JobSnapshot, AgentError> {
        self.request_job("job.cancel", json!({ "job_id": job_id }))
    }

    fn request_job(&self, method: &str, params: Value) -> Result<JobSnapshot, AgentError> {
        let result = self.request_result(method, params)?;
        serde_json::from_value(result.get("job").cloned().unwrap_or(Value::Null))
            .map_err(|err| protocol_mismatch("job_decode_failed", err))
    }

    fn request_result(&self, method: &str, params: Value) -> Result<Value, AgentError> {
        let response = self.send_request(method, params)?;
        if response.ok {
            Ok(response.result.unwrap_or_else(|| json!({})))
        } else {
            Err(response.error.unwrap_or_else(generic_daemon_error))
        }
    }
}

pub fn ensure_daemon() -> Result<(), AgentError> {
    let client = DaemonClient::new();
    if let Ok(status) = client.status() {
        validate_daemon_status(&status)?;
        return Ok(());
    }

    Err(AgentError {
        code: "daemon_unreachable".into(),
        message: "wrec daemon is not running. Start it with `wrec daemon serve`.".into(),
        recoverable: true,
        next: "Run `wrec daemon serve` in a terminal or set up the daemon as a service."
            .into(),
    })
}

pub fn run_daemon_foreground() -> Result<(), AgentError> {
    let status = daemon_launch()?
        .command()
        .status()
        .map_err(|err| AgentError {
            code: "daemon_start_failed".into(),
            message: format!("Could not run wrec daemon: {err}"),
            recoverable: true,
            next: "Set WREC_DAEMON_BIN to a daemon executable and retry.".into(),
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(AgentError {
            code: "daemon_exited".into(),
            message: format!("wrec daemon exited with {status}"),
            recoverable: true,
            next: "Inspect ~/.wrec/daemon.log and retry.".into(),
        })
    }
}

pub fn send_request(method: &str, params: Value) -> Result<IpcResponse, AgentError> {
    DaemonClient::new().send_request(method, params)
}

pub fn wait_for_job(job_id: u64, json_output: bool) -> Result<JobSnapshot, AgentError> {
    let client = DaemonClient::new();
    let mut seen_events = 0;
    loop {
        let job = client.show_job(job_id)?;
        for event in job.events.iter().skip(seen_events) {
            emit_job_event(json_output, job.id, event);
        }
        seen_events = job.events.len();

        if matches!(
            job.status,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
        ) {
            return Ok(job);
        }
        thread::sleep(WAIT_POLL_INTERVAL);
    }
}

pub fn emit_error(error: &AgentError, json_output: bool) {
    if json_output {
        println!(
            "{}",
            json!({
                "event": "error",
                "code": error.code,
                "message": error.message,
                "recoverable": error.recoverable,
                "next": error.next,
            })
        );
    } else {
        eprintln!("error: {}", error.message);
        eprintln!("next: {}", error.next);
    }
}

pub fn emit_job_event(json_output: bool, job_id: u64, event: &JobEvent) {
    if json_output {
        println!(
            "{}",
            json!({
                "event": "job_event",
                "job_id": job_id,
                "level": event.level,
                "message": event.message,
                "metrics": event.metrics,
                "timestamp_ms": event.timestamp_ms,
            })
        );
    } else {
        println!("{}", event.message);
    }
}

fn decode_result_field<T: serde::de::DeserializeOwned>(
    result: Value,
    field: &str,
    code: &str,
) -> Result<T, AgentError> {
    serde_json::from_value(result.get(field).cloned().unwrap_or(Value::Null))
        .map_err(|err| protocol_mismatch(code, err))
}

fn protocol_mismatch(code: &str, err: serde_json::Error) -> AgentError {
    AgentError {
        code: code.into(),
        message: format!("Could not decode daemon response: {err}"),
        recoverable: false,
        next: "Inspect ~/.wrec/daemon.log and report this as a wrec IPC protocol bug.".into(),
    }
}

fn validate_daemon_status(status: &Value) -> Result<(), AgentError> {
    let Some(protocol_version) = status.get("protocol_version").and_then(Value::as_u64) else {
        return Err(incompatible_daemon_error("missing protocol_version"));
    };

    if protocol_version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(incompatible_daemon_error(format!(
            "protocol_version {protocol_version}, expected {PROTOCOL_VERSION}"
        )))
    }
}

fn incompatible_daemon_error(reason: impl Into<String>) -> AgentError {
    AgentError {
        code: "daemon_incompatible".into(),
        message: format!("Running daemon is incompatible: {}", reason.into()),
        recoverable: true,
        next: "Stop the daemon with `wrec daemon stop`, then retry with matching app/CLI/runtime versions.".into(),
    }
}

fn daemon_launch() -> Result<DaemonLaunch, AgentError> {
    if let Some(path) = std::env::var_os("WREC_DAEMON_BIN").map(PathBuf::from) {
        return Ok(DaemonLaunch::executable(path));
    }

    let current = std::env::current_exe().map_err(|err| AgentError {
        code: "daemon_start_failed".into(),
        message: format!("Could not locate current executable: {err}"),
        recoverable: false,
        next: "Set WREC_DAEMON_BIN to a daemon executable and retry.".into(),
    })?;

    let candidates = daemon_candidates(&current);
    let installed_daemon = default_daemon_path();

    if let Some(launch) = candidates
        .iter()
        .filter(|path| **path != installed_daemon)
        .find_map(|path| daemon_executable_launch(path.clone()))
    {
        return Ok(launch);
    }

    if let Some(launch) = dev_cargo_daemon_launch() {
        return Ok(launch);
    }

    if let Some(launch) = daemon_executable_launch(installed_daemon) {
        return Ok(launch);
    }

    Err(AgentError {
        code: "daemon_start_failed".into(),
        message: "Could not locate the wrec daemon executable.".into(),
        recoverable: true,
        next: "Build the daemon through Cargo, install the full wrec runtime, or set WREC_DAEMON_BIN to the daemon executable.".into(),
    })
}

fn daemon_candidates(current: &Path) -> Vec<PathBuf> {
    let Some(current_dir) = current.parent() else {
        return vec![default_daemon_path()];
    };
    let profile_dir = current_dir
        .parent()
        .filter(|_| current_dir.file_name().is_some_and(|name| name == "deps"));

    [
        Some(current_dir.join("daemon.exe")),
        profile_dir.map(|dir| dir.join("daemon.exe")),
        Some(default_daemon_path()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn default_daemon_path() -> PathBuf {
    PathBuf::from(std::env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".into()))
        .join("Wrec")
        .join("daemon.exe")
}

fn daemon_executable_launch(path: PathBuf) -> Option<DaemonLaunch> {
    if !path.is_file() {
        return None;
    }
    Some(DaemonLaunch::executable(path))
}

fn cargo_profile_dir(executable: &Path) -> Option<&Path> {
    let dir = executable.parent()?;
    if dir.file_name().is_some_and(|name| name == "deps") {
        return dir.parent();
    }

    matches!(
        dir.file_name().and_then(|name| name.to_str()),
        Some("debug" | "release")
    )
    .then_some(dir)
}

fn dev_cargo_daemon_launch() -> Option<DaemonLaunch> {
    #[cfg(debug_assertions)]
    {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest_dir.parent()?.parent()?;
        let manifest = workspace.join("Cargo.toml");
        if !manifest.is_file() {
            return None;
        }

        Some(DaemonLaunch {
            program: std::env::var_os("CARGO")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("cargo")),
            args: vec![
                "run".into(),
                "--manifest-path".into(),
                manifest.into_os_string(),
                "-p".into(),
                "daemon".into(),
                "--bin".into(),
                "daemon".into(),
                "--".into(),
            ],
            envs: Vec::new(),
            startup_timeout: CARGO_STARTUP_TIMEOUT,
        })
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::daemon_candidates;
    use std::path::PathBuf;

    #[test]
    fn daemon_candidates_prefer_sibling_daemon() {
        let path = PathBuf::from(r"C:\wrec\bin\wrec.exe");
        let candidates = daemon_candidates(&path);

        assert_eq!(candidates[0], PathBuf::from(r"C:\wrec\bin\daemon.exe"));
    }

    #[test]
    fn daemon_candidates_include_profile_dir_for_deps_binary() {
        let path = PathBuf::from(r"C:\wrec\target\debug\deps\wrec-abc.exe");
        let candidates = daemon_candidates(&path);

        assert_eq!(
            candidates[0],
            PathBuf::from(r"C:\wrec\target\debug\deps\daemon.exe")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from(r"C:\wrec\target\debug\daemon.exe")
        );
    }

    #[test]
    fn daemon_executable_launch_accepts_any_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("wrec-control-daemon-{}", std::process::id()));
        let daemon = dir.join("daemon.exe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&daemon, "").unwrap();

        let launch = daemon_executable_launch(daemon.clone()).unwrap();

        assert_eq!(launch.program, daemon);

        let _ = std::fs::remove_dir_all(dir);
    }
}
