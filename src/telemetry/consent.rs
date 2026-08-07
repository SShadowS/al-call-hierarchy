//! Telemetry enable/disable resolution.
//!
//! See spec §7 "Resolution order for `enabled`". Three tiers:
//! hard-off (DNT, --no-telemetry, `AL_SEM_TELEMETRY`/`AL_CH_TELEMETRY`=0),
//! hard-on (`AL_SEM_TELEMETRY`/`AL_CH_TELEMETRY`=1, init-option, config),
//! defaults (off in debug/test/CI; on in release for interactive use).
//!
//! Two spellings of the switch are accepted. `AL_SEM_TELEMETRY` matches the crate name;
//! `AL_CH_TELEMETRY` is the pre-rename name and keeps working, because a variable set in
//! someone's shell profile or CI config is not ours to invalidate. Either one being `0`
//! is enough to disable — the OFF reading always wins, so adding the new name can never
//! turn telemetry ON for someone who had switched it off.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// `NoConnectionString` not yet emitted — future design (telemetry transport).
#[allow(dead_code)]
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

/// The telemetry switch, current name first. Both are honoured — see the module doc.
pub const TELEMETRY_ENV_VARS: &[&str] = &["AL_SEM_TELEMETRY", "AL_CH_TELEMETRY"];

/// True when any accepted spelling of the switch is set to `value`.
fn telemetry_env_is(inputs: &Inputs, value: &str) -> bool {
    TELEMETRY_ENV_VARS
        .iter()
        .any(|k| inputs.env.get(*k).map(|s| s.as_str()) == Some(value))
}

pub fn decide(inputs: &Inputs) -> Decision {
    // Hard-off tier
    if inputs.env.get("DO_NOT_TRACK").map(|s| s.as_str()) == Some("1") {
        return Decision::Disabled(DisabledReason::DoNotTrack);
    }
    if inputs.cli_no_telemetry {
        return Decision::Disabled(DisabledReason::CliFlag);
    }
    // Checked before the hard-on tier below, so a `0` under either spelling beats a `1`
    // under the other.
    if telemetry_env_is(inputs, "0") {
        return Decision::Disabled(DisabledReason::EnvOff);
    }

    // Hard-on tier
    if telemetry_env_is(inputs, "1") {
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
        let i = with_env(&[("AL_SEM_TELEMETRY", "0")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::EnvOff));
    }

    #[test]
    fn env_one_overrides_ci_default() {
        let i = with_env(&[("CI", "true"), ("AL_SEM_TELEMETRY", "1")]);
        assert_eq!(decide(&i), Decision::Enabled);
    }

    // The pre-rename spelling is a promise to anyone who already set it. Each test below
    // states the env map literally rather than asking `TELEMETRY_ENV_VARS` for a name,
    // so shortening that list to one entry fails these instead of silently passing.

    #[test]
    fn the_pre_rename_env_var_still_disables() {
        let i = with_env(&[("AL_CH_TELEMETRY", "0")]);
        assert_eq!(
            decide(&i),
            Decision::Disabled(DisabledReason::EnvOff),
            "AL_CH_TELEMETRY=0 in a shell profile must keep working after the rename"
        );
    }

    #[test]
    fn the_pre_rename_env_var_still_enables() {
        let i = with_env(&[("CI", "true"), ("AL_CH_TELEMETRY", "1")]);
        assert_eq!(decide(&i), Decision::Enabled);
    }

    #[test]
    fn off_under_either_spelling_beats_on_under_the_other() {
        let off_is_new = with_env(&[("AL_SEM_TELEMETRY", "0"), ("AL_CH_TELEMETRY", "1")]);
        let off_is_old = with_env(&[("AL_SEM_TELEMETRY", "1"), ("AL_CH_TELEMETRY", "0")]);
        assert_eq!(
            decide(&off_is_new),
            Decision::Disabled(DisabledReason::EnvOff),
            "adding a second accepted name must never turn telemetry on for someone who \
             had switched it off"
        );
        assert_eq!(
            decide(&off_is_old),
            Decision::Disabled(DisabledReason::EnvOff)
        );
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
