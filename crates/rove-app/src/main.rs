use std::time::{Instant, SystemTime, UNIX_EPOCH};
fn main() {
    let started = process_start();
    println!(
        "MAIN_EPOCH_MS {}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let strings = rove_ui::skeleton::appearance::strings().unwrap();
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!(
            "{}",
            rove_ui::skeleton::appearance::message(&strings, "app.usage", &[])
        );
        std::process::exit(2)
    };
    rove_ui::skeleton::launch(path.into(), started);
}

/// Include loader time before main on the supported Windows platform.
fn process_start() -> Instant {
    let now = Instant::now();
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::FILETIME,
            System::Threading::{GetCurrentProcess, GetProcessTimes},
        };
        let mut created: FILETIME = unsafe { std::mem::zeroed() };
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        // SAFETY: current-process pseudo-handle is valid and all four FILETIMEs are writable.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if ok != 0 {
            let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            // FILETIME is 100ns since 1601, Unix epoch starts in 1970.
            if let Some(ticks) = ticks.checked_sub(116_444_736_000_000_000) {
                let start = UNIX_EPOCH + std::time::Duration::from_nanos(ticks * 100);
                if let Ok(age) = SystemTime::now().duration_since(start) {
                    if let Some(instant) = now.checked_sub(age) {
                        return instant;
                    }
                }
            }
        }
        eprintln!("PROCESS_CLOCK_UNAVAILABLE: falling back to main entry");
    }
    now
}
