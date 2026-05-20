use anyhow::{Context, Result};
use colored::*;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalLevel {
    Nominal,
    Moderate,
    Heavy,
    Critical,
    Unknown,
}

impl ThermalLevel {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "nominal" | "0" => ThermalLevel::Nominal,
            "moderate" | "1" => ThermalLevel::Moderate,
            "heavy" | "2" => ThermalLevel::Heavy,
            "critical" | "trapping" | "sleeping" | "3" | "4" => ThermalLevel::Critical,
            _ => ThermalLevel::Unknown,
        }
    }

    pub fn from_pressure(pressure: u32) -> Self {
        match pressure {
            0 => ThermalLevel::Nominal,
            1..=30 => ThermalLevel::Moderate,
            31..=60 => ThermalLevel::Heavy,
            _ => ThermalLevel::Critical,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ThermalLevel::Nominal => "Nominal",
            ThermalLevel::Moderate => "Moderate",
            ThermalLevel::Heavy => "Heavy",
            ThermalLevel::Critical => "Critical",
            ThermalLevel::Unknown => "Unknown",
        }
    }

    pub fn colored_label(&self) -> ColoredString {
        match self {
            ThermalLevel::Nominal => self.label().green(),
            ThermalLevel::Moderate => self.label().yellow(),
            ThermalLevel::Heavy => self.label().truecolor(255, 165, 0), // orange
            ThermalLevel::Critical => self.label().red().bold(),
            ThermalLevel::Unknown => self.label().dimmed(),
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            ThermalLevel::Nominal => "❄️",
            ThermalLevel::Moderate => "🌡️",
            ThermalLevel::Heavy => "🔥",
            ThermalLevel::Critical => "🚨",
            ThermalLevel::Unknown => "❓",
        }
    }

    pub fn is_dangerous(&self) -> bool {
        matches!(self, ThermalLevel::Heavy | ThermalLevel::Critical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    Battery,
    AC,
    Unknown,
}

impl PowerSource {
    pub fn label(&self) -> &'static str {
        match self {
            PowerSource::Battery => "Battery",
            PowerSource::AC => "AC Power",
            PowerSource::Unknown => "Unknown",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            PowerSource::Battery => "🔋",
            PowerSource::AC => "🔌",
            PowerSource::Unknown => "❓",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThermalStatus {
    pub level: ThermalLevel,
    pub cpu_temp: Option<f64>,
    pub gpu_temp: Option<f64>,
    pub fan_rpm: Option<u32>,
    pub cpu_power: Option<f64>,
    pub power_source: PowerSource,
    pub battery_percent: Option<u32>,
}

impl Default for ThermalStatus {
    fn default() -> Self {
        Self {
            level: ThermalLevel::Unknown,
            cpu_temp: None,
            gpu_temp: None,
            fan_rpm: None,
            cpu_power: None,
            power_source: PowerSource::Unknown,
            battery_percent: None,
        }
    }
}

pub fn get_power_source() -> (PowerSource, Option<u32>) {
    // Use pmset to get power source info
    let output = Command::new("pmset")
        .args(["-g", "batt"])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);

            let source = if stdout.contains("AC Power") {
                PowerSource::AC
            } else if stdout.contains("Battery Power") {
                PowerSource::Battery
            } else {
                PowerSource::Unknown
            };

            // Parse battery percentage
            let battery = stdout
                .lines()
                .find(|l| l.contains('%'))
                .and_then(|line| {
                    line.split_whitespace()
                        .find(|w| w.contains('%'))
                        .and_then(|w| w.replace('%', "").replace(';', "").parse::<u32>().ok())
                });

            return (source, battery);
        }
    }

    (PowerSource::Unknown, None)
}

pub fn get_thermal_status() -> Result<ThermalStatus> {
    let mut status = ThermalStatus::default();

    // Get power source first
    let (power_source, battery) = get_power_source();
    status.power_source = power_source;
    status.battery_percent = battery;

    // Try powermetrics first (requires sudo)
    if let Ok(pm_status) = get_thermal_from_powermetrics() {
        if pm_status.level != ThermalLevel::Unknown {
            status.level = pm_status.level;
            status.cpu_temp = pm_status.cpu_temp;
            status.gpu_temp = pm_status.gpu_temp;
            status.fan_rpm = pm_status.fan_rpm;
            status.cpu_power = pm_status.cpu_power;
            return Ok(status);
        }
    }

    // Fallback 1: thermal_pressure via sysctl (Apple Silicon)
    if let Ok(level) = get_thermal_from_sysctl() {
        status.level = level;
        return Ok(status);
    }

    // Fallback 2: Estimate from CPU usage
    if let Ok(level) = estimate_thermal_from_cpu() {
        status.level = level;
        return Ok(status);
    }

    Ok(status)
}

fn get_thermal_from_powermetrics() -> Result<ThermalStatus> {
    let output = Command::new("sudo")
        .args(["-n", "powermetrics", "-s", "thermal", "-n", "1", "-i", "100"])
        .output()
        .context("Failed to run powermetrics")?;

    if !output.status.success() {
        anyhow::bail!("powermetrics failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_powermetrics_output(&stdout)
}

fn get_thermal_from_sysctl() -> Result<ThermalLevel> {
    // Try machdep.xcpm.cpu_thermal_level (Intel)
    if let Ok(output) = Command::new("sysctl")
        .args(["-n", "machdep.xcpm.cpu_thermal_level"])
        .output()
    {
        if output.status.success() {
            let level_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Ok(level_num) = level_str.parse::<u32>() {
                return Ok(ThermalLevel::from_pressure(level_num));
            }
        }
    }

    // Try kern.thermal_state (generic)
    if let Ok(output) = Command::new("sysctl")
        .args(["-n", "kern.thermal_state"])
        .output()
    {
        if output.status.success() {
            let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Ok(ThermalLevel::from_str(&state));
        }
    }

    // Apple Silicon: check thermal pressure via host_statistics
    // This requires calling system APIs, so we estimate from load instead
    anyhow::bail!("No sysctl thermal info available")
}

fn estimate_thermal_from_cpu() -> Result<ThermalLevel> {
    // Get load average as proxy for thermal state
    let output = Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .context("Failed to get load average")?;

    if !output.status.success() {
        anyhow::bail!("sysctl failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Format: { 3.45 2.67 2.12 }
    let load: f64 = stdout
        .trim()
        .trim_matches(|c| c == '{' || c == '}')
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    // Get number of CPUs
    let num_cpus = num_cpus::get() as f64;
    let load_ratio = load / num_cpus;

    let level = if load_ratio < 0.5 {
        ThermalLevel::Nominal
    } else if load_ratio < 0.8 {
        ThermalLevel::Moderate
    } else if load_ratio < 1.2 {
        ThermalLevel::Heavy
    } else {
        ThermalLevel::Critical
    };

    Ok(level)
}

fn parse_powermetrics_output(output: &str) -> Result<ThermalStatus> {
    let mut status = ThermalStatus::default();

    for line in output.lines() {
        let line = line.trim();

        // Parse thermal level
        if line.starts_with("Thermal level:") {
            if let Some(level_str) = line.split(':').nth(1) {
                status.level = ThermalLevel::from_str(level_str.trim());
            }
        }

        // Parse CPU die temperature
        if line.starts_with("CPU die temperature:") {
            if let Some(temp_str) = line.split(':').nth(1) {
                let temp_str = temp_str.trim().replace(" C", "").replace("°", "");
                if let Ok(temp) = temp_str.parse::<f64>() {
                    status.cpu_temp = Some(temp);
                }
            }
        }

        // Parse GPU temperature
        if line.starts_with("GPU die temperature:") {
            if let Some(temp_str) = line.split(':').nth(1) {
                let temp_str = temp_str.trim().replace(" C", "").replace("°", "");
                if let Ok(temp) = temp_str.parse::<f64>() {
                    status.gpu_temp = Some(temp);
                }
            }
        }

        // Parse fan speed
        if line.contains("Fan:") || line.contains("fan speed:") {
            for word in line.split_whitespace() {
                if let Ok(rpm) = word.replace("rpm", "").parse::<u32>() {
                    status.fan_rpm = Some(rpm);
                    break;
                }
            }
        }

        // Parse CPU power
        if line.starts_with("CPU Power:") || line.contains("Package Power:") {
            for word in line.split_whitespace() {
                let cleaned = word.replace("mW", "").replace("W", "");
                if let Ok(power) = cleaned.parse::<f64>() {
                    status.cpu_power = Some(if word.contains("mW") {
                        power / 1000.0
                    } else {
                        power
                    });
                    break;
                }
            }
        }
    }

    Ok(status)
}

#[allow(dead_code)]
pub fn format_thermal_inline(status: &ThermalStatus) -> String {
    let mut parts = vec![];

    // Thermal level with emoji
    parts.push(format!("{} {}", status.level.emoji(), status.level.colored_label()));

    // Temperature if available
    if let Some(temp) = status.cpu_temp {
        let temp_str = if temp > 90.0 {
            format!("{:.0}°C", temp).red().to_string()
        } else if temp > 75.0 {
            format!("{:.0}°C", temp).yellow().to_string()
        } else {
            format!("{:.0}°C", temp).green().to_string()
        };
        parts.push(temp_str);
    }

    // Fan if available
    if let Some(rpm) = status.fan_rpm {
        parts.push(format!("{}rpm", rpm));
    }

    // Power source
    parts.push(format!(
        "{} {}",
        status.power_source.emoji(),
        if let Some(pct) = status.battery_percent {
            format!("{}%", pct)
        } else {
            status.power_source.label().to_string()
        }
    ));

    parts.join(" | ")
}

pub fn display_thermal_status(status: &ThermalStatus) {
    println!();
    println!("{}", "THERMAL STATUS".cyan().bold());
    println!("{}", "==============".dimmed());
    println!();

    println!(
        "  {} {}",
        "Thermal Level:".bold(),
        status.level.colored_label()
    );

    if let Some(temp) = status.cpu_temp {
        let temp_display = if temp > 90.0 {
            format!("{:.1}°C", temp).red().bold()
        } else if temp > 75.0 {
            format!("{:.1}°C", temp).yellow()
        } else {
            format!("{:.1}°C", temp).green()
        };
        println!("  {} {}", "CPU Temperature:".bold(), temp_display);
    }

    if let Some(temp) = status.gpu_temp {
        let temp_display = if temp > 90.0 {
            format!("{:.1}°C", temp).red().bold()
        } else if temp > 75.0 {
            format!("{:.1}°C", temp).yellow()
        } else {
            format!("{:.1}°C", temp).green()
        };
        println!("  {} {}", "GPU Temperature:".bold(), temp_display);
    }

    if let Some(rpm) = status.fan_rpm {
        println!("  {} {} RPM", "Fan Speed:".bold(), rpm);
    }

    if let Some(power) = status.cpu_power {
        println!("  {} {:.1}W", "CPU Power:".bold(), power);
    }

    println!();
    println!(
        "  {} {} {}",
        "Power Source:".bold(),
        status.power_source.emoji(),
        status.power_source.label()
    );

    if let Some(pct) = status.battery_percent {
        let pct_display = if pct < 20 {
            format!("{}%", pct).red().bold()
        } else if pct < 50 {
            format!("{}%", pct).yellow()
        } else {
            format!("{}%", pct).green()
        };
        println!("  {} {}", "Battery:".bold(), pct_display);
    }

    println!();

    if status.level.is_dangerous() {
        println!(
            "  {} System is running hot! Consider running: mac-cooldown clean",
            "WARNING:".red().bold()
        );
        println!();
    }
}

pub fn watch_thermal(interval_secs: u64) -> Result<()> {
    println!("{}", "Watching thermal status (Ctrl+C to stop)...".dimmed());
    println!();

    loop {
        // Clear screen
        print!("\x1B[2J\x1B[1;1H");

        match get_thermal_status() {
            Ok(status) => {
                display_thermal_status(&status);

                let now = chrono::Local::now();
                println!(
                    "  {} {}",
                    "Last update:".dimmed(),
                    now.format("%H:%M:%S").to_string().dimmed()
                );
                println!(
                    "  {} {} seconds",
                    "Refresh interval:".dimmed(),
                    interval_secs
                );
            }
            Err(e) => {
                println!("  {} {}", "Error:".red().bold(), e);
            }
        }

        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thermal_level_from_str() {
        assert_eq!(ThermalLevel::from_str("nominal"), ThermalLevel::Nominal);
        assert_eq!(ThermalLevel::from_str("MODERATE"), ThermalLevel::Moderate);
        assert_eq!(ThermalLevel::from_str("heavy"), ThermalLevel::Heavy);
        assert_eq!(ThermalLevel::from_str("critical"), ThermalLevel::Critical);
        assert_eq!(ThermalLevel::from_str("0"), ThermalLevel::Nominal);
        assert_eq!(ThermalLevel::from_str("1"), ThermalLevel::Moderate);
    }

    #[test]
    fn test_thermal_level_from_pressure() {
        assert_eq!(ThermalLevel::from_pressure(0), ThermalLevel::Nominal);
        assert_eq!(ThermalLevel::from_pressure(15), ThermalLevel::Moderate);
        assert_eq!(ThermalLevel::from_pressure(50), ThermalLevel::Heavy);
        assert_eq!(ThermalLevel::from_pressure(100), ThermalLevel::Critical);
    }

    #[test]
    fn test_thermal_level_is_dangerous() {
        assert!(!ThermalLevel::Nominal.is_dangerous());
        assert!(!ThermalLevel::Moderate.is_dangerous());
        assert!(ThermalLevel::Heavy.is_dangerous());
        assert!(ThermalLevel::Critical.is_dangerous());
    }

    #[test]
    fn test_parse_powermetrics_output() {
        let sample = r#"
Thermal level: nominal
CPU die temperature: 45.5 C
GPU die temperature: 42.3 C
Fan: 2500 rpm
CPU Power: 15.2 W
"#;
        let status = parse_powermetrics_output(sample).unwrap();
        assert_eq!(status.level, ThermalLevel::Nominal);
        assert_eq!(status.cpu_temp, Some(45.5));
        assert_eq!(status.gpu_temp, Some(42.3));
        assert_eq!(status.fan_rpm, Some(2500));
        assert_eq!(status.cpu_power, Some(15.2));
    }
}
