use anyhow::Context;

#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid as NixPid;

pub struct KillResult {
    pub pid: u32,
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
}

pub fn kill_process(pid: u32, name: &str, force: bool) -> KillResult {
    match kill_process_inner(pid, force) {
        Ok(()) => KillResult {
            pid,
            name: name.to_string(),
            success: true,
            error: None,
        },
        Err(e) => KillResult {
            pid,
            name: name.to_string(),
            success: false,
            error: Some(e.to_string()),
        },
    }
}

#[cfg(unix)]
fn kill_process_inner(pid: u32, force: bool) -> anyhow::Result<()> {
    let signal = if force { Signal::SIGKILL } else { Signal::SIGTERM };
    kill(NixPid::from_raw(pid as i32), signal)
        .with_context(|| format!("Failed to kill PID {}", pid))?;
    Ok(())
}

#[cfg(not(unix))]
fn kill_process_inner(pid: u32, _force: bool) -> anyhow::Result<()> {
    // Windows fallback using taskkill
    let output = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
        .with_context(|| format!("Failed to run taskkill for PID {}", pid))?;
    if !output.status.success() {
        anyhow::bail!(
            "taskkill failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[allow(dead_code)]
pub fn kill_processes(
    targets: &[(u32, String)],
    force: bool,
    dry_run: bool,
) -> Vec<KillResult> {
    if dry_run {
        return targets
            .iter()
            .map(|(pid, name)| KillResult {
                pid: *pid,
                name: name.clone(),
                success: true,
                error: None,
            })
            .collect();
    }

    targets
        .iter()
        .map(|(pid, name)| kill_process(*pid, name, force))
        .collect()
}

/// Send SIGTERM then wait, then SIGKILL if needed
#[cfg(unix)]
pub fn graceful_kill(pid: u32, name: &str, timeout_secs: u64) -> KillResult {
    use std::time::{Duration, Instant};

    // Send SIGTERM first
    if let Err(e) = kill(NixPid::from_raw(pid as i32), Signal::SIGTERM) {
        return KillResult {
            pid,
            name: name.to_string(),
            success: false,
            error: Some(e.to_string()),
        };
    }

    // Wait for process to die
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        std::thread::sleep(Duration::from_millis(200));
        if !process_exists(pid) {
            return KillResult {
                pid,
                name: name.to_string(),
                success: true,
                error: None,
            };
        }
        if Instant::now() >= deadline {
            break;
        }
    }

    // Escalate to SIGKILL
    match kill(NixPid::from_raw(pid as i32), Signal::SIGKILL) {
        Ok(()) => KillResult {
            pid,
            name: name.to_string(),
            success: true,
            error: None,
        },
        Err(e) => KillResult {
            pid,
            name: name.to_string(),
            success: false,
            error: Some(format!("SIGKILL failed: {}", e)),
        },
    }
}

#[cfg(not(unix))]
pub fn graceful_kill(pid: u32, name: &str, _timeout_secs: u64) -> KillResult {
    kill_process(pid, name, true)
}

#[allow(dead_code)]
fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        match kill(NixPid::from_raw(pid as i32), None) {
            Ok(()) => true,
            Err(Errno::ESRCH) => false,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}
