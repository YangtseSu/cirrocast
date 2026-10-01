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
    #[error(
        "missing API key for {provider}: run `cirrocast key set {provider}` or set {env} in the environment"
    )]
    MissingKey {
        /// Provider id, e.g. `qweather`.
        provider: String,
        /// Environment variable that would supply the key.
        env: String,
    },

    /// A provider rejected the API key it was given.
    ///
    /// Separate from [`Error::Upstream`] because the answer is never "try again": the credential
    /// is wrong, disabled or not entitled, and only the user can fix it.
    #[error(
        "provider {provider} rejected the API key (HTTP {status}): replace it with `cirrocast key set {provider}`"
    )]
    InvalidKey {
        /// Provider id, e.g. `openweathermap`.
        provider: String,
        /// The HTTP status the rejection came with (`401` today; a provider may refine it).
        status: u16,
    },

    /// Every entry of a fallback chain failed; the message names each attempt in order.
    #[error("all {subject} failed: {}", attempts.join("; "))]
    Chain {
        /// What the chain was made of, for the message: `providers`, `IP location services`.
        subject: &'static str,
        /// One `name (reason)` entry per attempt.
        attempts: Vec<String>,
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
            Self::Network(_) | Self::Upstream { .. } | Self::Chain { .. } => 3,
            Self::Config(_) => 4,
            Self::LocationNotFound(_) => 5,
            Self::MissingKey { .. } | Self::InvalidKey { .. } => 6,
        }
    }

    /// The short classifier a fallback warning or a chain-attempt entry names: `network: …` or
    /// `upstream: …`; anything else keeps its own wording.
    #[must_use]
    pub fn chain_reason(&self) -> String {
        match self {
            Self::Network(message) => format!("network: {message}"),
            Self::Upstream { message, .. } => format!("upstream: {message}"),
            other => other.to_string(),
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
            (
                Error::InvalidKey {
                    provider: "openweathermap".into(),
                    status: 401,
                },
                6,
            ),
            (
                Error::Chain {
                    subject: "providers",
                    attempts: vec![
                        "open-meteo (network: timeout)".to_owned(),
                        "smhi (upstream: out of coverage)".to_owned(),
                    ],
                },
                3,
            ),
        ]
    }

    #[test]
    fn a_chain_failure_names_every_attempt() {
        let error = Error::Chain {
            subject: "providers",
            attempts: vec![
                "open-meteo (network: timeout after 15s)".to_owned(),
                "smhi (upstream: out of coverage: 39.90,116.40)".to_owned(),
            ],
        };
        assert_eq!(
            error.to_string(),
            "all providers failed: open-meteo (network: timeout after 15s); \
             smhi (upstream: out of coverage: 39.90,116.40)"
        );

        let error = Error::Chain {
            subject: "IP location services",
            attempts: vec![
                "ipwho.is (network: timeout)".to_owned(),
                "ipapi.co (upstream: RateLimited)".to_owned(),
            ],
        };
        assert_eq!(
            error.to_string(),
            "all IP location services failed: ipwho.is (network: timeout); \
             ipapi.co (upstream: RateLimited)"
        );
        assert_eq!(error.exit_code(), 3);
    }

    #[test]
    fn an_invalid_key_names_the_fix() {
        let error = Error::InvalidKey {
            provider: "weatherapi".into(),
            status: 401,
        };
        assert_eq!(
            error.to_string(),
            "provider weatherapi rejected the API key (HTTP 401): \
             replace it with `cirrocast key set weatherapi`"
        );
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
            "missing API key for qweather: run `cirrocast key set qweather` or set \
             CIRROCAST_QWEATHER_KEY in the environment"
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
