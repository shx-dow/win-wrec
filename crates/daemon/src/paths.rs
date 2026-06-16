use control::{daemon_log_path, job_events_path, now_ms, JobEvent};
use serde_json::json;
use std::{fs::OpenOptions, io::Write, path::Path};

pub(crate) fn append_daemon_log(message: impl AsRef<str>) {
    append_line(
        &daemon_log_path(),
        &format!("{} {}", now_ms(), message.as_ref()),
    );
}

pub(crate) fn append_job_event(job_id: u64, event: &JobEvent) {
    if let Ok(value) = serde_json::to_string(&json!({
        "job_id": job_id,
        "event": event,
    })) {
        append_line(&job_events_path(), &value);
    }
}

pub(crate) fn append_line(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}
