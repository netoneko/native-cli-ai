//! Opt-in timing instrumentation for chasing the multi-minute input-freeze
//! bug (see `docs/archive/SCHEDULING_INVESTIGATION.md`). Off by default —
//! set `NCA_TUI_TIMING_LOG=1` to append timestamped lines to
//! `/tmp/nca-tui-timing.log`. Not wired to `tracing`: the TUI takes over the
//! terminal, so stderr-based logging is invisible/corrupting, and this needs
//! to survive regardless of `RUST_LOG`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("NCA_TUI_TIMING_LOG").ok().as_deref() == Some("1"))
}

fn file() -> Option<&'static Mutex<File>> {
    static CELL: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    CELL.get_or_init(|| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/nca-tui-timing.log")
            .ok()
            .map(Mutex::new)
    })
    .as_ref()
}

/// Append one timestamped line. No-op unless `NCA_TUI_TIMING_LOG=1`.
pub fn log(tag: &str, msg: &str) {
    if !enabled() {
        return;
    }
    let Some(m) = file() else { return };
    let Ok(mut f) = m.lock() else { return };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let _ = writeln!(f, "[{}.{:03}] {tag}: {msg}", now.as_secs(), now.subsec_millis());
}
