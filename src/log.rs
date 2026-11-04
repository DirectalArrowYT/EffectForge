//! What Forge is doing, and how it died: `%APPDATA%/EffectForge/forge.log`.
//!
//! The log names the file and effect on screen, so a crash line says which one caused it. Panics and native faults
//! both land here, as do the GPU's own errors.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// A longer log is started over.
const MAX_BYTES: u64 = 4 << 20;

static CONTEXT: Mutex<String> = Mutex::new(String::new());
static STARTED: OnceLock<Instant> = OnceLock::new();

pub fn path() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("EffectForge").join("forge.log")
}

pub fn note(line: &str) {
    let elapsed = STARTED.get().map_or(0.0, |t| t.elapsed().as_secs_f32());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(file, "[{elapsed:8.2}s] {line}");
    }
}

/// Say what is on screen. Logged when it changes.
pub fn context(now: &str) {
    let Ok(mut context) = CONTEXT.lock() else { return };
    if *context != now {
        *context = now.to_string();
        drop(context);
        note(&format!("showing {now}"));
    }
}

fn last_context() -> String {
    CONTEXT.try_lock().map(|c| c.clone()).unwrap_or_else(|_| "(busy)".to_string())
}

pub fn install() {
    STARTED.get_or_init(Instant::now);
    let file = path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::remove_file(&file);
    }
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    note(&format!("--- Effect Forge started, unix time {unix}, version {}", env!("CARGO_PKG_VERSION")));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        note(&format!(
            "PANIC on thread {} while showing {}\n{info}\n{}",
            thread.name().unwrap_or("unnamed"),
            last_context(),
            std::backtrace::Backtrace::force_capture()
        ));
        default(info);
    }));
    #[cfg(windows)]
    native::install();
}

/// Faults Rust can't catch: bad memory access and stack overflow, in Forge or in a graphics driver.
#[cfg(windows)]
mod native {
    use std::ffi::c_void;

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        next: *const ExceptionRecord,
        address: *const c_void,
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *const ExceptionRecord,
        context: *const c_void,
    }

    type Filter = unsafe extern "system" fn(*const ExceptionPointers) -> i32;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
    }

    /// Log it, then let Windows end the process as it would have.
    unsafe extern "system" fn on_fault(pointers: *const ExceptionPointers) -> i32 {
        let record = unsafe { &*(*pointers).record };
        super::note(&format!("NATIVE FAULT {:#010x} at {:p} while showing {}", record.code, record.address, super::last_context()));
        0
    }

    pub fn install() {
        unsafe { SetUnhandledExceptionFilter(Some(on_fault)) };
    }
}
