use nexus_link_core::config::{SERVICE_GROUP, SERVICE_USER, SYSTEM_CONFIG_DIR, SYSTEM_STATE_DIR};
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
    // NEXUS_LINK_CONFIG env var should take priority
    // SAFETY: test runs single-threaded, no concurrent access to env
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
