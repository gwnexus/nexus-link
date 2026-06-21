use nexus_link_core::preflight::{CheckStatus, PreflightVerdict, run_preflight};

#[test]
fn test_preflight_runs_without_panic() {
    // Should not panic regardless of platform
    let report = run_preflight();
    assert!(!report.checks.is_empty());
}

#[test]
fn test_preflight_has_required_checks() {
    let report = run_preflight();
    let check_names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();

    assert!(check_names.contains(&"architecture"));
    assert!(check_names.contains(&"gpu"));
    assert!(check_names.contains(&"docker"));
    assert!(check_names.contains(&"network"));
    assert!(check_names.contains(&"disk"));
}

#[test]
fn test_preflight_architecture_not_fail_on_any_platform() {
    let report = run_preflight();
    let arch_check = report
        .checks
        .iter()
        .find(|c| c.name == "architecture")
        .unwrap();
    // Architecture should never be Fail (even macOS is Warn, not Fail)
    assert_ne!(arch_check.status, CheckStatus::Fail);
}

#[test]
fn test_preflight_verdict_values() {
    // Test that enum variants are distinct
    assert_ne!(
        PreflightVerdict::Compatible,
        PreflightVerdict::NotRecommended
    );
    assert_ne!(PreflightVerdict::Compatible, PreflightVerdict::Incompatible);
    assert_ne!(
        PreflightVerdict::NotRecommended,
        PreflightVerdict::Incompatible
    );
}

#[test]
fn test_preflight_check_status_values() {
    assert_ne!(CheckStatus::Pass, CheckStatus::Warn);
    assert_ne!(CheckStatus::Pass, CheckStatus::Fail);
    assert_ne!(CheckStatus::Warn, CheckStatus::Fail);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn test_preflight_non_linux_gives_arch_warning() {
    let report = run_preflight();
    let arch_check = report
        .checks
        .iter()
        .find(|c| c.name == "architecture")
        .unwrap();
    assert_eq!(arch_check.status, CheckStatus::Warn);
    assert!(arch_check.detail.contains("non-Linux"));
}
