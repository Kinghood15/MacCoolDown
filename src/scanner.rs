use sysinfo::{ProcessStatus, System};

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cmd: String,
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub parent_pid: Option<u32>,
    pub start_time: u64,
    #[allow(dead_code)]
    pub status: ProcessStatus,
}

pub fn scan_processes(threshold: f32) -> Vec<ProcessInfo> {
    let mut sys = System::new_all();
    // Refresh twice to get accurate CPU readings (first reading is always 0)
    sys.refresh_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_all();

    let mut processes: Vec<ProcessInfo> = sys
        .processes()
        .iter()
        .filter(|(_, proc)| proc.cpu_usage() >= threshold)
        .map(|(pid, proc)| {
            let cmd = proc.cmd().join(" ");
            ProcessInfo {
                pid: pid.as_u32(),
                name: proc.name().to_string(),
                cmd: if cmd.is_empty() {
                    proc.name().to_string()
                } else {
                    cmd
                },
                cpu_percent: proc.cpu_usage(),
                memory_mb: proc.memory() / 1024 / 1024,
                parent_pid: proc.parent().map(|p| p.as_u32()),
                start_time: proc.start_time(),
                status: proc.status(),
            }
        })
        .collect();

    processes.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap());
    processes
}

pub fn scan_all_processes() -> Vec<ProcessInfo> {
    scan_processes(0.0)
}

pub fn get_system_load() -> (f64, f64, f64) {
    // Returns (1min, 5min, 15min) load averages
    let load = System::load_average();
    (load.one, load.five, load.fifteen)
}

pub fn get_memory_info() -> (u64, u64) {
    let mut sys = System::new();
    sys.refresh_memory();
    let used_gb = sys.used_memory() / 1024 / 1024 / 1024;
    let total_gb = sys.total_memory() / 1024 / 1024 / 1024;
    (used_gb, total_gb)
}

pub fn get_cpu_usage() -> f32 {
    let mut sys = System::new_all();
    sys.refresh_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_all();

    let cpus = sys.cpus();
    if cpus.is_empty() {
        return 0.0;
    }
    cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
}
