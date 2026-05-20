// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// privilege.rs — Sudo wrapper for commands requiring root
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::util::run_cmd;
use std::io::Write;
use std::process::{Command, Stdio};

/// Run a command with sudo. If already root, sudo is a passthrough.
pub fn run_privileged(program: &str, args: &[&str]) -> (bool, String, String) {
    let mut sudo_args = vec![program];
    sudo_args.extend_from_slice(args);
    run_cmd("sudo", &sudo_args)
}

/// Run a command with sudo, piping `stdin` to its standard input.
///
/// Returns `(success, stdout, stderr)` like `run_privileged`. Used when we
/// need to feed a ruleset (or similar) to a privileged tool without writing
/// it to a predictable temp path first — closes the TOCTOU window in
/// `killswitch::enable`.
pub fn run_privileged_stdin(
    program: &str,
    args: &[&str],
    stdin_data: &str,
) -> (bool, String, String) {
    log::debug!("exec (with stdin): sudo {} {}", program, args.join(" "));

    let mut child = match Command::new("sudo")
        .arg(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return (false, String::new(), e.to_string()),
    };

    if let Some(mut sin) = child.stdin.take() {
        if let Err(e) = sin.write_all(stdin_data.as_bytes()) {
            return (false, String::new(), format!("stdin write failed: {e}"));
        }
        // Drop sin to close the pipe so the child sees EOF.
    }

    match child.wait_with_output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            (output.status.success(), stdout, stderr)
        }
        Err(e) => (false, String::new(), e.to_string()),
    }
}
