//! Process discovery and termination helpers for the PTY driver and probe clients.

/// Scans the PATH for executable binaries to check if the CLI is installed.
pub(crate) fn installed(cmd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        std::fs::metadata(dir.join(cmd))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

/// Gets the list of PIDs with an exact process name match via `pgrep -x`.
pub(super) fn pids_of(name: &str) -> Vec<i32> {
    let Ok(o) = std::process::Command::new("pgrep")
        .args(["-x", name])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<i32>().ok())
        .collect()
}

/// Queries the parent PID using `ps`.
pub(super) fn parent_pid(pid: i32) -> Option<i32> {
    let o = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&o.stdout)
        .trim()
        .parse::<i32>()
        .ok()
}

/// Recursively collects child processes of a PID via `pgrep -P` (must be called before the parent process dies).
pub(super) fn collect_descendants(root: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let mut queue = vec![root];
    while let Some(pid) = queue.pop() {
        let Ok(o) = std::process::Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
        else {
            continue;
        };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Ok(child) = line.trim().parse::<i32>() {
                out.push(child);
                queue.push(child);
            }
        }
    }
    out
}
