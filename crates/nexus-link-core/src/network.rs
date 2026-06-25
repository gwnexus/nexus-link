/// Detect the primary private IP address of this machine.
///
/// Strategy 1: `ip route get 1.1.1.1` — finds the source IP used for outbound
/// traffic, which is the correct primary interface IP on Linux.
/// Strategy 2: `hostname -I` — first IP from the list (fallback).
///
/// Returns `None` if neither command succeeds or produces output.
pub fn detect_private_ip() -> Option<String> {
    // Strategy 1: ip route get 1.1.1.1
    // Output: "1.1.1.1 via 10.0.0.1 dev eth0 src 10.0.10.121 uid 1000"
    let output = std::process::Command::new("ip")
        .args(["route", "get", "1.1.1.1"])
        .output();

    if let Ok(o) = output
        && o.status.success()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        if let Some(src_idx) = stdout.find("src ") {
            let after_src = &stdout[src_idx + 4..];
            let ip = after_src.split_whitespace().next().unwrap_or("");
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }

    // Strategy 2: hostname -I (first IP)
    let output = std::process::Command::new("hostname").arg("-I").output();

    if let Ok(o) = output
        && o.status.success()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let first_ip = stdout.split_whitespace().next().unwrap_or("");
        if !first_ip.is_empty() {
            return Some(first_ip.to_string());
        }
    }

    None
}

/// Format a private IP as a nexus-link service endpoint.
/// Appends the service port: `<ip>:<port>` (default 8443).
pub fn format_service_endpoint(ip: &str, port: u16) -> String {
    format!("{}:{}", ip, port)
}
