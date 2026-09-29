// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The single error type used across `cirrocast`.
//!
//! Every fallible operation returns [`Result<T>`] and reports failures as one of the [`Error`]
//! variants below. The variant also decides the process exit code that `main` hands back to the
//! shell, so the mapping in [`Error::exit_code`] is part of the CLI's public contract:
//!
//! | code | meaning                          |
//! |------|----------------------------------|
//! | 0    | success                          |
//! | 1    | generic / unexpected             |
//! | 2    | command line usage               |
//! | 3    | network or upstream failure      |
//! | 4    | configuration or state on disk   |
//! | 5    | location not found               |
//! | 6    | missing or invalid API key       |

/// Crate-wide result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong while running `cirrocast`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The user asked for something that does not exist or cannot be combined.
    #[error("{0}")]
    Usage(String),

    /// The request never reached a provider, or the transport failed on the way.
    #[error("network error: {0}")]
    Network(String),

    /// A provider answered, but with an error status or an unusable body.
    #[error("{}", upstream_message(.provider, *.status, .message))]
    Upstream {
        /// Provider id, e.g. `open-meteo`.
        provider: String,
        /// HTTP status code, when the failure came with a response.
        status: Option<u16>,
        /// Human readable detail.
        message: String,
    },

    /// Configuration or state on disk is missing, unreadable or invalid.
    #[error("config error: {0}")]
    Config(String),

    /// A location string could not be resolved to a place.
    #[error("location not found: {0}")]
    LocationNotFound(String),

    /// A provider that needs an API key did not find one.
    #[error("missing API key for {provider}: set {env}")]
    MissingKey {
        /// Provider id, e.g. `qweather`.
        provider: String,
        /// Environment variable that would supply the key.
        env: String,
    },

    /// Anything else: an unexpected failure that still has to exit non-zero.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// The process exit code this error maps to.
    ///
    /// See the table in the module documentation; the values are stable and script-visible.
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Other(_) => 1,
            Self::Usage(_) => 2,
            Self::Network(_) | Self::Upstream { .. } => 3,
            Self::Config(_) => 4,
            Self::LocationNotFound(_) => 5,
            Self::MissingKey { .. } => 6,
        }
    }
}

// `Network` and `Config` carry a ready-to-print message instead of the original error object, so
// their conversions are written by hand rather than derived: the payload stays a `String` and the
// underlying error text is folded into it. Formatting the context is the variant's own job.

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Network(error.to_string())
    }
}

impl From<etcetera::HomeDirError> for Error {
    fn from(error: etcetera::HomeDirError) -> Self {
        Self::Config(error.to_string())
    }
}

/// Renders [`Error::Upstream`] with or without an HTTP status.
fn upstream_message(provider: &str, status: Option<u16>, message: &str) -> String {
    match status {
        Some(code) => format!("provider {provider} answered HTTP {code}: {message}"),
        None => format!("provider {provider} failed: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::Error;

    fn cases() -> Vec<(Error, u8)> {
        vec![
            (Error::Other("boom".into()), 1),
            (Error::Usage("bad flag".into()), 2),
            (Error::Network("timeout".into()), 3),
            (
                Error::Upstream {
                    provider: "open-meteo".into(),
                    status: Some(503),
                    message: "unavailable".into(),
                },
                3,
            ),
            (Error::Config("bad toml".into()), 4),
            (Error::LocationNotFound("Atlantis".into()), 5),
            (
                Error::MissingKey {
                    provider: "qweather".into(),
                    env: "CIRROCAST_QWEATHER_KEY".into(),
                },
                6,
            ),
        ]
    }

    #[test]
    fn exit_codes_follow_the_contract() {
        for (error, expected) in cases() {
            assert_eq!(error.exit_code(), expected, "wrong code for {error}");
        }
    }

    #[test]
    fn upstream_message_includes_status_when_known() {
        let with_status = Error::Upstream {
            provider: "weatherapi".into(),
            status: Some(429),
            message: "quota exceeded".into(),
        };
        assert_eq!(
            with_status.to_string(),
            "provider weatherapi answered HTTP 429: quota exceeded"
        );

        let without_status = Error::Upstream {
            provider: "weatherapi".into(),
            status: None,
            message: "truncated body".into(),
        };
        assert_eq!(
            without_status.to_string(),
            "provider weatherapi failed: truncated body"
        );
    }

    #[test]
    fn variants_name_what_the_user_has_to_do() {
        assert_eq!(
            Error::MissingKey {
                provider: "qweather".into(),
                env: "CIRROCAST_QWEATHER_KEY".into(),
            }
            .to_string(),
            "missing API key for qweather: set CIRROCAST_QWEATHER_KEY"
        );
        assert_eq!(
            Error::LocationNotFound("Atlantis".into()).to_string(),
            "location not found: Atlantis"
        );
    }

    #[test]
    fn underlying_errors_convert_into_their_variant() {
        let io = Error::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "connection timed out",
        ));
        assert!(matches!(io, Error::Network(_)));
        assert!(io.to_string().contains("connection timed out"));

        let home = Error::from(etcetera::HomeDirError);
        assert!(matches!(home, Error::Config(_)));
        assert_eq!(home.exit_code(), 4);
    }
}
