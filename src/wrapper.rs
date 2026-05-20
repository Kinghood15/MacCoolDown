use anyhow::{Context, Result};
use colored::*;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid as NixPid;

use crate::thermal::{get_thermal_status, ThermalLevel};

#[derive(Debug, Clone)]
pub struct WrapConfig {
    pub cpu_limit: Option<u32>,      // Max CPU% (uses throttling)
    pub thermal_limit: ThermalLevel, // Kill if thermal exceeds this
    pub timeout: Option<Duration>,   // Max runtime
    pub check_interval: Duration,    // How often to check
}

impl Default for WrapConfig {
    fn default() -> Self {
        Self {
            cpu_limit: None,
            thermal_limit: ThermalLevel::Critical,
            timeout: None,
            check_interval: Duration::from_secs(5),
        }
    }
}

#[derive(Debug)]
pub struct WrapResult {
    pub exit_code: Option<i32>,
    pub killed_by_thermal: bool,
    pub killed_by_timeout: bool,
    pub runtime: Duration,
}

fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        match kill(NixPid::from_raw(pid as i32), None) {
            Ok(()) => true,
            Err(Errno::ESRCH) => false,
            Err(_) => true,
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

#[cfg(unix)]
fn kill_process(pid: u32) -> Result<()> {
    kill(NixPid::from_raw(pid as i32), Signal::SIGTERM)
        .with_context(|| format!("Failed to SIGTERM PID {}", pid))?;
    Ok(())
}

#[cfg(not(unix))]
fn stop_process(_pid: u32) -> Result<()> {
    anyhow::bail!("SIGSTOP not supported")
}

#[cfg(not(unix))]
fn cont_process(_pid: u32) -> Result<()> {
    anyhow::bail!("SIGCONT not supported")
}

#[cfg(not(unix))]
fn kill_process(_pid: u32) -> Result<()> {
    anyhow::bail!("SIGTERM not supported")
}

fn spawn_command(cmd: &str, args: &[String]) -> Result<Child> {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("Failed to spawn: {} {:?}", cmd, args))
}

pub fn wrap_command(
    cmd: &str,
    args: &[String],
    config: WrapConfig,
    running: Arc<AtomicBool>,
) -> Result<WrapResult> {
    let start_time = Instant::now();

    println!();
    println!(
        "  {} {} {}",
        "WRAP".cyan().bold(),
        cmd.green(),
        args.join(" ").dimmed()
    );

    if let Some(limit) = config.cpu_limit {
        println!("  {} {}%", "CPU limit:".dimmed(), limit);
    }
    println!(
        "  {} {:?}",
        "Thermal limit:".dimmed(),
        config.thermal_limit.label()
    );
    if let Some(timeout) = config.timeout {
        println!("  {} {:?}", "Timeout:".dimmed(), timeout);
    }
    println!();

    let mut child = spawn_command(cmd, args)?;
    let child_pid = child.id();

    let mut killed_by_thermal = false;
    let mut killed_by_timeout = false;

    // Throttling state
    let cpu_limit = config.cpu_limit.unwrap_or(100);
    let window_ms = 100u64;
    let run_ms = (window_ms * cpu_limit as u64) / 100;
    let stop_ms = window_ms - run_ms;
    let should_throttle = cpu_limit < 100;

    let mut last_thermal_check = Instant::now();
    let mut is_stopped = false;

    while running.load(Ordering::SeqCst) {
        // Check if process has exited
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(WrapResult {
                    exit_code: status.code(),
                    killed_by_thermal,
                    killed_by_timeout,
                    runtime: start_time.elapsed(),
                });
            }
            Ok(None) => {
                // Still running
            }
            Err(e) => {
                anyhow::bail!("Failed to check process status: {}", e);
            }
        }

        // Check timeout
        if let Some(timeout) = config.timeout {
            if start_time.elapsed() >= timeout {
                println!(
                    "\n  {} Timeout reached, killing process...",
                    "TIMEOUT".yellow().bold()
                );
                killed_by_timeout = true;
                let _ = kill_process(child_pid);
                let _ = child.wait();
                return Ok(WrapResult {
                    exit_code: None,
                    killed_by_thermal,
                    killed_by_timeout,
                    runtime: start_time.elapsed(),
                });
            }
        }

        // Periodic thermal check
        if last_thermal_check.elapsed() >= config.check_interval {
            last_thermal_check = Instant::now();

            if let Ok(status) = get_thermal_status() {
                let current_level = status.level;
                let limit_level = &config.thermal_limit;

                let should_kill = match (current_level, limit_level) {
                    (ThermalLevel::Critical, _) => true,
                    (ThermalLevel::Heavy, ThermalLevel::Heavy) => true,
                    (ThermalLevel::Heavy, ThermalLevel::Moderate) => true,
                    (ThermalLevel::Moderate, ThermalLevel::Moderate) => true,
                    _ => false,
                };

                if should_kill {
                    println!(
                        "\n  {} Thermal level {} exceeded limit {}, killing process...",
                        "THERMAL".red().bold(),
                        current_level.colored_label(),
                        limit_level.label().yellow()
                    );
                    killed_by_thermal = true;

                    // Resume if stopped before killing
                    if is_stopped {
                        let _ = cont_process(child_pid);
                    }
                    let _ = kill_process(child_pid);
                    let _ = child.wait();

                    return Ok(WrapResult {
                        exit_code: None,
                        killed_by_thermal,
                        killed_by_timeout,
                        runtime: start_time.elapsed(),
                    });
                }
            }
        }

        // CPU throttling via SIGSTOP/SIGCONT
        if should_throttle && process_exists(child_pid) {
            // Run phase
            if is_stopped {
                let _ = cont_process(child_pid);
                is_stopped = false;
            }
            std::thread::sleep(Duration::from_millis(run_ms));

            // Stop phase
            if process_exists(child_pid) {
                let _ = stop_process(child_pid);
                is_stopped = true;
                std::thread::sleep(Duration::from_millis(stop_ms));
            }
        } else {
            // No throttling, just sleep and check again
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // Interrupted by Ctrl+C
    println!(
        "\n  {} Interrupted, terminating wrapped process...",
        "INTERRUPT".yellow().bold()
    );

    if is_stopped {
        let _ = cont_process(child_pid);
    }
    let _ = kill_process(child_pid);
    let _ = child.wait();

    Ok(WrapResult {
        exit_code: None,
        killed_by_thermal: false,
        killed_by_timeout: false,
        runtime: start_time.elapsed(),
    })
}

pub fn run_wrap(
    command: Vec<String>,
    cpu_limit: Option<u32>,
    thermal_limit: Option<String>,
    timeout_secs: Option<u64>,
) -> Result<()> {
    if command.is_empty() {
        anyhow::bail!("No command specified");
    }

    let cmd = command[0].clone();
    let args: Vec<String> = command.into_iter().skip(1).collect();

    let thermal_limit = match thermal_limit.as_deref() {
        Some("moderate") => ThermalLevel::Moderate,
        Some("heavy") => ThermalLevel::Heavy,
        Some("critical") | None => ThermalLevel::Critical,
        Some(other) => {
            println!(
                "  {} Unknown thermal level '{}', using 'critical'",
                "WARN".yellow().bold(),
                other
            );
            ThermalLevel::Critical
        }
    };

    let config = WrapConfig {
        cpu_limit,
        thermal_limit,
        timeout: timeout_secs.map(Duration::from_secs),
        check_interval: Duration::from_secs(5),
    };

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    let result = wrap_command(&cmd, &args, config, running)?;

    println!();
    if result.killed_by_thermal {
        println!(
            "  {} Process killed due to thermal limit",
            "THERMAL".red().bold()
        );
    } else if result.killed_by_timeout {
        println!(
            "  {} Process killed due to timeout",
            "TIMEOUT".yellow().bold()
        );
    } else if let Some(code) = result.exit_code {
        if code == 0 {
            println!("  {} Exited with code {}", "OK".green().bold(), code);
        } else {
            println!("  {} Exited with code {}", "EXIT".yellow().bold(), code);
        }
    } else {
        println!("  {} Process terminated", "DONE".dimmed());
    }

    println!(
        "  {} {:.1}s",
        "Runtime:".dimmed(),
        result.runtime.as_secs_f64()
    );
    println!();

    // Exit with same code as wrapped process
    if let Some(code) = result.exit_code {
        std::process::exit(code);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_config_default() {
        let config = WrapConfig::default();
        assert!(config.cpu_limit.is_none());
        assert_eq!(config.thermal_limit, ThermalLevel::Critical);
        assert!(config.timeout.is_none());
    }

    #[test]
    fn test_spawn_command_echo() {
        let mut child = spawn_command("echo", &["test".to_string()]).unwrap();
        let status = child.wait().unwrap();
        assert!(status.success());
    }
}
