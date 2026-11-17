use nexus_link_core::config::{
    self, DEFAULT_PROFILE, SERVICE_GROUP, SERVICE_USER, SYSTEM_CONFIG_DIR, SYSTEM_STATE_DIR,
};
use nexus_link_core::setup::{SetupReport, SetupStep, StepStatus};
use std::path::PathBuf;

#[test]
fn test_setup_constants_are_consistent() {
    assert_eq!(SERVICE_USER, "nexus-link");
    assert_eq!(SERVICE_GROUP, "nexus-link");
    assert_eq!(SYSTEM_CONFIG_DIR, "/etc/nexus-link");
    assert_eq!(SYSTEM_STATE_DIR, "/var/lib/nexus-link");
}

#[test]
fn test_config_and_state_dirs_are_different() {
    // Config (FHS /etc) and state (/var/lib) must never be the same
    assert_ne!(SYSTEM_CONFIG_DIR, SYSTEM_STATE_DIR);
}

#[test]
fn test_install_dir_not_in_user_paths() {
    // /usr/local/bin must not overlap with user binary dirs
    let install_dir = PathBuf::from("/usr/local/bin");
    let user_local = PathBuf::from("/home/test/.local/bin");
    let user_cargo = PathBuf::from("/home/test/.cargo/bin");
    assert_ne!(install_dir, user_local);
    assert_ne!(install_dir, user_cargo);
}

#[test]
fn test_setup_report_success_when_no_failures() {
    let report = SetupReport {
        steps: vec![
            SetupStep {
                name: "test step",
                status: StepStatus::Created,
                message: "ok".to_string(),
            },
            SetupStep {
                name: "skipped step",
                status: StepStatus::Skipped,
                message: "already done".to_string(),
            },
        ],
        success: true,
    };
    assert!(report.success);
    assert!(report.steps.iter().all(|s| s.status != StepStatus::Failed));
}

#[test]
fn test_setup_report_failure_when_step_fails() {
    let report = SetupReport {
        steps: vec![
            SetupStep {
                name: "good step",
                status: StepStatus::Created,
                message: "ok".to_string(),
            },
            SetupStep {
                name: "bad step",
                status: StepStatus::Failed,
                message: "error".to_string(),
            },
        ],
        success: false,
    };
    assert!(!report.success);
}

#[test]
fn test_step_status_equality() {
    assert_eq!(StepStatus::Created, StepStatus::Created);
    assert_eq!(StepStatus::Skipped, StepStatus::Skipped);
    assert_eq!(StepStatus::Failed, StepStatus::Failed);
    assert_ne!(StepStatus::Created, StepStatus::Failed);
    assert_ne!(StepStatus::Created, StepStatus::Skipped);
    assert_ne!(StepStatus::Skipped, StepStatus::Failed);
}

#[test]
fn test_config_path_resolution_env_override() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    // NEXUS_LINK_CONFIG env var should take priority
    unsafe { std::env::set_var("NEXUS_LINK_CONFIG", "/tmp/test-nexus-link.toml") };
    let path = nexus_link_core::config::effective_config_path();
    assert_eq!(path, PathBuf::from("/tmp/test-nexus-link.toml"));
    unsafe { std::env::remove_var("NEXUS_LINK_CONFIG") };
}

#[test]
fn test_system_config_path_is_etc() {
    let path = nexus_link_core::config::system_config_path();
    assert_eq!(path, PathBuf::from("/etc/nexus-link/config.toml"));
}

#[test]
fn test_config_path_env_override_ignores_existing_system_paths() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    // When NEXUS_LINK_CONFIG is set, the result must be the env var value
    // regardless of whether /etc/ or /var/lib/ paths exist on disk.
    unsafe { std::env::set_var("NEXUS_LINK_CONFIG", "/opt/custom/nexus-link.toml") };
    let path = nexus_link_core::config::effective_config_path();
    assert_eq!(path, PathBuf::from("/opt/custom/nexus-link.toml"));
    // The env var path is returned even if the file does not exist --
    // it is the caller's responsibility to handle missing files.
    assert!(!path.exists());
    unsafe { std::env::remove_var("NEXUS_LINK_CONFIG") };
}

#[test]
fn test_config_path_env_override_is_not_empty_string() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    // Empty NEXUS_LINK_CONFIG should still be treated as set (pathological but valid).
    unsafe { std::env::set_var("NEXUS_LINK_CONFIG", "") };
    let path = nexus_link_core::config::effective_config_path();
    assert_eq!(path, PathBuf::from(""));
    unsafe { std::env::remove_var("NEXUS_LINK_CONFIG") };
}

#[test]
fn test_config_path_fallback_without_env_var() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    // Without env var, effective_config_path should return one of the
    // filesystem fallback paths (whichever exists, or the default).
    unsafe { std::env::remove_var("NEXUS_LINK_CONFIG") };
    let path = nexus_link_core::config::effective_config_path();
    // On dev machines none of the system paths exist, so it falls back
    // to ~/.nexus-link/config.toml
    let default = nexus_link_core::config::default_config_path();
    let system = nexus_link_core::config::system_config_path();
    let legacy = PathBuf::from(SYSTEM_STATE_DIR).join("config.toml");
    assert!(
        path == system || path == legacy || path == default,
        "effective_config_path returned unexpected path: {}",
        path.display()
    );
}

// ── Profile tests ───────────────────────────────────────────────────────────

#[test]
fn test_config_filename_default_profile() {
    assert_eq!(
        config::config_filename_for_profile("default"),
        "config.toml"
    );
}

#[test]
fn test_config_filename_named_profile() {
    assert_eq!(
        config::config_filename_for_profile("staging"),
        "config.staging.toml"
    );
    assert_eq!(
        config::config_filename_for_profile("production"),
        "config.production.toml"
    );
}

#[test]
fn test_state_dir_default_profile() {
    let dir = config::state_dir_for_profile("default");
    assert_eq!(dir, PathBuf::from(SYSTEM_STATE_DIR));
}

#[test]
fn test_state_dir_named_profile() {
    let dir = config::state_dir_for_profile("staging");
    assert_eq!(dir, PathBuf::from(SYSTEM_STATE_DIR).join("staging"));
}

#[test]
fn test_systemd_service_names_default() {
    assert_eq!(
        config::systemd_agent_service_name("default"),
        "nexus-link-agent"
    );
    assert_eq!(
        config::systemd_service_service_name("default"),
        "nexus-link-service"
    );
}

#[test]
fn test_systemd_service_names_named_profile() {
    assert_eq!(
        config::systemd_agent_service_name("staging"),
        "nexus-link-agent@staging"
    );
    assert_eq!(
        config::systemd_service_service_name("staging"),
        "nexus-link-service@staging"
    );
}

#[test]
fn test_active_profile_default() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    unsafe { std::env::remove_var("NEXUS_LINK_PROFILE") };
    assert_eq!(config::active_profile(), DEFAULT_PROFILE);
}

#[test]
fn test_active_profile_env_override() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    unsafe { std::env::set_var("NEXUS_LINK_PROFILE", "staging") };
    assert_eq!(config::active_profile(), "staging");
    unsafe { std::env::remove_var("NEXUS_LINK_PROFILE") };
}

#[test]
fn test_profile_config_path_resolution() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    unsafe { std::env::remove_var("NEXUS_LINK_CONFIG") };
    unsafe { std::env::set_var("NEXUS_LINK_PROFILE", "staging") };
    let path = config::effective_config_path();
    // Non-default profile should resolve to config.staging.toml under system dir
    assert!(
        path.to_string_lossy().contains("config.staging.toml"),
        "Expected config.staging.toml in path, got: {}",
        path.display()
    );
    unsafe { std::env::remove_var("NEXUS_LINK_PROFILE") };
}

#[test]
fn test_nexus_link_config_env_overrides_profile() {
    let _lock = crate::ENV_MUTEX.lock().unwrap();
    // NEXUS_LINK_CONFIG must take precedence over NEXUS_LINK_PROFILE
    unsafe { std::env::set_var("NEXUS_LINK_PROFILE", "staging") };
    unsafe { std::env::set_var("NEXUS_LINK_CONFIG", "/custom/override.toml") };
    let path = config::effective_config_path();
    assert_eq!(path, PathBuf::from("/custom/override.toml"));
    unsafe { std::env::remove_var("NEXUS_LINK_CONFIG") };
    unsafe { std::env::remove_var("NEXUS_LINK_PROFILE") };
}

#[test]
fn test_setup_secures_config_and_key_files_only() {
    use nexus_link_core::setup::is_system_config_file_name;
    for name in ["config.toml", "config.staging.toml", "signing_key.pub"] {
        assert!(is_system_config_file_name(name), "{name}");
    }
    for name in [
        "config.toml.bak",
        "README",
        "tls.pem",
        "signing_key.pub.old",
    ] {
        assert!(!is_system_config_file_name(name), "{name}");
    }
}
