pub const DEFAULT_DISPLAY: &str = ":0";
pub const DEFAULT_MIRROR_CLEARS: bool = true;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub display: String,
    pub mirror_clears: bool,
}

impl Config {
    pub fn render(&self) -> String {
        format!(
            "display={}\nmirror_clears={}\n",
            self.display, self.mirror_clears
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    pub display: Option<String>,
    pub mirror_clears: Option<String>,
}

impl Env {
    pub fn capture() -> Self {
        Self {
            display: std::env::var("DISPLAY").ok(),
            mirror_clears: std::env::var("MIRROR_CLEARS").ok(),
        }
    }
}

pub fn parse_bool(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

pub fn resolve(
    display_override: Option<&str>,
    mirror_clears_override: Option<bool>,
    env: &Env,
) -> Config {
    let display = display_override
        .map(str::to_owned)
        .or_else(|| env.display.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_DISPLAY.to_owned());

    let mirror_clears = mirror_clears_override
        .or_else(|| env.mirror_clears.as_deref().map(parse_bool))
        .unwrap_or(DEFAULT_MIRROR_CLEARS);

    Config {
        display,
        mirror_clears,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(display: Option<&str>, mirror_clears: Option<&str>) -> Env {
        Env {
            display: display.map(str::to_owned),
            mirror_clears: mirror_clears.map(str::to_owned),
        }
    }

    #[test]
    fn boolean_spellings() {
        for truthy in ["1", "true", "yes", "on", "TRUE", "anything", ""] {
            assert!(parse_bool(truthy), "{truthy} should be truthy");
        }
        for falsy in ["0", "false", "no", "off", "OFF", " false "] {
            assert!(!parse_bool(falsy), "{falsy} should be falsy");
        }
    }

    #[test]
    fn defaults_are_used_when_nothing_is_set() {
        let config = resolve(None, None, &Env::default());
        assert_eq!(config.display, DEFAULT_DISPLAY);
        assert!(config.mirror_clears);
    }

    #[test]
    fn empty_environment_values_fall_back_to_defaults() {
        let config = resolve(None, None, &env(Some(""), Some("")));
        assert_eq!(config.display, DEFAULT_DISPLAY);
        assert!(config.mirror_clears);
    }

    #[test]
    fn environment_is_used_when_no_flag_is_given() {
        let config = resolve(None, None, &env(Some(":7"), Some("off")));
        assert_eq!(config.display, ":7");
        assert!(!config.mirror_clears);
    }

    #[test]
    fn flags_win_over_the_environment() {
        let config = resolve(Some(":42"), Some(true), &env(Some(":7"), Some("off")));
        assert_eq!(config.display, ":42");
        assert!(config.mirror_clears);
    }

    #[test]
    fn partial_overrides_only_touch_their_field() {
        let config = resolve(Some(":42"), None, &env(Some(":7"), Some("false")));
        assert_eq!(config.display, ":42");
        assert!(!config.mirror_clears);
    }

    #[test]
    fn render_is_stable_key_value_lines() {
        let config = Config {
            display: ":9".to_owned(),
            mirror_clears: false,
        };
        assert_eq!(config.render(), "display=:9\nmirror_clears=false\n");
    }
}
