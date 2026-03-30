// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// util.rs — Shared command execution and binary detection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use std::process::Command;

/// Run an external command and return (success, stdout, stderr).
pub fn run_cmd(program: &str, args: &[&str]) -> (bool, String, String) {
    log::debug!("exec: {} {}", program, args.join(" "));
    match Command::new(program).args(args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            (output.status.success(), stdout, stderr)
        }
        Err(e) => (false, String::new(), e.to_string()),
    }
}

/// Check if a binary exists in PATH.
pub fn binary_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
