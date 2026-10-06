// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Step 27: `QWeather`'s JWT authentication.
//!
//! Every test here is offline. The byte-exact token vector is computed from a pinned throwaway key
//! (`fixtures/qweather/ed25519-test-key.pem`) and a frozen instant; its `exp` is in the past, so it
//! is a test vector and never a live credential. The mode matrix drives the real backend through
//! `StubTransport`; the exit codes, `key set --jwt` and `key list` run the real binary against a
//! throwaway XDG sandbox with the network guard on.

// The key material in this file is a generated test vector; the assertions are about it.
#![allow(clippy::similar_names)]

mod common;

use std::fs;
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{NaiveDate, TimeZone as _, Utc};
use cirrocast::auth::jwt::qweather_token;
use cirrocast::cache::{CacheKey, CacheMode};
use cirrocast::config::keys::JwtCredential;
use cirrocast::http::{HttpRequest, StubReply};
use cirrocast::provider::qweather::QWeather;
use common::{ProviderRun, Sandbox, fixture, fixture_location, fixture_reply};
use predicates::prelude::*;

/// The throwaway Ed25519 key the tests sign with (PKCS#8 v1, what `openssl genpkey` writes).
const TEST_PEM: &str = include_str!("fixtures/qweather/ed25519-test-key.pem");

/// The console identifiers the tests configure.
const CREDENTIAL_ID: &str = "ABCDE12345";
const DEVELOPER_ID: &str = "Q12345ABCD";
const PROJECT_ID: &str = "ABC2345DEF";

/// The account host the tests configure.
const HOST: &str = "https://example.re.qweatherapi.com";

/// The pinned vector: this key, these identifiers, minted at `2026-10-01T00:00:00Z`.
///
/// Computed independently (Python `cryptography`, cross-checked with `openssl pkeyutl -sign
/// -rawin`) from the same header and claims, so the test pins the wire format rather than our own
/// implementation of it. `exp` is `2026-10-01T00:14:30Z`, in the past.
const PINNED_TOKEN: &str = "eyJhbGciOiJFZERTQSIsImtpZCI6IkFCQ0RFMTIzNDUifQ.\
                            eyJpc3MiOiJRMTIzNDVBQkNEIiwic3ViIjoiQUJDMjM0NURFRiIsImlhdCI6MTc5MDgxMjc3MCwiZXhwIjoxNzkwODEzNjcwfQ.\
                            NPykyKKJ0Rj5Bnax3V3DT6oQEeBHRSvt-Drqbb4j7dKaxvgWLIEYRxSp3X4SiraIJ7kWwP2JweM9w1VrkMegDg";

/// The instant the pinned vector was minted at.
fn pinned_now() -> SystemTime {
    SystemTime::from(
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0)
            .single()
            .expect("a valid instant"),
    )
}

/// The instant [`common::provider_clock`] freezes the provider runs at: `2026-10-01T06:00:00Z`.
fn run_now() -> SystemTime {
    SystemTime::from(
        Utc.with_ymd_and_hms(2026, 10, 1, 6, 0, 0)
            .single()
            .expect("a valid instant"),
    )
}

fn credential() -> JwtCredential {
    JwtCredential {
        credential_id: CREDENTIAL_ID.to_owned(),
        developer_id: DEVELOPER_ID.to_owned(),
        project_id: PROJECT_ID.to_owned(),
        private_key: TEST_PEM.to_owned(),
    }
}

/// A run over the recorded responses with the host configured and a JWT credential stored.
fn jwt_run(replies: Vec<StubReply>, mode: CacheMode) -> ProviderRun {
    let mut config = cirrocast::config::Config::default();
    HOST.clone_into(&mut config.providers.qweather.host);
    let run = ProviderRun::with_config(replies, common::provider_clock(2026, 10, 1), mode, config);
    run.with_jwt("qweather", &credential());
    run
}

/// The two recorded responses one fetch consumes.
fn fixture_replies() -> Vec<StubReply> {
    vec![
        fixture_reply("qweather", "current.json"),
        fixture_reply("qweather", "hourly.json"),
    ]
}

/// A machine that exports any of the JWT quartet cannot exercise the file path: the environment
/// wins by design, and a test cannot clear it (edition 2024 makes `set_var` unsafe, and the crate
/// forbids unsafe).
fn jwt_env_unset() -> bool {
    [
        "CIRROCAST_QWEATHER_JWT_CREDENTIAL_ID",
        "CIRROCAST_QWEATHER_JWT_DEVELOPER_ID",
        "CIRROCAST_QWEATHER_JWT_PROJECT_ID",
        "CIRROCAST_QWEATHER_JWT_PRIVATE_KEY",
        "CIRROCAST_QWEATHER_KEY",
    ]
    .iter()
    .all(|name| std::env::var_os(name).is_none())
}

/// Decodes one base64url segment.
fn decode(segment: &str) -> String {
    String::from_utf8(URL_SAFE_NO_PAD.decode(segment).expect("base64url")).expect("UTF-8")
}

/// The bearer token of a recorded request, without the scheme.
fn bearer(request: &HttpRequest) -> String {
    request
        .headers()
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.clone())
        .expect("an Authorization header")
        .strip_prefix("Bearer ")
        .expect("the bearer scheme")
        .to_owned()
}

// ---------------------------------------------------------------------------------------------
// The token itself
// ---------------------------------------------------------------------------------------------

#[test]
fn the_pinned_token_matches_the_independent_vector_byte_for_byte() {
    let token = qweather_token(&credential(), pinned_now()).expect("the fixture key parses");
    assert_eq!(token, PINNED_TOKEN);
}

#[test]
fn the_header_and_payload_carry_exactly_the_documented_fields() {
    let token = qweather_token(&credential(), pinned_now()).expect("the fixture key parses");
    let segments: Vec<&str> = token.split('.').collect();
    assert_eq!(segments.len(), 3, "header.payload.signature");

    // The header is exactly `alg` and `kid` — `typ`, `aud` and `nbf` are reserved and absent.
    assert_eq!(decode(segments[0]), r#"{"alg":"EdDSA","kid":"ABCDE12345"}"#);
    assert_eq!(
        decode(segments[1]),
        r#"{"iss":"Q12345ABCD","sub":"ABC2345DEF","iat":1790812770,"exp":1790813670}"#
    );

    let claims: serde_json::Value =
        serde_json::from_str(&decode(segments[1])).expect("the payload is JSON");
    let iat = claims["iat"].as_i64().expect("iat is a number");
    let exp = claims["exp"].as_i64().expect("exp is a number");
    assert_eq!(exp - iat, 900, "the documented 15-minute lifetime");
    assert!(exp - iat <= 86_400, "the vendor's 24 h ceiling");
    assert_eq!(
        iat,
        i64::try_from(
            pinned_now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("after 1970")
                .as_secs()
        )
        .expect("fits")
            - 30,
        "the iat is backdated to absorb clock skew"
    );
    // The signature is one Ed25519 signature: 64 bytes, unpadded base64url.
    assert_eq!(segments[2].len(), 86, "{}", segments[2]);
}

// ---------------------------------------------------------------------------------------------
// The two modes, through the backend
// ---------------------------------------------------------------------------------------------

#[test]
fn jwt_mode_sends_a_bearer_token_and_never_the_api_key_header() {
    if !jwt_env_unset() {
        return;
    }
    let run = jwt_run(fixture_replies(), CacheMode::Normal);
    run.fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    let calls = run.calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert!(
            !call
                .headers()
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("X-QW-Api-Key")),
            "the API-key header must be absent in JWT mode"
        );
    }

    // One token per fetch, reused by both requests, minted at the run's frozen instant.
    let token = bearer(&calls[0]);
    assert_eq!(token, bearer(&calls[1]), "one mint serves the whole fetch");
    let segments: Vec<&str> = token.split('.').collect();
    assert_eq!(decode(segments[0]), r#"{"alg":"EdDSA","kid":"ABCDE12345"}"#);
    let expected_iat = i64::try_from(
        run_now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs(),
    )
    .expect("fits")
        - 30;
    assert_eq!(
        decode(segments[1]),
        format!(
            r#"{{"iss":"Q12345ABCD","sub":"ABC2345DEF","iat":{expected_iat},"exp":{}}}"#,
            expected_iat + 900
        )
    );

    // The token is registered as a secret: the redacted spelling of the request keeps it out of
    // every log line, error message and cache envelope.
    let redacted = calls[0].redacted_normalized();
    assert!(redacted.contains("authorization: Bearer ***"), "{redacted}");
    assert!(!redacted.contains(&token), "the token leaked: {redacted}");
}

#[test]
fn api_key_mode_still_sends_the_key_header_and_no_bearer() {
    if !jwt_env_unset() {
        return;
    }
    let mut config = cirrocast::config::Config::default();
    HOST.clone_into(&mut config.providers.qweather.host);
    let run = ProviderRun::with_config(
        fixture_replies(),
        common::provider_clock(2026, 10, 1),
        CacheMode::Normal,
        config,
    );
    run.with_key("qweather", "test-key-0123456789abcdef");
    run.fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    let calls = run.calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert!(
            call.headers()
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("X-QW-Api-Key")
                    && value == "test-key-0123456789abcdef"),
            "{:?}",
            call.headers()
        );
        assert!(
            !call
                .headers()
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("authorization")),
            "no bearer without a JWT credential"
        );
    }
}

#[test]
fn a_rejected_credential_names_both_remedies() {
    if !jwt_env_unset() {
        return;
    }
    let run = jwt_run(
        vec![StubReply::status(
            401,
            Vec::new(),
            fixture("qweather/error_401.json"),
        )],
        CacheMode::Normal,
    );
    let error = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect_err("a 401 is a rejected credential");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("cirrocast key set qweather"), "{text}");
    assert!(text.contains("cirrocast key set qweather --jwt"), "{text}");
    assert!(text.contains("JWT Validation"), "{text}");
    assert!(!text.contains(TEST_PEM.trim()), "the PEM leaked: {text}");
}

#[test]
fn the_verbose_note_names_the_mode_and_never_the_token() {
    if !jwt_env_unset() {
        return;
    }
    let run = jwt_run(fixture_replies(), CacheMode::Normal).with_verbose(1);
    let report = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");
    let raw = report
        .attribution
        .raw
        .as_deref()
        .expect("-v keeps the provider's own note");
    assert!(raw.contains("auth: jwt (kid ABCDE12345)"), "{raw}");

    let token = bearer(&run.calls()[0]);
    assert!(!raw.contains(&token), "the token leaked into -v: {raw}");
    assert!(
        !raw.contains("PRIVATE KEY"),
        "the PEM leaked into -v: {raw}"
    );

    // The API-key mode says so too.
    let mut config = cirrocast::config::Config::default();
    HOST.clone_into(&mut config.providers.qweather.host);
    let run = ProviderRun::with_config(
        fixture_replies(),
        common::provider_clock(2026, 10, 1),
        CacheMode::Normal,
        config,
    )
    .with_verbose(1);
    run.with_key("qweather", "test-key-0123456789abcdef");
    let report = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");
    let raw = report
        .attribution
        .raw
        .as_deref()
        .expect("-v keeps the provider's own note");
    assert!(raw.contains("auth: api key"), "{raw}");
}

#[test]
fn a_hand_edited_bad_pem_is_a_configuration_error_at_mint_time() {
    if !jwt_env_unset() {
        return;
    }
    // `set_jwt` stores what it is given (the CLI validates first); a file edited by hand is
    // re-validated when the token is minted.
    let mut config = cirrocast::config::Config::default();
    HOST.clone_into(&mut config.providers.qweather.host);
    let run = ProviderRun::with_config(
        fixture_replies(),
        common::provider_clock(2026, 10, 1),
        CacheMode::Normal,
        config,
    );
    run.with_jwt(
        "qweather",
        &JwtCredential {
            private_key: "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n"
                .to_owned(),
            ..credential()
        },
    );
    let error = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect_err("a truncated key cannot sign");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("PKCS#8"), "{error}");
    assert_eq!(
        run.calls(),
        Vec::<HttpRequest>::new(),
        "nothing may be sent with a credential that cannot sign"
    );
}

#[test]
fn two_fetches_at_different_instants_share_the_cache_key_and_differ_in_token() {
    if !jwt_env_unset() {
        return;
    }
    // Refresh mode forces both fetches upstream, so both mints are observable.
    let run = jwt_run(
        [fixture_replies(), fixture_replies()].concat(),
        CacheMode::Refresh,
    );
    run.fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");
    run.clock().advance(Duration::from_secs(60));
    run.fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the second fetch parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 4, "two fetches, two requests each");
    let first = bearer(&calls[0]);
    let second = bearer(&calls[2]);
    assert_ne!(first, second, "a minute later is a different token");

    // The cache key holds the provider, the place, the day count and the date — never the token,
    // so both fetches wrote the same two entries instead of four.
    let date = NaiveDate::from_ymd_opt(2026, 10, 1).expect("a valid date");
    let current = CacheKey::weather_part("qweather", "current", 39.9042, 116.4074, 3, date);
    let hourly = CacheKey::weather_part("qweather", "hourly", 39.9042, 116.4074, 3, date);
    for key in [&current, &hourly] {
        assert!(
            run.cache().entry_path(key).exists(),
            "{} must exist",
            run.cache().entry_path(key).display()
        );
    }
    let entries: Vec<String> = fs::read_dir(run.cache().root().join("weather"))
        .expect("the weather namespace exists")
        .map(|entry| {
            entry
                .expect("a readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(entries.len(), 2, "one entry per resource: {entries:?}");

    // No token byte reaches the disk.
    let stored =
        fs::read_to_string(run.cache().entry_path(&current)).expect("the entry is readable");
    assert!(!stored.contains(&first), "the token leaked into the cache");
    assert!(
        !stored.contains("Bearer"),
        "the header leaked into the cache"
    );
}

// ---------------------------------------------------------------------------------------------
// The command line: `key set --jwt`, `key list`, exit codes
// ---------------------------------------------------------------------------------------------

/// A sandbox whose configuration names the account host, so a run reaches the credential.
fn sandbox_with_host() -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.write_config(&format!("[providers.qweather]\nhost = \"{HOST}\"\n"));
    sandbox
}

#[test]
fn key_set_jwt_stores_the_pem_and_key_list_prints_identifiers_only() {
    let sandbox = Sandbox::new();
    let key_path = sandbox.home().join("ed25519-private.pem");
    fs::write(&key_path, TEST_PEM).expect("the key file is written");

    sandbox
        .cirrocast()
        .args([
            "key",
            "set",
            "qweather",
            "--jwt",
            "--key-file",
            &key_path.to_string_lossy(),
            "--credential-id",
            CREDENTIAL_ID,
            "--developer-id",
            DEVELOPER_ID,
            "--project-id",
            PROJECT_ID,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("stored qweather JWT credential"));

    let stored = fs::read_to_string(sandbox.keys_file()).expect("keys.toml is written");
    assert!(stored.contains("[jwt.qweather]"), "{stored}");
    assert!(
        stored.contains("credential_id = \"ABCDE12345\""),
        "{stored}"
    );
    assert!(stored.contains("BEGIN PRIVATE KEY"), "{stored}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(sandbox.keys_file())
            .expect("the file exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the credential file stays owner-only");
    }

    let assert = sandbox.cirrocast().args(["key", "list"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(
        stdout.contains("qweather  jwt (kid ABCDE12345, iss Q12345ABCD, sub ABC2345DEF)"),
        "{stdout}"
    );
    assert!(stdout.contains("(file)"), "{stdout}");
    assert!(!stdout.contains("PRIVATE KEY"), "the PEM leaked: {stdout}");
    assert!(!stdout.contains("MC4CAQAw"), "the PEM leaked: {stdout}");

    // With an API key stored beside it, the row names both forms.
    sandbox
        .cirrocast()
        .args(["key", "set", "qweather"])
        .write_stdin("sk-test-abcdef123456\n")
        .assert()
        .success();
    let assert = sandbox.cirrocast().args(["key", "list"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(
        stdout.contains("jwt (kid ABCDE12345, iss Q12345ABCD, sub ABC2345DEF), api key sk-t…56"),
        "{stdout}"
    );

    // One command removes both forms, and nothing is left behind.
    sandbox
        .cirrocast()
        .args(["key", "rm", "qweather"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "removed qweather JWT credential and API key",
        ));
    let assert = sandbox.cirrocast().args(["key", "list"]).assert().success();
    assert_eq!(
        assert.get_output().stdout,
        Vec::<u8>::new(),
        "no credential is configured any more"
    );
    common::assert_no_temporary_files(&sandbox.config_dir());
}

#[test]
fn key_set_jwt_reads_the_pem_from_stdin_with_a_dash() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args([
            "key",
            "set",
            "qweather",
            "--jwt",
            "--key-file",
            "-",
            "--credential-id",
            CREDENTIAL_ID,
            "--developer-id",
            DEVELOPER_ID,
            "--project-id",
            PROJECT_ID,
        ])
        .write_stdin(TEST_PEM)
        .assert()
        .success();

    let stored = fs::read_to_string(sandbox.keys_file()).expect("keys.toml is written");
    assert!(stored.contains("[jwt.qweather]"), "{stored}");
}

#[test]
fn key_set_jwt_refuses_a_pem_that_cannot_sign() {
    let sandbox = Sandbox::new();
    let truncated = sandbox.home().join("truncated.pem");
    fs::write(
        &truncated,
        "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGz9\n-----END PRIVATE KEY-----\n",
    )
    .expect("the key file is written");
    let public = sandbox.home().join("public.pem");
    fs::write(
        &public,
        "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAmk6IK9/VslfDxjQQSqM3wakazLkAUxghsRDyMS3T0K0=\n-----END PUBLIC KEY-----\n",
    )
    .expect("the key file is written");

    for (path, needle) in [(&truncated, "PKCS#8"), (&public, "PUBLIC KEY")] {
        sandbox
            .cirrocast()
            .args([
                "key",
                "set",
                "qweather",
                "--jwt",
                "--key-file",
                &path.to_string_lossy(),
                "--credential-id",
                CREDENTIAL_ID,
                "--developer-id",
                DEVELOPER_ID,
                "--project-id",
                PROJECT_ID,
            ])
            .assert()
            .code(4)
            .stderr(predicate::str::contains(needle));
    }
    assert!(!sandbox.keys_file().exists(), "nothing was written");
}

#[test]
fn key_set_jwt_is_refused_where_there_is_no_jwt_mode() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args([
            "key",
            "set",
            "smhi",
            "--jwt",
            "--key-file",
            "-",
            "--credential-id",
            CREDENTIAL_ID,
            "--developer-id",
            DEVELOPER_ID,
            "--project-id",
            PROJECT_ID,
        ])
        .write_stdin(TEST_PEM)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("has no JWT mode"));

    // A flag combination clap can judge without running anything.
    sandbox
        .cirrocast()
        .args(["key", "set", "qweather", "--jwt", "--key-file", "-"])
        .assert()
        .code(2);
}

#[test]
fn a_partial_env_quartet_is_exit_4_naming_the_missing_variables() {
    let sandbox = sandbox_with_host();
    sandbox
        .cirrocast()
        .env("CIRROCAST_QWEATHER_JWT_CREDENTIAL_ID", CREDENTIAL_ID)
        .env("CIRROCAST_QWEATHER_JWT_PRIVATE_KEY", TEST_PEM)
        .args(["-p", "qweather", "Beijing"])
        .assert()
        .code(4)
        .stderr(
            predicate::str::contains("CIRROCAST_QWEATHER_JWT_DEVELOPER_ID")
                .and(predicate::str::contains(
                    "CIRROCAST_QWEATHER_JWT_PROJECT_ID",
                ))
                .and(predicate::str::contains("never falls back to the API key")),
        );
}

#[test]
fn a_missing_credential_is_exit_6_with_both_key_set_forms() {
    let sandbox = sandbox_with_host();
    sandbox
        .cirrocast()
        .args(["-p", "qweather", "Beijing"])
        .assert()
        .code(6)
        .stderr(
            predicate::str::contains("missing credential for qweather")
                .and(predicate::str::contains(
                    "cirrocast key set qweather` (API key)",
                ))
                .and(predicate::str::contains(
                    "cirrocast key set qweather --jwt` (JWT)",
                ))
                .and(predicate::str::contains("CIRROCAST_QWEATHER_KEY")),
        );
}

#[test]
fn provider_info_names_both_modes() {
    let sandbox = Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .args(["provider", "info", "qweather"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(
        stdout.contains("auth:        API key or JWT (Ed25519)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("store jwt:   cirrocast key set qweather --jwt"),
        "{stdout}"
    );
}
