//! Telemetry enable/disable resolution.
//!
//! See spec §7 "Resolution order for `enabled`". Three tiers:
//! hard-off (DNT, --no-telemetry, AL_CH_TELEMETRY=0),
//! hard-on (AL_CH_TELEMETRY=1, init-option, config),
//! defaults (off in debug/test/CI; on in release for interactive use).

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledReason {
    DoNotTrack,
    CliFlag,
    EnvOff,
    DebugBuild,
    CfgTest,
    CiEnvironment,
    ConfigOff,
    InitOptionOff,
    NoConnectionString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Enabled,
    Disabled(DisabledReason),
}

/// Inputs to the consent decision. Centralized to keep tests pure.
#[derive(Debug, Default, Clone)]
pub struct Inputs {
    /// `--no-telemetry` CLI flag.
    pub cli_no_telemetry: bool,
    /// LSP `initializationOptions.telemetry.enabled`, if provided.
    pub init_option: Option<bool>,
    /// `config.json` `telemetry.enabled`, if provided.
    pub config: Option<bool>,
    /// All environment variables, snapshotted (testable).
    pub env: HashMap<String, String>,
    /// True for `cfg(debug_assertions)` builds.
    pub is_debug: bool,
    /// True for `cfg(test)` builds.
    pub is_test: bool,
}

const CI_ENV_VARS: &[&str] = &[
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "TRAVIS",
    "JENKINS_URL",
    "TEAMCITY_VERSION",
    "TF_BUILD",
];

pub fn decide(inputs: &Inputs) -> Decision {
    // Hard-off tier
    if inputs.env.get("DO_NOT_TRACK").map(|s| s.as_str()) == Some("1") {
        return Decision::Disabled(DisabledReason::DoNotTrack);
    }
    if inputs.cli_no_telemetry {
        return Decision::Disabled(DisabledReason::CliFlag);
    }
    if inputs.env.get("AL_CH_TELEMETRY").map(|s| s.as_str()) == Some("0") {
        return Decision::Disabled(DisabledReason::EnvOff);
    }

    // Hard-on tier
    if inputs.env.get("AL_CH_TELEMETRY").map(|s| s.as_str()) == Some("1") {
        return Decision::Enabled;
    }
    if let Some(true) = inputs.init_option {
        return Decision::Enabled;
    }
    if let Some(false) = inputs.init_option {
        return Decision::Disabled(DisabledReason::InitOptionOff);
    }
    if let Some(true) = inputs.config {
        return Decision::Enabled;
    }
    if let Some(false) = inputs.config {
        return Decision::Disabled(DisabledReason::ConfigOff);
    }

    // Default heuristics
    if inputs.is_test {
        return Decision::Disabled(DisabledReason::CfgTest);
    }
    if inputs.is_debug {
        return Decision::Disabled(DisabledReason::DebugBuild);
    }
    for var in CI_ENV_VARS {
        if inputs.env.contains_key(*var) {
            return Decision::Disabled(DisabledReason::CiEnvironment);
        }
    }

    Decision::Enabled
}

/// Snapshot the current process environment.
pub fn live_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Inputs {
        Inputs::default()
    }

    fn with_env(pairs: &[(&str, &str)]) -> Inputs {
        let mut i = empty();
        i.env = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        i
    }

    #[test]
    fn defaults_to_enabled_in_release_interactive() {
        // is_debug=false, is_test=false, no env, no config.
        let i = empty();
        assert_eq!(decide(&i), Decision::Enabled);
    }

    #[test]
    fn do_not_track_disables() {
        let i = with_env(&[("DO_NOT_TRACK", "1")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::DoNotTrack));
    }

    #[test]
    fn cli_flag_disables() {
        let mut i = empty();
        i.cli_no_telemetry = true;
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CliFlag));
    }

    #[test]
    fn env_zero_disables() {
        let i = with_env(&[("AL_CH_TELEMETRY", "0")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::EnvOff));
    }

    #[test]
    fn env_one_overrides_ci_default() {
        let i = with_env(&[("CI", "true"), ("AL_CH_TELEMETRY", "1")]);
        assert_eq!(decide(&i), Decision::Enabled);
    }

    #[test]
    fn ci_env_disables_by_default() {
        let i = with_env(&[("CI", "true")]);
        assert_eq!(
            decide(&i),
            Decision::Disabled(DisabledReason::CiEnvironment)
        );
    }

    #[test]
    fn github_actions_disables_by_default() {
        let i = with_env(&[("GITHUB_ACTIONS", "true")]);
        assert_eq!(
            decide(&i),
            Decision::Disabled(DisabledReason::CiEnvironment)
        );
    }

    #[test]
    fn debug_build_disables_by_default() {
        let mut i = empty();
        i.is_debug = true;
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::DebugBuild));
    }

    #[test]
    fn cfg_test_disables_by_default() {
        let mut i = empty();
        i.is_test = true;
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CfgTest));
    }

    #[test]
    fn dnt_beats_explicit_on() {
        let i = with_env(&[("DO_NOT_TRACK", "1"), ("AL_CH_TELEMETRY", "1")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::DoNotTrack));
    }

    #[test]
    fn cli_flag_beats_config_on() {
        let mut i = empty();
        i.cli_no_telemetry = true;
        i.config = Some(true);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CliFlag));
    }

    #[test]
    fn init_option_overrides_config() {
        let mut i = empty();
        i.config = Some(true);
        i.init_option = Some(false);
        assert_eq!(
            decide(&i),
            Decision::Disabled(DisabledReason::InitOptionOff)
        );
    }
}
