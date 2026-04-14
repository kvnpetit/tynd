// Auto-generated module split — edit freely.

use std::collections::HashMap;
use std::time::Duration;
use serde_json::{json, Value};
use sysinfo::{Networks, NetworkData};

use crate::ipc;
use super::round1;

pub fn get_network_info(id: String) {
    std::thread::spawn(move || {
        let networks = Networks::new_with_refreshed_list();

        // Build name → IP list map using if-addrs (cross-platform, no subprocess)
        let ip_map: HashMap<String, Vec<String>> = {
            let mut map: HashMap<String, Vec<String>> = HashMap::new();
            if let Ok(ifaces) = if_addrs::get_if_addrs() {
                for iface in ifaces {
                    if iface.is_loopback() { continue; }
                    let ip = iface.ip().to_string();
                    map.entry(iface.name).or_default().push(ip);
                }
            }
            map
        };

        let list: Vec<Value> = networks
            .iter()
            .map(|(name, data): (&String, &NetworkData)| {
                let mac = data.mac_address().to_string();
                let ips = ip_map.get(name).cloned().unwrap_or_default();
                json!({
                    "name": name,
                    "mac": mac,
                    "ips": ips,
                    "received": data.total_received(),
                    "transmitted": data.total_transmitted(),
                    "up": interface_is_up(name),
                })
            })
            .collect();

        ipc::emit_response(&id, json!(list));
    });
}

pub fn get_network_speed(id: String) {
    std::thread::spawn(move || {
        let snap1: Vec<(String, u64, u64)> = {
            let nets = Networks::new_with_refreshed_list();
            nets.iter()
                .map(|(name, data)| (name.clone(), data.total_received(), data.total_transmitted()))
                .collect()
        };

        std::thread::sleep(Duration::from_secs(1));

        let snap2_map: HashMap<String, (u64, u64)> = {
            let nets = Networks::new_with_refreshed_list();
            nets.iter()
                .map(|(name, data)| (name.clone(), (data.total_received(), data.total_transmitted())))
                .collect()
        };

        let result: Vec<Value> = snap1.iter().filter_map(|(name, rx1, tx1)| {
            let (rx2, tx2) = snap2_map.get(name).copied()?;
            let rx_bps = rx2.saturating_sub(*rx1);
            let tx_bps = tx2.saturating_sub(*tx1);
            Some(json!({
                "name": name,
                "rxBytesPerSec": rx_bps,
                "txBytesPerSec": tx_bps,
                "rxMbps": round1(rx_bps as f64 * 8.0 / 1_000_000.0),
                "txMbps": round1(tx_bps as f64 * 8.0 / 1_000_000.0),
            }))
        }).collect();

        ipc::emit_response(&id, json!(result));
    });
}

fn interface_is_up(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
            .map(|s| s.trim() == "up")
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Exclude known virtual/loopback names; treat everything else as up.
        let lower = name.to_lowercase();
        !lower.starts_with("lo") && lower != "loopback"
    }
}
