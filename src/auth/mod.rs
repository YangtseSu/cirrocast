// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Credential presentation: turning a stored [`Credential`] into the header a request carries.
//!
//! `QWeather` is the one backend with two authentication modes (step 27), so this module exposes a
//! single resolver rather than a trait: [`QWeatherAuth::resolve`] turns the credential the key store
//! resolved into the header to send — the API key itself, or a JWT minted at that instant — and
//! [`QWeatherAuth::apply`] puts it on a request with the value registered as a secret. The forecast
//! backend and the alert source call the same two methods, so both follow the configured mode with
//! no second code path.

pub mod jwt;

use std::time::SystemTime;

use crate::config::keys::Credential;
use crate::error::Result;
use crate::http::HttpRequest;

/// The `QWeather` credential header, resolved once per fetch.
///
/// A JWT is minted by [`QWeatherAuth::resolve`] and then reused for every request of that fetch;
/// two fetches at different instants mint two different tokens, and nothing here is persisted.
#[derive(Clone)]
pub struct QWeatherAuth {
    /// The header name: `X-QW-Api-Key` for an API key, `Authorization` for a bearer token.
    name: &'static str,
    /// The value that goes on the wire.
    value: String,
    /// The part of `value` that must never reach a log line, an error or a cache envelope.
    secret: String,
}

impl std::fmt::Debug for QWeatherAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The value is a credential; only its header name is printable.
        formatter
            .debug_struct("QWeatherAuth")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl QWeatherAuth {
    /// Resolves `credential` into the header to send, minting a token at `now` for the JWT form.
    pub fn resolve(credential: &Credential, now: SystemTime) -> Result<Self> {
        Ok(match credential {
            Credential::ApiKey(key) => Self {
                name: "X-QW-Api-Key",
                value: key.clone(),
                secret: key.clone(),
            },
            Credential::QWeatherJwt(credential) => {
                let token = jwt::qweather_token(credential, now)?;
                Self {
                    name: "Authorization",
                    value: format!("Bearer {token}"),
                    secret: token,
                }
            }
        })
    }

    /// Applies the header to `request`, registering [`Self::secret`] so every redacted spelling of
    /// the request shows `***` where the credential was.
    #[must_use]
    pub fn apply(&self, request: HttpRequest) -> HttpRequest {
        request
            .header(self.name, self.value.clone())
            .secret(self.secret.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::QWeatherAuth;
    use crate::config::keys::{Credential, JwtCredential};
    use crate::http::HttpRequest;

    /// The throwaway key `tests/fixtures/qweather/ed25519-test-key.pem` holds, inlined so the unit
    /// test does not depend on the integration fixture.
    const TEST_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
                            MC4CAQAwBQYDK2VwBCIEIGz9V4O1zvyyI4YmI4MaqhC+IzBG728KXXzqz4z2sKqA\n\
                            -----END PRIVATE KEY-----\n";

    fn credential() -> JwtCredential {
        JwtCredential {
            credential_id: "ABCDE12345".to_owned(),
            developer_id: "Q12345ABCD".to_owned(),
            project_id: "ABC2345DEF".to_owned(),
            private_key: TEST_PEM.to_owned(),
        }
    }

    #[test]
    fn an_api_key_travels_in_the_documented_header() {
        let auth = QWeatherAuth::resolve(&Credential::ApiKey("secret-key".to_owned()), UNIX_EPOCH)
            .expect("an API key needs no clock");
        let request = auth.apply(HttpRequest::get(
            "https://example.re.qweatherapi.com/current",
        ));
        assert_eq!(
            request.headers(),
            [("X-QW-Api-Key".to_owned(), "secret-key".to_owned())]
        );
        assert!(!request.normalized().contains("Authorization"));
    }

    #[test]
    fn a_jwt_becomes_a_bearer_header_that_never_prints() {
        let now = UNIX_EPOCH + Duration::from_hours(497_448);
        let auth = QWeatherAuth::resolve(&Credential::QWeatherJwt(credential()), now)
            .expect("the fixture key parses");
        let request = auth.apply(HttpRequest::get(
            "https://example.re.qweatherapi.com/current",
        ));

        let header = request
            .headers()
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.as_str())
            .expect("a bearer header");
        let token = header.strip_prefix("Bearer ").expect("the bearer scheme");
        assert_eq!(token.split('.').count(), 3, "header.payload.signature");
        assert!(
            !request
                .headers()
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("X-QW-Api-Key"))
        );

        let redacted = request.redacted_normalized();
        assert!(redacted.contains("authorization: Bearer ***"), "{redacted}");
        assert!(!redacted.contains(token), "the token leaked: {redacted}");
        assert!(
            !format!("{auth:?}").contains(token),
            "the Debug output leaked the token"
        );
    }
}
