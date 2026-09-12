use std::fmt;

use crate::config::{parse_bool, resolve, Config, Env};

pub const USAGE: &str = "\
watari - mirror the X11 (XWayland) CLIPBOARD selection to the Wayland clipboard

USAGE:
    watari [OPTIONS]

OPTIONS:
    --display <DISPLAY>     X server to connect to [env: DISPLAY] [default: :0]
    --mirror-clears[=BOOL]  Propagate an X CLIPBOARD clear to Wayland
                            [env: MIRROR_CLEARS] [default: true]
    --no-mirror-clears      Shorthand for --mirror-clears=false
    --print-config          Print the resolved configuration and exit
    -h, --help              Print this help and exit
    -V, --version           Print the version and exit

ENVIRONMENT:
    DISPLAY        X server to connect to (default :0)
    MIRROR_CLEARS  Whether to propagate clipboard clears (default true)
    RUST_LOG       env_logger filter, e.g. info or debug (default info)
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run(Config),
    PrintConfig(Config),
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    message: String,
}

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

pub fn parse<I>(args: I, env: &Env) -> Result<Action, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut display_override: Option<String> = None;
    let mut mirror_clears_override: Option<bool> = None;
    let mut print_config = false;
    let mut help = false;
    let mut version = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            "--print-config" => print_config = true,
            "--mirror-clears" => mirror_clears_override = Some(true),
            "--no-mirror-clears" => mirror_clears_override = Some(false),
            "--display" => {
                let value = args
                    .next()
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| CliError::new("--display requires a value"))?;
                display_override = Some(value);
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--mirror-clears=") {
                    mirror_clears_override = Some(parse_bool(value));
                } else if let Some(value) = arg.strip_prefix("--display=") {
                    display_override = Some(value.to_owned());
                } else {
                    return Err(CliError::new(format!("unknown argument: {arg}")));
                }
            }
        }
    }

    if help {
        return Ok(Action::Help);
    }
    if version {
        return Ok(Action::Version);
    }

    let config = resolve(display_override.as_deref(), mirror_clears_override, env);
    Ok(if print_config {
        Action::PrintConfig(config)
    } else {
        Action::Run(config)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_DISPLAY, DEFAULT_MIRROR_CLEARS};

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn no_arguments_runs_with_defaults() {
        let action = parse(args(&[]), &Env::default()).unwrap();
        assert_eq!(
            action,
            Action::Run(Config {
                display: DEFAULT_DISPLAY.to_owned(),
                mirror_clears: DEFAULT_MIRROR_CLEARS,
            })
        );
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert_eq!(
            parse(args(&["--help"]), &Env::default()).unwrap(),
            Action::Help
        );
        assert_eq!(parse(args(&["-h"]), &Env::default()).unwrap(), Action::Help);
        assert_eq!(
            parse(args(&["--version"]), &Env::default()).unwrap(),
            Action::Version
        );
        assert_eq!(
            parse(args(&["-V"]), &Env::default()).unwrap(),
            Action::Version
        );
    }

    #[test]
    fn help_beats_version_beats_run() {
        assert_eq!(
            parse(args(&["--version", "--help"]), &Env::default()).unwrap(),
            Action::Help
        );
        assert_eq!(
            parse(args(&["--print-config", "--version"]), &Env::default()).unwrap(),
            Action::Version
        );
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        let error = parse(args(&["--nope"]), &Env::default()).unwrap_err();
        assert!(error.to_string().contains("--nope"));
        assert!(parse(args(&["positional"]), &Env::default()).is_err());
    }

    #[test]
    fn print_config_is_reported() {
        let action = parse(args(&["--print-config"]), &Env::default()).unwrap();
        assert_eq!(
            action,
            Action::PrintConfig(Config {
                display: DEFAULT_DISPLAY.to_owned(),
                mirror_clears: true,
            })
        );
    }

    #[test]
    fn mirror_clear_flags_are_parsed() {
        let config = |action| match action {
            Action::Run(config) => config,
            other => panic!("expected Run, got {other:?}"),
        };

        assert!(
            !config(parse(args(&["--no-mirror-clears"]), &Env::default()).unwrap()).mirror_clears
        );
        assert!(
            !config(parse(args(&["--mirror-clears=false"]), &Env::default()).unwrap())
                .mirror_clears
        );
        assert!(
            config(parse(args(&["--mirror-clears=yes"]), &Env::default()).unwrap()).mirror_clears
        );
        assert!(config(parse(args(&["--mirror-clears"]), &Env::default()).unwrap()).mirror_clears);
    }

    #[test]
    fn display_flag_overrides_environment() {
        let env = Env {
            display: Some(":1".to_owned()),
            mirror_clears: None,
        };
        let action = parse(args(&["--display=:9"]), &env).unwrap();
        match action {
            Action::Run(config) => assert_eq!(config.display, ":9"),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn display_flag_accepts_a_separate_value() {
        let action = parse(args(&["--display", ":9"]), &Env::default()).unwrap();
        match action {
            Action::Run(config) => assert_eq!(config.display, ":9"),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn display_flag_without_a_value_is_rejected() {
        assert!(parse(args(&["--display"]), &Env::default()).is_err());
        assert!(parse(args(&["--display", "--help"]), &Env::default()).is_err());
    }

    #[test]
    fn usage_documents_every_flag() {
        for flag in [
            "--display",
            "--mirror-clears",
            "--no-mirror-clears",
            "--print-config",
            "--help",
            "--version",
        ] {
            assert!(USAGE.contains(flag), "usage is missing {flag}");
        }
    }
}
