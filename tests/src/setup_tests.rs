use nexus_link_core::config::{SERVICE_GROUP, SERVICE_USER, SYSTEM_CONFIG_DIR, SYSTEM_STATE_DIR};
use nexus_link_core::setup::{SetupReport, SetupStep, StepStatus};

#[test]
fn test_setup_constants_are_consistent() {
    assert_eq!(SERVICE_USER, "nexus-link");
    assert_eq!(SERVICE_GROUP, "nexus-link");
    assert_eq!(SYSTEM_CONFIG_DIR, "/etc/nexus-link");
    assert_eq!(SYSTEM_STATE_DIR, "/var/lib/nexus-link");
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
