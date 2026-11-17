//! systemd unit rendering, shared by `nexus-link setup` and
//! `nexus-link [--profile P] agent start`, so every profile gets the same
//! units.
//!
//! Layout (dedicated `nexus-link` user):
//! - `/etc/nexus-link` (`0750 root:nexus-link`): configs and signing key,
//!   read-only for the services (`ProtectSystem=strict`).
//! - `/var/lib/nexus-link[/<profile>]` (`0700 nexus-link`): runtime state,
//!   created by systemd (`StateDirectory=`) and used as `WorkingDirectory`.
//! - The compose dir is writable for the service only (`put_file`).

use std::path::Path;

use crate::config::{DEFAULT_PROFILE, SERVICE_GROUP, SERVICE_USER};

/// Which account the units run as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitUser {
    /// The dedicated `nexus-link` system user (created by `nexus-link setup`).
    Dedicated,
    /// A regular login user (no setup); its home holds `~/.nexus-link`.
    Login { user: String, home: String },
    /// Root, no `User=` line.
    Root,
}

/// Whether the units are system units (hardened) or `systemctl --user` units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitScope {
    System,
    User,
}

/// Inputs for both units.
#[derive(Debug, Clone)]
pub struct UnitParams<'a> {
    pub profile: &'a str,
    pub agent_bin: &'a Path,
    pub service_bin: &'a Path,
    pub config_path: &'a Path,
    pub compose_dir: &'a Path,
    pub user: UnitUser,
    pub scope: UnitScope,
}

/// `StateDirectory=` value, relative to `/var/lib`.
pub fn state_directory(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        "nexus-link".to_string()
    } else {
        format!("nexus-link/{profile}")
    }
}

/// Agent unit (telemetry push). Needs no write access outside its state dir.
pub fn render_agent_unit(p: &UnitParams<'_>) -> String {
    render(
        p,
        "Nexus Link Telemetry Agent",
        p.agent_bin,
        "nexus_link_agent=info",
        &[],
    )
}

/// Service unit (command channel). Writes the compose dir (`put_file`).
pub fn render_service_unit(p: &UnitParams<'_>) -> String {
    render(
        p,
        "Nexus Link Command Service",
        p.service_bin,
        "nexus_link_service=info",
        &[p.compose_dir],
    )
}

fn render(
    p: &UnitParams<'_>,
    description: &str,
    bin: &Path,
    rust_log: &str,
    writable: &[&Path],
) -> String {
    let suffix = if p.profile == DEFAULT_PROFILE {
        String::new()
    } else {
        format!(" ({})", p.profile)
    };

    let mut service = vec![
        "Type=simple".to_string(),
        format!("ExecStart={}", bin.display()),
        "Restart=always".to_string(),
        "RestartSec=5".to_string(),
        format!("Environment=RUST_LOG={rust_log}"),
        format!("Environment=NEXUS_LINK_PROFILE={}", p.profile),
        format!("Environment=NEXUS_LINK_CONFIG={}", p.config_path.display()),
    ];

    let install = match p.scope {
        UnitScope::User => "WantedBy=default.target",
        UnitScope::System => {
            match &p.user {
                UnitUser::Dedicated => {
                    service.push(format!("User={SERVICE_USER}"));
                    service.push(format!("Group={SERVICE_GROUP}"));
                    service.push("SupplementaryGroups=docker".to_string());
                }
                UnitUser::Login { user, home } => {
                    service.push(format!("User={user}"));
                    service.push(format!("Group={user}"));
                    service.push(format!("Environment=HOME={home}"));
                }
                UnitUser::Root => {}
            }
            let state = state_directory(p.profile);
            service.push(format!("StateDirectory={state}"));
            service.push("StateDirectoryMode=0700".to_string());
            service.push(format!("WorkingDirectory=/var/lib/{state}"));
            service.push(String::new());
            service.push("# Hardening: /etc/nexus-link and the rest of the OS are".to_string());
            service.push("# read-only; only the state dir (and for the service the".to_string());
            service.push("# compose dir) is writable.".to_string());
            service.push("NoNewPrivileges=true".to_string());
            service.push("ProtectSystem=strict".to_string());
            // A login user's config lives in its home; keep it readable.
            service.push(match p.user {
                UnitUser::Login { .. } => "ProtectHome=read-only".to_string(),
                _ => "ProtectHome=true".to_string(),
            });
            service.push("PrivateTmp=true".to_string());
            service.push("ProtectKernelTunables=true".to_string());
            service.push("ProtectKernelModules=true".to_string());
            service.push("ProtectControlGroups=true".to_string());
            service.push("RestrictSUIDSGID=true".to_string());
            if !writable.is_empty() {
                let paths: Vec<String> = writable.iter().map(|w| w.display().to_string()).collect();
                service.push(format!("ReadWritePaths={}", paths.join(" ")));
            }
            "WantedBy=multi-user.target"
        }
    };

    format!(
        "[Unit]
Description={description}{suffix}
Documentation=https://github.com/gwnexus/nexus-link
After=network-online.target docker.service
Wants=network-online.target docker.service

[Service]
{service}

[Install]
{install}
",
        service = service.join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn render_both(profile: &str, user: UnitUser, scope: UnitScope) -> (String, String) {
        let config = if profile == DEFAULT_PROFILE {
            PathBuf::from("/etc/nexus-link/config.toml")
        } else {
            PathBuf::from(format!("/etc/nexus-link/config.{profile}.toml"))
        };
        let p = UnitParams {
            profile,
            agent_bin: Path::new("/usr/local/bin/nexus-link-agent"),
            service_bin: Path::new("/usr/local/bin/nexus-link-service"),
            config_path: &config,
            compose_dir: Path::new("/opt/dgx-llm"),
            user,
            scope,
        };
        (render_agent_unit(&p), render_service_unit(&p))
    }

    fn lines(unit: &str) -> Vec<&str> {
        unit.lines().collect()
    }

    #[test]
    fn profile_units_use_state_dir_not_etc() {
        let (agent, service) = render_both("staging", UnitUser::Dedicated, UnitScope::System);
        for unit in [&agent, &service] {
            let l = lines(unit);
            assert!(l.contains(&"StateDirectory=nexus-link/staging"));
            assert!(l.contains(&"WorkingDirectory=/var/lib/nexus-link/staging"));
            assert!(l.contains(&"ProtectSystem=strict"));
            assert!(l.contains(&"Environment=NEXUS_LINK_PROFILE=staging"));
            assert!(
                l.contains(&"Environment=NEXUS_LINK_CONFIG=/etc/nexus-link/config.staging.toml")
            );
            assert!(l.contains(&"User=nexus-link"));
            // The config dir must never be writable for the services
            assert!(!unit.contains("ReadWritePaths=/etc"));
            assert!(!unit.contains("WorkingDirectory=/etc"));
        }
        assert!(service.contains("Description=Nexus Link Command Service (staging)"));
    }

    #[test]
    fn only_the_service_may_write_the_compose_dir() {
        let (agent, service) = render_both("default", UnitUser::Dedicated, UnitScope::System);
        assert!(lines(&service).contains(&"ReadWritePaths=/opt/dgx-llm"));
        assert!(!agent.contains("ReadWritePaths"));
    }

    #[test]
    fn default_profile_keeps_legacy_paths() {
        let (agent, _) = render_both("default", UnitUser::Dedicated, UnitScope::System);
        let l = lines(&agent);
        assert!(l.contains(&"StateDirectory=nexus-link"));
        assert!(l.contains(&"WorkingDirectory=/var/lib/nexus-link"));
        assert!(l.contains(&"Environment=NEXUS_LINK_CONFIG=/etc/nexus-link/config.toml"));
        assert!(l.contains(&"Description=Nexus Link Telemetry Agent"));
    }

    #[test]
    fn login_user_keeps_home_readable() {
        let user = UnitUser::Login {
            user: "op".into(),
            home: "/home/op".into(),
        };
        let (agent, _) = render_both("default", user, UnitScope::System);
        let l = lines(&agent);
        assert!(l.contains(&"User=op"));
        assert!(l.contains(&"Environment=HOME=/home/op"));
        assert!(l.contains(&"ProtectHome=read-only"));
        assert!(!l.contains(&"SupplementaryGroups=docker"));
    }

    #[test]
    fn user_scope_has_no_system_directives() {
        let (agent, service) = render_both("default", UnitUser::Root, UnitScope::User);
        for unit in [&agent, &service] {
            assert!(unit.contains("WantedBy=default.target"));
            assert!(!unit.contains("ProtectSystem"));
            assert!(!unit.contains("StateDirectory"));
            assert!(!unit.contains("User="));
        }
    }

    /// Writes the units for `systemd-analyze verify`:
    /// `NEXUS_LINK_UNIT_DIR=<dir> cargo test -p nexus-link-core -- --ignored write_units`
    #[test]
    #[ignore = "writes files for manual verification"]
    fn write_units_for_verification() {
        let dir = std::env::var("NEXUS_LINK_UNIT_DIR").expect("set NEXUS_LINK_UNIT_DIR");
        for (profile, prefix) in [("default", ""), ("staging", "@staging")] {
            let (agent, service) = render_both(profile, UnitUser::Dedicated, UnitScope::System);
            std::fs::write(format!("{dir}/nexus-link-agent{prefix}.service"), agent).unwrap();
            std::fs::write(format!("{dir}/nexus-link-service{prefix}.service"), service).unwrap();
        }
    }
}
