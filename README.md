# cooldown

CLI tool to manage CPU-heavy processes and keep your MacBook cool.

## Features

- **Realtime dashboard** - Live monitoring with auto-refresh (default mode)
- **Interactive menu** - Select and kill processes with confirmation
- **Smart detection** - Identifies orphan, stuck, and old processes
- **System app protection** - Auto-whitelist Safari, Finder, etc.
- **Thermal monitoring** - CPU temperature, fan speed, thermal level
- **Battery aware** - Lower thresholds when on battery power
- **CPU throttling** - Limit any process CPU usage via SIGSTOP/SIGCONT
- **Wrap command** - Run commands with thermal safety and CPU limits
- **Maintenance tasks** - DNS flush, memory purge, Time Machine cleanup

## Installation

```bash
cargo build --release
sudo cp target/release/cooldown /usr/local/bin/cooldown
```

## Usage

### Default: Realtime Dashboard

Just run `cooldown` to start the realtime dashboard (auto-refresh every 5s):

**Keyboard shortcuts:**
- `↑/↓` or `j/k` - Navigate processes
- `x` - Kill selected process
- `K` - Kill all problematic
- `m` or `Enter` - Open menu
- `p` or `Space` - Pause/Resume
- `q` or `Esc` - Quit

```
COOLDOWN REALTIME
=================

  System: Load 3.60 | CPU 34% | Mem 12GB/16GB (75%)
  Thermal: ❄️ Nominal | CPU 45°C | Fan 2500rpm | 🔌 AC Power

  ⚠ PROBLEMATIC 1 process(es) - 99% CPU, 150MB RAM

    • [40632] claude               99% CPU   150MB Orphan process

  TOP PROCESSES
       PID  NAME                    CPU%  MEM MB  RUNTIME   STATUS
    ──────────────────────────────────────────────────────────────────
    40632  claude                  99%     150    4d 5h    [ORPHAN]
    73333  Chrome Helper          125%     800      2h    [OK]
    96564  qemu-system-aarch64     72%    2048      5h    [OK]

  Last update: 10:45:32 | Press Ctrl+C to stop
```

Press `Ctrl+C` to stop and enter interactive menu.

### Interactive Menu

```bash
cooldown menu
```

Select actions with arrow keys:
- Kill all problematic processes
- Select specific processes to kill
- Quick throttle (limit CPU to 50%)
- Run maintenance tasks

### Other Commands

```bash
cooldown status          # Quick system overview
cooldown scan            # Scan high CPU processes
cooldown scan -t 30      # Custom threshold (30%)
cooldown clean           # Kill problematic processes
cooldown clean --dry-run # Preview what would be killed
```

### Kill Specific Processes

```bash
cooldown kill --orphans       # Kill orphan processes
cooldown kill --stuck         # Kill stuck processes
cooldown kill --old 3d        # Kill processes running > 3 days
cooldown kill --pid 12345     # Kill specific PID
```

### Throttle

Limit a process CPU usage:

```bash
cooldown throttle 12345 --cpu 50        # Limit to 50% CPU
cooldown throttle 12345 --cpu 30 -d 60  # Limit for 60 seconds
```

### Wrap Command

Run commands with thermal safety:

```bash
cooldown wrap cargo build                    # Monitor thermal
cooldown wrap --cpu 50 npm run build         # Limit to 50% CPU
cooldown wrap --thermal heavy cargo test     # Kill if too hot
cooldown wrap --timeout 300 make all         # Timeout after 5min
```

### Maintenance

```bash
cooldown maintenance --dns         # Flush DNS cache
cooldown maintenance --purgeable   # Free purgeable space
cooldown maintenance --timemachine # Clear Time Machine snapshots
cooldown maintenance --all         # Run all tasks
```

### Whitelist

Protect processes from being killed:

```bash
cooldown whitelist add "node*expo"
cooldown whitelist add "cargo build"
cooldown whitelist list
cooldown whitelist remove "cargo build"
```

### Configuration

```bash
cooldown config --init              # Create config file
cooldown config --show              # Show current config
cooldown config --set cpu_threshold=75
```

## Process Status Types

| Status | Meaning |
|--------|---------|
| `[ORPHAN]` | Parent PID = 1, high CPU, running > 1h |
| `[STUCK]` | CPU > 200%, running > 1h |
| `[OLD]` | Running >= 3 days with high CPU |
| `[SYS]` | System app (protected) |
| `[WL]` | Whitelisted |
| `[OK]` | Normal |

## Battery Aware Mode

When running on battery:
- Lower CPU threshold (30% vs 50%)
- More aggressive about identifying problematic processes
- Shown in dashboard: `🔋 Battery 85%`

## Thermal Levels

| Level | Meaning |
|-------|---------|
| ❄️ Nominal | Cool, normal operation |
| 🌡️ Moderate | Slightly warm |
| 🔥 Heavy | Hot, consider action |
| 🚨 Critical | Very hot, immediate action needed |

## Configuration File

Location: `~/Library/Application Support/cooldown/config.toml`

```toml
cpu_threshold = 50
watch_interval = 30
auto_clean = false
thermal_limit = "critical"
wrap_cpu_limit = 80
whitelist = ["Xcode", "cargo build"]
```

## Notes

- CPU percentages are per-core (200% = 2 full cores)
- System apps (Safari, Finder, etc.) are automatically protected
- Throttling uses SIGSTOP/SIGCONT cycling
- Some maintenance tasks require `sudo`
