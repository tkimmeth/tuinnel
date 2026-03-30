// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// bandwidth.rs — Live bandwidth monitoring from sysfs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Reads TX/RX bytes from the kernel's sysfs interface:
//   /sys/class/net/<iface>/statistics/rx_bytes
//   /sys/class/net/<iface>/statistics/tx_bytes
//
// Computes throughput by diffing values over time intervals.
// Maintains a rolling buffer for sparkline rendering.

use std::fs;
use std::time::Instant;

const HISTORY_LEN: usize = 30;

pub struct BandwidthMonitor {
    interface: Option<String>,
    last_rx: u64,
    last_tx: u64,
    last_sample: Instant,
    /// Rolling buffer of RX bytes/sec values.
    pub rx_history: Vec<u64>,
    /// Rolling buffer of TX bytes/sec values.
    pub tx_history: Vec<u64>,
    /// Current RX rate in bytes/sec.
    pub rx_rate: u64,
    /// Current TX rate in bytes/sec.
    pub tx_rate: u64,
}

impl BandwidthMonitor {
    pub fn new() -> Self {
        let interface: Option<String> = None;
        let (rx, tx) = interface
            .as_ref()
            .map(|iface| read_bytes(iface))
            .unwrap_or((0, 0));

        Self {
            interface,
            last_rx: rx,
            last_tx: tx,
            last_sample: Instant::now(),
            rx_history: vec![0; HISTORY_LEN],
            tx_history: vec![0; HISTORY_LEN],
            rx_rate: 0,
            tx_rate: 0,
        }
    }

    /// Take a new sample and update rates + history.
    pub fn sample(&mut self) {
        // Re-detect interface if we don't have one (might have connected since last check)
        if self.interface.is_none() {
            self.interface = detect_vpn_interface();
            if let Some(ref iface) = self.interface {
                let (rx, tx) = read_bytes(iface);
                self.last_rx = rx;
                self.last_tx = tx;
                self.last_sample = Instant::now();
                return;
            }
        }

        let Some(ref iface) = self.interface else {
            return;
        };

        let (rx, tx) = read_bytes(iface);
        let elapsed = self.last_sample.elapsed().as_secs_f64();

        if elapsed > 0.1 {
            // Compute rates (handle counter reset gracefully)
            self.rx_rate = if rx >= self.last_rx {
                ((rx - self.last_rx) as f64 / elapsed) as u64
            } else {
                0
            };
            self.tx_rate = if tx >= self.last_tx {
                ((tx - self.last_tx) as f64 / elapsed) as u64
            } else {
                0
            };

            // Push to rolling buffer
            self.rx_history.push(self.rx_rate);
            self.tx_history.push(self.tx_rate);

            if self.rx_history.len() > HISTORY_LEN {
                self.rx_history.remove(0);
            }
            if self.tx_history.len() > HISTORY_LEN {
                self.tx_history.remove(0);
            }

            self.last_rx = rx;
            self.last_tx = tx;
            self.last_sample = Instant::now();
        }
    }

    /// Set interface from SessionInfo. Call when session changes.
    pub fn set_interface(&mut self, name: Option<String>) {
        if name != self.interface {
            self.interface = name;
            if let Some(ref iface) = self.interface {
                let (rx, tx) = read_bytes(iface);
                self.last_rx = rx;
                self.last_tx = tx;
            }
            self.rx_rate = 0;
            self.tx_rate = 0;
            self.last_sample = Instant::now();
        }
    }

    /// Check if interface is still up; reset if it went away.
    pub fn refresh_interface(&mut self) {
        let new_iface = detect_vpn_interface();
        if new_iface != self.interface {
            self.interface = new_iface;
            if let Some(ref iface) = self.interface {
                let (rx, tx) = read_bytes(iface);
                self.last_rx = rx;
                self.last_tx = tx;
            }
            self.rx_rate = 0;
            self.tx_rate = 0;
            self.last_sample = Instant::now();
        }
    }

    /// Format bytes/sec as human-readable string.
    pub fn format_rate(bytes_per_sec: u64) -> String {
        if bytes_per_sec >= 1_000_000_000 {
            format!("{:.1} GB/s", bytes_per_sec as f64 / 1_000_000_000.0)
        } else if bytes_per_sec >= 1_000_000 {
            format!("{:.1} MB/s", bytes_per_sec as f64 / 1_000_000.0)
        } else if bytes_per_sec >= 1_000 {
            format!("{:.1} KB/s", bytes_per_sec as f64 / 1_000.0)
        } else {
            format!("{} B/s", bytes_per_sec)
        }
    }

    pub fn has_interface(&self) -> bool {
        self.interface.is_some()
    }
}

/// Read RX and TX byte counters from sysfs.
fn read_bytes(iface: &str) -> (u64, u64) {
    let rx = read_sysfs_counter(iface, "rx_bytes");
    let tx = read_sysfs_counter(iface, "tx_bytes");
    (rx, tx)
}

fn read_sysfs_counter(iface: &str, counter: &str) -> u64 {
    let path = format!("/sys/class/net/{iface}/statistics/{counter}");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Detect any active VPN tunnel interface from sysfs.
fn detect_vpn_interface() -> Option<String> {
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if lower.starts_with("wg") || lower.starts_with("tun") {
                return Some(name);
            }
        }
    }
    None
}
