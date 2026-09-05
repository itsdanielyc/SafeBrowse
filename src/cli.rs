//! Validated launch options shared by the launcher and its desktop worker.

use crate::browser::ProfileMode;

/// Options that affect the lifetime and protection of one browser session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptions {
    pub windowed: bool,
    pub worker: bool,
    pub profile_mode: ProfileMode,
    pub allow_screen_recording: bool,
    pub target_url: Option<String>,
    pub show_help: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            windowed: false,
            worker: false,
            profile_mode: ProfileMode::Ephemeral,
            allow_screen_recording: false,
            target_url: None,
            show_help: false,
        }
    }
}

impl LaunchOptions {
    /// Parses arguments after the executable name, rejecting ambiguous or unsupported options.
    /// Time: O(n) and space: O(n), where n is the total argument length.
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--help" | "-h" => options.show_help = true,
                "--windowed" | "-w" => options.windowed = true,
                "--worker" => {
                    if options.worker {
                        return Err("The internal --worker flag may only be provided once".into());
                    }
                    options.worker = true;
                }
                "--persistent" | "-p" => options.profile_mode = ProfileMode::Persistent,
                "--allow-screen-recording" => options.allow_screen_recording = true,
                "--url" => {
                    if options.target_url.is_some() {
                        return Err("--url may only be provided once".into());
                    }
                    let value = arguments
                        .next()
                        .ok_or("--url requires an HTTP or HTTPS URL")?;
                    options.target_url = Some(validate_launch_url(&value)?);
                }
                _ => {
                    return Err(format!(
                        "Unknown argument: {argument}. Use --help for usage."
                    ))
                }
            }
        }
        if options.windowed && options.worker {
            return Err("--windowed and the internal --worker flag cannot be combined".into());
        }
        Ok(options)
    }

    /// Serializes supported options for the isolated worker without inheriting windowed mode.
    pub fn worker_arguments(&self) -> Vec<String> {
        let mut arguments = vec!["--worker".to_string()];
        if self.profile_mode == ProfileMode::Persistent {
            arguments.push("--persistent".into());
        }
        if self.allow_screen_recording {
            arguments.push("--allow-screen-recording".into());
        }
        if let Some(url) = &self.target_url {
            arguments.push("--url".into());
            arguments.push(url.clone());
        }
        arguments
    }
}

/// Accepts only explicit web destinations, avoiding local files and executable URL schemes.
fn validate_launch_url(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err("--url must not contain control characters".into());
    }
    let url = url::Url::parse(value).map_err(|error| format!("Invalid --url: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("--url requires an absolute HTTP or HTTPS URL".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("--url must not contain embedded credentials".into());
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::LaunchOptions;

    fn parse(arguments: &[&str]) -> Result<LaunchOptions, String> {
        LaunchOptions::parse(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn recording_requires_explicit_flag_even_when_windowed() {
        assert!(!parse(&[]).unwrap().allow_screen_recording);
        assert!(!parse(&["--windowed"]).unwrap().allow_screen_recording);
        assert!(
            parse(&["--windowed", "--allow-screen-recording"])
                .unwrap()
                .allow_screen_recording
        );
    }

    #[test]
    fn worker_round_trip_preserves_url_and_explicit_protection_mode() {
        let options = parse(&[
            "--persistent",
            "--allow-screen-recording",
            "--url",
            "https://example.com/?q=a b&next=\"test\"",
        ])
        .unwrap();
        let worker = LaunchOptions::parse(options.worker_arguments()).unwrap();
        assert!(worker.worker);
        assert!(worker.allow_screen_recording);
        assert_eq!(worker.target_url, options.target_url);
        assert_eq!(worker.profile_mode, options.profile_mode);
    }

    #[test]
    fn malformed_or_ambiguous_arguments_fail_before_startup() {
        for arguments in [
            vec!["--allow-screenrecording"],
            vec!["--url"],
            vec!["--url", "--windowed"],
            vec!["--url", "javascript:alert(1)"],
            vec!["--url", "file:///C:/Windows/win.ini"],
            vec!["--url", "https://user:secret@example.com"],
            vec!["--url", "https://example.com\n"],
            vec![
                "--url",
                "https://example.com",
                "--url",
                "https://other.example",
            ],
            vec!["--worker", "--windowed"],
            vec!["--worker", "--worker"],
        ] {
            assert!(parse(&arguments).is_err(), "accepted {arguments:?}");
        }
    }
}
