use std::time::Duration;

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

use crate::analyzer::analyze_processes;
use crate::display::{
    display_analyzed, display_system_status, display_watch_header, display_watch_tick,
};
use crate::killer::{kill_process, KillResult};
use crate::scanner::{get_cpu_usage, get_memory_info, get_system_load, scan_processes};
use crate::whitelist::Whitelist;

pub struct WatchConfig {
    pub threshold: u32,
    pub interval_secs: u64,
    pub auto_clean: bool,
}

pub fn run_watch(config: WatchConfig) -> anyhow::Result<()> {
    let whitelist = Whitelist::load()?;

    display_watch_header(config.threshold, config.interval_secs);

    let mut tick: u64 = 0;

    loop {
        tick += 1;
        display_watch_tick(tick);

        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        spinner.set_message("Scanning processes...");
        spinner.enable_steady_tick(Duration::from_millis(80));

        let processes = scan_processes(config.threshold as f32);
        let analyzed = analyze_processes(&processes, &whitelist);

        spinner.finish_and_clear();

        // System stats
        let load = get_system_load();
        let mem = get_memory_info();
        let cpu = get_cpu_usage();
        display_system_status(load, mem, cpu);

        display_analyzed(&analyzed, config.threshold);

        // Problematic summary
        let problematic: Vec<_> = analyzed
            .iter()
            .filter(|p| p.issue.is_problematic() && !p.whitelisted)
            .collect();

        if problematic.is_empty() {
            println!("  {}", "All clear - no problematic processes.".green());
        } else {
            let wasted: f32 = problematic.iter().map(|p| p.info.cpu_percent).sum();
            println!(
                "  {} {} problematic processes ({:.0}% CPU wasted)",
                "WARNING:".yellow().bold(),
                problematic.len(),
                wasted
            );

            if config.auto_clean {
                println!("  {} Auto-cleaning...", "ACTION:".red().bold());
                let killed_cpu: f32 = problematic.iter().map(|p| p.info.cpu_percent).sum();
                let results: Vec<KillResult> = problematic
                    .iter()
                    .map(|p| kill_process(p.info.pid, &p.info.name, false))
                    .collect();

                let success_count = results.iter().filter(|r| r.success).count();
                for r in &results {
                    if r.success {
                        println!(
                            "    {} Killed PID {} ({})",
                            "OK".green(),
                            r.pid,
                            r.name.cyan()
                        );
                    } else {
                        println!(
                            "    {} Failed PID {}: {}",
                            "ERR".red(),
                            r.pid,
                            r.error.as_deref().unwrap_or("?").red()
                        );
                    }
                }
                if success_count > 0 {
                    println!(
                        "  {} Freed ~{:.0}% CPU",
                        "Done!".green().bold(),
                        killed_cpu
                    );
                }
            } else {
                println!(
                    "  Run {} to clean",
                    "`cooldown clean`".cyan()
                );
            }
        }

        // Countdown to next scan
        let pb = ProgressBar::new(config.interval_secs);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "  {msg} [{bar:30.cyan/blue}] {pos}/{len}s",
                )
                .unwrap()
                .progress_chars("=>-"),
        );
        pb.set_message("Next scan in");
        for _ in 0..config.interval_secs {
            std::thread::sleep(Duration::from_secs(1));
            pb.inc(1);
        }
        pb.finish_and_clear();
    }
}
