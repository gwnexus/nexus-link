/// Guards all tests that mutate the `NEXUS_LINK_CONFIG` environment variable.
/// All env-var tests across modules must acquire this lock to prevent races.
#[cfg(test)]
pub(crate) static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod error_tests;
#[cfg(test)]
mod preflight_tests;
#[cfg(test)]
mod setup_tests;
#[cfg(test)]
mod telemetry_tests;
#[cfg(test)]
mod token_tests;
#[cfg(test)]
mod types_tests;
