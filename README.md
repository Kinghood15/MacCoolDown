# cooldown

CLI tool to manage CPU-heavy processes and keep your MacBook cool.

## Features

- Scan processes above a CPU threshold with issue detection (orphan, stuck, old)
- Clean problematic processes with whitelist support
- Target specific process types: orphans, stuck, or old processes
- Watch mode with continuous monitoring and optional auto-clean
- JSON output for scripting
- Persistent whitelist stored in `~/.config/cooldown/whitelist.json`

## Installation

```bash
cd /Volumes/Mac/cooldown
cargo build --release
# Copy binary to PATH (optional)
cp target/release/cooldown /usr/local/bin/cooldown
```

## Usage

### Scan

Display all processes using >= 50% CPU:

```
cooldown scan
cooldown scan --threshold 20
cooldown scan --json
```

### Status

System overview (load, CPU, memory, top processes):

```
cooldown status
```

### Clean

Kill all detected orphan/stuck/old processes (respects whitelist):

```
cooldown clean --dry-run     # preview only
cooldown clean               # SIGTERM
cooldown clean --force       # SIGKILL
```

### Kill

Target specific process categories:

```
cooldown kill --orphans --dry-run
cooldown kill --stuck
cooldown kill --old 3d           # processes running > 3 days
cooldown kill --old 12h          # processes running > 12 hours
cooldown kill --pid 38902        # kill a specific PID
cooldown kill --orphans --force  # SIGKILL
```

### Watch

Continuous monitoring with configurable interval:

```
cooldown watch                            # 80% threshold, 30s interval
cooldown watch --threshold 60 --interval 10
cooldown watch --auto-clean               # auto-kill on detection
```

### Whitelist

Protect processes from being killed (substring match, `*` wildcard supported):

```
cooldown whitelist add "claude --dangerously"
cooldown whitelist add "node*expo"
cooldown whitelist add "Xcode"
cooldown whitelist list
cooldown whitelist remove "Xcode"
cooldown whitelist clear
```

## Process Issue Types

| Status | Meaning |
|--------|---------|
| `[ORPHAN]` | Parent PID = 1 (init), high CPU, running > 1h |
| `[STUCK]` | CPU > 200% (2+ full cores), running > 1h |
| `[OLD Nd]` | Running >= 3 days with CPU > 30% |
| `[OK]` | Normal |

## Sample Output

```
HIGH CPU PROCESSES (>= 50%)
+--------+--------------------+------+--------+---------+-----------+
| PID    | Process            | CPU% | MEM MB | Running | Status    |
+--------+--------------------+------+--------+---------+-----------+
| 38902  | esbuild            | 365% | 48     | 4d 2h   | [STUCK]   |
| 76224  | claude             | 93%  | 2      | 5d 10h  | [ORPHAN]  |
| 94757  | claude             | 21%  | 344    | 10h     | [OK] [WL] |
+--------+--------------------+------+--------+---------+-----------+

  System: Load 10.81 | CPU 83% | Memory 15GB/16GB

  Hint: Found 2 problematic processes (458% CPU wasted)
     Run `cooldown clean` to fix
```

## Configuration

Whitelist file: `~/.config/cooldown/whitelist.json` (macOS: `~/Library/Application Support/cooldown/whitelist.json`)

```json
{
  "patterns": [
    "claude --dangerously",
    "node*expo",
    "Xcode",
    "cargo build"
  ]
}
```

## Notes

- CPU percentages are per-core (sysinfo convention). 200% = 2 full cores busy.
- Process scanning refreshes twice with a short sleep to get accurate CPU readings (first reading from sysinfo is always 0).
- `clean` and `kill` use SIGTERM by default; add `--force` for SIGKILL.
- Whitelisted processes are shown with `[WL]` tag in scan output and are always skipped in clean/kill.
