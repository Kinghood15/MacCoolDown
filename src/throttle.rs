use anyhow::{Context, Result};
use colored::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid as NixPid;

#[derive(Debug, Clone)]
pub struct ThrottleConfig {
    pub pid: u32,
    pub cpu_limit: u32, // percentage 1-100
    pub duration: Option<Duration>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ThrottleResult {
    pub pid: u32,
    pub success: bool,
    pub cycles: u64,
    pub total_time: Duration,
    pub error: Option<String>,
}

fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        match kill(NixPid::from_raw(pid as i32), None) {
            Ok(()) => true,
            Err(Errno::ESRCH) => false,
            Err(_) => true, // Process exists but we can't signal it
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(unix)]
fn stop_process(pid: u32) -> Result<()> {
    kill(NixPid::from_raw(pid as i32), Signal::SIGSTOP)
        .with_context(|| format!("Failed to SIGSTOP PID {}", pid))?;
    Ok(())
}

#[cfg(unix)]
fn cont_process(pid: u32) -> Result<()> {
    kill(NixPid::from_raw(pid as i32), Signal::SIGCONT)
        .with_context(|| format!("Failed to SIGCONT PID {}", pid))?;
    Ok(())
}

#[cfg(not(unix))]
fn stop_process(_pid: u32) -> Result<()> {
    anyhow::bail!("SIGSTOP not supported on this platform")
}

#[cfg(not(unix))]
fn cont_process(_pid: u32) -> Result<()> {
    anyhow::bail!("SIGCONT not supported on this platform")
}

pub fn throttle_process(config: ThrottleConfig, running: Arc<AtomicBool>) -> ThrottleResult {
    let pid = config.pid;
    let cpu_limit = config.cpu_limit.clamp(1, 99);

    // Calculate duty cycle
    // If cpu_limit = 50, we run 50ms, stop 50ms (within 100ms window)
    let window_ms = 100u64;
    let run_ms = (window_ms * cpu_limit as u64) / 100;
    let stop_ms = window_ms - run_ms;

    let start_time = Instant::now();
    let mut cycles = 0u64;

    println!();
    println!(
        "  {} Throttling PID {} to {}% CPU",
        "THROTTLE".cyan().bold(),
        pid,
        cpu_limit
    );
    println!(
        "  {} Run {}ms / Stop {}ms per cycle",
        "Pattern:".dimmed(),
        run_ms,
        stop_ms
    );
    if let Some(dur) = config.duration {
        println!("  {} {:?}", "Duration:".dimmed(), dur);
    } else {
        println!("  {} Until Ctrl+C", "Duration:".dimmed());
    }
    println!();

    while running.load(Ordering::SeqCst) {
        // Check duration limit
        if let Some(max_duration) = config.duration {
            if start_time.elapsed() >= max_duration {
                break;
            }
        }

        // Check if process still exists
        if !process_exists(pid) {
            return ThrottleResult {
                pid,
                success: false,
                cycles,
                total_time: start_time.elapsed(),
                error: Some("Process no longer exists".to_string()),
            };
        }

        // Run phase
        if let Err(e) = cont_process(pid) {
            // Process might have terminated
            if !process_exists(pid) {
                break;
            }
            return ThrottleResult {
                pid,
                success: false,
                cycles,
                total_time: start_time.elapsed(),
                error: Some(e.to_string()),
            };
        }
        std::thread::sleep(Duration::from_millis(run_ms));

        // Stop phase
        if let Err(e) = stop_process(pid) {
            if !process_exists(pid) {
                break;
            }
            return ThrottleResult {
                pid,
                success: false,
                cycles,
                total_time: start_time.elapsed(),
                error: Some(e.to_string()),
            };
        }
        std::thread::sleep(Duration::from_millis(stop_ms));

        cycles += 1;

        // Print progress every 10 cycles
        if cycles % 10 == 0 {
            print!(
                "\r  {} {} cycles, {:.1}s elapsed",
                "Progress:".dimmed(),
                cycles,
                start_time.elapsed().as_secs_f64()
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    }

    // Ensure process is resumed when we exit
    let _ = cont_process(pid);

    println!();
    println!();

    ThrottleResult {
        pid,
        success: true,
        cycles,
        total_time: start_time.elapsed(),
        error: None,
    }
}

pub fn run_throttle(pid: u32, cpu_limit: u32, duration_secs: Option<u64>) -> Result<()> {
    if !process_exists(pid) {
        anyhow::bail!("Process {} does not exist", pid);
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    let config = ThrottleConfig {
        pid,
        cpu_limit,
        duration: duration_secs.map(Duration::from_secs),
    };

    let result = throttle_process(config, running);

    if result.success {
        println!(
            "  {} Throttling completed: {} cycles over {:.1}s",
            "OK".green().bold(),
            result.cycles,
            result.total_time.as_secs_f64()
        );
    } else {
        println!(
            "  {} {}",
            "ERR".red().bold(),
            result.error.unwrap_or_else(|| "Unknown error".to_string())
        );
    }

    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_exists_self() {
        let pid = std::process::id();
        assert!(process_exists(pid));
    }

    #[test]
    fn test_process_exists_invalid() {
        // Very unlikely to exist
        assert!(!process_exists(999999999));
    }
}
