// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// privilege.rs — Sudo wrapper for commands requiring root
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::util::run_cmd;

/// Run a command with sudo. If already root, sudo is a passthrough.
pub fn run_privileged(program: &str, args: &[&str]) -> (bool, String, String) {
    let mut sudo_args = vec![program];
    sudo_args.extend_from_slice(args);
    run_cmd("sudo", &sudo_args)
}
