// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `QWeather`'s JWT authentication: an Ed25519 (`EdDSA`) token minted in-process.
//!
//! The vendor documents the shape (read 2026-10-03,
//! <https://dev.qweather.com/en/docs/configuration/authentication/>): the header carries
//! `{"alg":"EdDSA","kid":"<credential id>"}`, the payload
//! `{"iss":"<developer id>","sub":"<project id>","iat":<unix>,"exp":<unix>}`, and the signature
//! covers the ASCII `header.payload` with the private key whose public half was registered in the
//! console. `exp − iat` must not exceed 24 h; this mints a 15-minute token per fetch, backdated
//! 30 s to absorb clock skew. `typ`, `aud` and `nbf` are reserved and must not be sent.
//!
//! Ed25519 signing is deterministic, so a pinned key and a frozen instant yield a byte-exact
//! token: `tests/qweather_jwt.rs` pins one against a vector computed independently, and any change
//! to the header, the claim set, the base64 alphabet or the signing input fails it.
//!
//! The private key is a PKCS#8 PEM document. `openssl genpkey -algorithm ED25519` writes the v1
//! form, which carries no public key and which `ring` accepts only through
//! `from_pkcs8_maybe_unchecked`; a v2 document is parsed with `from_pkcs8` first, so the key's own
//! consistency check still runs whenever the file carries the public half.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::signature::Ed25519KeyPair;
use serde::Serialize;

use crate::config::keys::JwtCredential;
use crate::error::{Error, Result};

/// How long a minted token stays valid: well inside the documented 24 h ceiling.
const LIFETIME_SECS: i64 = 900;

/// How far the `iat` claim is backdated, so a validator whose clock is a little behind still
/// accepts a token minted here.
const BACKDATE_SECS: i64 = 30;

/// The header segment: exactly `alg` and `kid`, in that order.
#[derive(Debug, Serialize)]
struct Header<'a> {
    /// The signature algorithm; the vendor accepts only `EdDSA`.
    alg: &'static str,
    /// The credential id the console issued.
    kid: &'a str,
}

/// The payload segment: exactly the four documented claims, in that order.
#[derive(Debug, Serialize)]
struct Claims<'a> {
    /// The developer id (`iss`).
    iss: &'a str,
    /// The project id (`sub`).
    sub: &'a str,
    /// When the token was minted, minus [`BACKDATE_SECS`].
    iat: i64,
    /// `iat + `[`LIFETIME_SECS`].
    exp: i64,
}

/// Mints a `QWeather` bearer token for `credential` at `now`.
///
/// The token is signed with the stored private key and never persisted: a caller mints one per
/// fetch, so a long-running consumer never presents a token older than its own request.
pub fn qweather_token(credential: &JwtCredential, now: SystemTime) -> Result<String> {
    let key = key_pair(credential)?;
    let iat = unix_seconds(now)? - BACKDATE_SECS;
    let header = segment(&Header {
        alg: "EdDSA",
        kid: credential.credential_id.trim(),
    })?;
    let claims = segment(&Claims {
        iss: credential.developer_id.trim(),
        sub: credential.project_id.trim(),
        iat,
        exp: iat + LIFETIME_SECS,
    })?;
    let signing_input = format!("{header}.{claims}");
    let signature = key.sign(signing_input.as_bytes());
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    ))
}

/// Checks that `credential` can mint a token: non-empty identifiers and a PKCS#8 Ed25519 PEM.
///
/// `key set --jwt` runs this before storing anything, so a mistyped identifier or the wrong key
/// file is refused at the console step that produced it instead of at the first fetch.
pub fn validate(credential: &JwtCredential) -> Result<()> {
    for (flag, value) in [
        ("--credential-id", &credential.credential_id),
        ("--developer-id", &credential.developer_id),
        ("--project-id", &credential.project_id),
    ] {
        if value.trim().is_empty() {
            return Err(Error::Usage(format!("{flag} must not be empty")));
        }
    }
    key_pair(credential).map(|_| ())
}

/// The Ed25519 key pair of a stored credential; a PEM that is not one is a configuration error.
fn key_pair(credential: &JwtCredential) -> Result<Ed25519KeyPair> {
    let der = pem_der(&credential.private_key)?;
    Ed25519KeyPair::from_pkcs8(&der)
        .or_else(|_| Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der))
        .map_err(|error| {
            Error::Config(format!(
                "the QWeather private key is not an Ed25519 PKCS#8 document ({error}); \
                 generate one with `openssl genpkey -algorithm ED25519`, upload its public half in \
                 the console, and store it again with `cirrocast key set qweather --jwt \
                 --key-file <PATH>`"
            ))
        })
}

/// One base64url (unpadded) JSON segment.
fn segment<T: Serialize>(value: &T) -> Result<String> {
    let json = serde_json::to_vec(value)
        .map_err(|error| Error::Other(format!("cannot encode a JWT segment: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

/// `now` as Unix seconds; a clock set before 1970 cannot mint a token.
fn unix_seconds(now: SystemTime) -> Result<i64> {
    let seconds = now.duration_since(UNIX_EPOCH).map_err(|_| {
        Error::Other("the system clock is set before 1970; cannot mint a JWT".to_owned())
    })?;
    i64::try_from(seconds.as_secs())
        .map_err(|_| Error::Other("the system clock is out of range for a JWT".to_owned()))
}

/// The DER bytes inside a PEM document.
///
/// Only the unencrypted PKCS#8 armour (`-----BEGIN PRIVATE KEY-----`) is accepted; the public-key,
/// certificate and encrypted-key armours are refused by name, so the message says what was supplied
/// instead of only that it did not parse. Text outside the armour is ignored, which is what lets a
/// comment live in the file.
fn pem_der(pem: &str) -> Result<Vec<u8>> {
    let mut body = String::new();
    let mut inside = false;
    let mut found = false;
    for line in pem.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("-----BEGIN ") {
            if inside {
                return Err(not_a_key("more than one PEM document"));
            }
            let label = rest.strip_suffix("-----").unwrap_or(rest);
            if label != "PRIVATE KEY" {
                return Err(not_a_key(&format!("a `{label}` PEM document")));
            }
            inside = true;
            found = true;
            continue;
        }
        if line.starts_with("-----END ") {
            inside = false;
            continue;
        }
        if inside {
            body.push_str(line);
        }
    }
    if !found {
        return Err(not_a_key("no PEM armour"));
    }
    STANDARD
        .decode(&body)
        .map_err(|_| not_a_key("a body that is not base64"))
}

/// The error for a PEM that cannot hold a `QWeather` signing key.
fn not_a_key(what: &str) -> Error {
    Error::Config(format!(
        "the QWeather private key is not an unencrypted PKCS#8 Ed25519 PEM document ({what}); \
         generate one with `openssl genpkey -algorithm ED25519` and upload its public half to the \
         console"
    ))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use base64::Engine as _;
    use ring::rand::SystemRandom;
    use ring::signature::Ed25519KeyPair;

    use super::{pem_der, qweather_token, validate};
    use crate::config::keys::JwtCredential;

    /// The throwaway key `tests/fixtures/qweather/ed25519-test-key.pem` holds (PKCS#8 v1, the form
    /// `openssl genpkey -algorithm ED25519` writes).
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

    /// The instant the unit tests mint at; the byte-exact vector lives in
    /// `tests/qweather_jwt.rs`, where it is pinned against an independently computed token.
    fn frozen_now() -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_hours(497_448)
    }

    #[test]
    fn a_second_pkcs8_document_parses_through_the_checked_path() {
        // `ring`'s own generator writes PKCS#8 v2 (with the public key), the form the checked
        // parser accepts; the fixture above is v1 and takes the unchecked fallback.
        let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .expect("the system random source works");
        let body = base64::engine::general_purpose::STANDARD.encode(document.as_ref());
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n");
        let credential = JwtCredential {
            private_key: pem,
            ..credential()
        };
        assert!(qweather_token(&credential, frozen_now()).is_ok());
    }

    #[test]
    fn a_public_key_or_garbage_is_refused_by_name() {
        let error = pem_der("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n")
            .expect_err("a public key cannot sign");
        assert!(error.to_string().contains("PUBLIC KEY"), "{error}");
        assert_eq!(error.exit_code(), 4);

        let error = pem_der("not a pem at all").expect_err("no armour");
        assert!(error.to_string().contains("no PEM armour"), "{error}");

        let error = pem_der("-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----\n")
            .expect_err("not base64");
        assert!(error.to_string().contains("not base64"), "{error}");

        // Valid base64, but the DER is truncated, so `ring` refuses it.
        let truncated = JwtCredential {
            private_key:
                "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGz9\n-----END PRIVATE KEY-----\n"
                    .to_owned(),
            ..credential()
        };
        let error = validate(&truncated).expect_err("a truncated key cannot sign");
        assert_eq!(error.exit_code(), 4);
    }

    #[test]
    fn an_empty_identifier_is_a_usage_error() {
        let credential = JwtCredential {
            project_id: "  ".to_owned(),
            ..credential()
        };
        let error = validate(&credential).expect_err("an empty identifier");
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("--project-id"), "{error}");
    }
}
