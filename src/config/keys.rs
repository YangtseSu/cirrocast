// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The BYOK API key store.
//!
//! Keys never enter `config.toml`: that document is `0644`, meant to be pasted into bug reports,
//! dumped wholesale by `cirrocast config show` and edited by hand, so a secret there would leak
//! through all four. Instead they live in `$XDG_CONFIG_HOME/cirrocast/keys.toml`, a `0600` document
//! with one `[keys]` table:
//!
//! ```toml
//! [keys]
//! openweathermap = "…"
//! ```
//!
//! Lookup order, first hit wins: `CIRROCAST_<PROVIDER>_KEY`, then `keys.toml`. There is no third
//! tier: OS keyring storage is out of scope for v1 (step 10's `## Out of scope` lists it), so a
//! key that is in neither place is reported as missing. Any key file that group or other can read
//! is refused rather than used, so a stray `chmod 644` is loud instead of silent.
//!
//! Step 25's `GeoNames` geocoder has an account *name* rather than a provider key, so the same two
//! tiers serve it through [`NAMED_CREDENTIALS`]: `CIRROCAST_GEONAMES_USER` first, then a
//! `[keys] geonames` entry, written by the same `key set` path (stdin only) and shown masked by
//! the same `key list`.
//!
//! A provider with a second authentication mode stores a `[jwt.<provider>]` table beside `[keys]`
//! (step 27, `QWeather`):
//!
//! ```toml
//! [jwt.qweather]
//! credential_id = "ABCDE12345"
//! developer_id = "Q12345ABCD"
//! project_id = "ABC2345DEF"
//! private_key = """
//! -----BEGIN PRIVATE KEY-----
//! …
//! -----END PRIVATE KEY-----
//! """
//! ```
//!
//! [`KeyStore::credential`] resolves such a provider through four tiers, first complete set wins:
//! the `CIRROCAST_<PROVIDER>_JWT_*` quartet, the `[jwt.<provider>]` table, `CIRROCAST_<PROVIDER>_KEY`,
//! then `[keys]`. A *partial* JWT set — some but not all of the quartet, or a table missing a field
//! — is [`Error::Config`] naming what is missing, never a silent fall-through to the other mode. The
//! private key is the secret; the three identifiers are not, which is why `key list` prints them.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::atomic_write;
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::provider::{JwtEnv, ProviderId};

/// Mode of `keys.toml`: owner read/write only.
const KEYS_FILE_MODE: u32 = 0o600;

/// The on-disk shape of `keys.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct KeyFile {
    /// The one table of the document, keyed by canonical provider id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    keys: BTreeMap<String, String>,
    /// The JWT credentials, keyed by canonical provider id (step 27).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    jwt: BTreeMap<String, JwtTable>,
}

/// One `[jwt.<provider>]` table: the three console identifiers plus the private key.
///
/// Every field is an `Option` so that a table written by hand with a key missing is *detected* and
/// named in the error, instead of deserialising into an empty string that would later sign an
/// invalid token.
#[derive(Debug, Default, Serialize, Deserialize)]
struct JwtTable {
    /// The credential id the console issued (`kid`).
    #[serde(default)]
    credential_id: Option<String>,
    /// The developer id (`iss`).
    #[serde(default)]
    developer_id: Option<String>,
    /// The project id (`sub`).
    #[serde(default)]
    project_id: Option<String>,
    /// The PKCS#8 Ed25519 private key, as PEM text.
    #[serde(default)]
    private_key: Option<String>,
}

impl JwtTable {
    /// The fields that are absent or empty, in table order.
    fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        for (name, value) in [
            ("credential_id", &self.credential_id),
            ("developer_id", &self.developer_id),
            ("project_id", &self.project_id),
            ("private_key", &self.private_key),
        ] {
            if value.as_deref().is_none_or(|value| value.trim().is_empty()) {
                missing.push(name);
            }
        }
        missing
    }

    /// The credential this table holds, `None` while a field is missing.
    fn credential(&self) -> Option<JwtCredential> {
        Some(JwtCredential {
            credential_id: self.credential_id.as_deref()?.trim().to_owned(),
            developer_id: self.developer_id.as_deref()?.trim().to_owned(),
            project_id: self.project_id.as_deref()?.trim().to_owned(),
            private_key: self.private_key.as_deref()?.trim().to_owned(),
        })
    }

    /// The table for `credential`, in the field order `keys.toml` is written in.
    fn of(credential: &JwtCredential) -> Self {
        Self {
            credential_id: Some(credential.credential_id.clone()),
            developer_id: Some(credential.developer_id.clone()),
            project_id: Some(credential.project_id.clone()),
            private_key: Some(credential.private_key.clone()),
        }
    }
}

/// A credential a request can present.
///
/// `QWeather` accepts two forms (step 27); a provider with one form is always [`Credential::ApiKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// The API key, sent in the provider's documented header or query parameter.
    ApiKey(String),
    /// A `QWeather` JWT credential, signed into an `Authorization: Bearer` token per fetch.
    QWeatherJwt(JwtCredential),
}

/// A `QWeather` JWT credential: the three console identifiers and the PEM private key.
#[derive(Clone, PartialEq, Eq)]
pub struct JwtCredential {
    /// The credential id the console issued (`kid`).
    pub credential_id: String,
    /// The developer id (`iss`), ten characters starting with `Q`.
    pub developer_id: String,
    /// The project id (`sub`).
    pub project_id: String,
    /// The PKCS#8 Ed25519 private key, as PEM text; its public half is registered in the console.
    pub private_key: String,
}

impl JwtCredential {
    /// The non-secret identifiers, for `key list`.
    #[must_use]
    pub fn ids(&self) -> JwtIds {
        JwtIds {
            credential_id: self.credential_id.clone(),
            developer_id: self.developer_id.clone(),
            project_id: self.project_id.clone(),
        }
    }
}

impl std::fmt::Debug for JwtCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The private key is a secret; only the identifiers may be printed.
        formatter
            .debug_struct("JwtCredential")
            .field("credential_id", &self.credential_id)
            .field("developer_id", &self.developer_id)
            .field("project_id", &self.project_id)
            .field("private_key", &"***")
            .finish()
    }
}

/// The non-secret identifiers of a JWT credential, as `key list` prints them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtIds {
    /// The credential id the console issued (`kid`).
    pub credential_id: String,
    /// The developer id (`iss`).
    pub developer_id: String,
    /// The project id (`sub`).
    pub project_id: String,
}

/// Where a configured key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// The `CIRROCAST_<PROVIDER>_KEY` environment variable.
    Env,
    /// `keys.toml`.
    File,
}

/// A credential that belongs to a *service* rather than to a forecast provider.
///
/// Step 25's `GeoNames` geocoder is BYOK: the search endpoint refuses to answer without an account
/// name, and that name is stored exactly like a provider key — `CIRROCAST_GEONAMES_USER` first,
/// then the `[keys]` table of the `0600` file — so `key set geonames` (read from stdin, never
/// argv) writes it and `key list` shows it masked. Nothing else about the store changes: a name
/// that is neither a provider id nor one of these is still a usage error.
pub struct NamedCredential {
    /// The canonical name, which is also the `[keys]` entry and the `key set` argument.
    pub name: &'static str,
    /// The environment variable that can supply the value instead.
    pub env: &'static str,
    /// What the value is, for the `key set` prompt.
    pub what: &'static str,
}

/// The non-provider credentials this build knows, in listing order.
pub const NAMED_CREDENTIALS: &[NamedCredential] = &[NamedCredential {
    name: "geonames",
    env: "CIRROCAST_GEONAMES_USER",
    what: "GeoNames user name",
}];

/// One stored credential form, as `key list` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyForm {
    /// An API key: the masked value, never the key itself.
    ApiKey {
        /// The masked secret — never the value itself.
        masked: String,
        /// Where the key was found.
        source: KeySource,
    },
    /// A JWT credential: the identifiers only, never the PEM.
    QWeatherJwt {
        /// The console identifiers.
        ids: JwtIds,
        /// Where the credential was found.
        source: KeySource,
    },
}

impl KeyForm {
    /// Where this form was found.
    #[must_use]
    pub const fn source(&self) -> KeySource {
        match self {
            Self::ApiKey { source, .. } | Self::QWeatherJwt { source, .. } => *source,
        }
    }
}

/// One row of `cirrocast key list`: a provider and every form it has stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySummary {
    /// Canonical provider id, `id.as_str()`.
    pub provider: String,
    /// The stored forms, JWT first when both exist: it is what a request would present.
    pub forms: Vec<KeyForm>,
}

/// What a `key rm` removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Removed {
    /// The API-key form was stored and is now gone.
    pub api_key: bool,
    /// The JWT form was stored and is now gone.
    pub jwt: bool,
}

impl Removed {
    /// Whether anything was removed at all.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.api_key || self.jwt
    }
}

/// The key store behind `cirrocast key …`.
#[derive(Debug, Clone)]
pub struct KeyStore {
    /// `$XDG_CONFIG_HOME/cirrocast/keys.toml`.
    path: PathBuf,
}

impl KeyStore {
    /// Opens the store at the resolved key file path.
    pub fn new(paths: &Paths) -> Self {
        Self {
            path: paths.keys_file.clone(),
        }
    }

    /// The document this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The environment variable that can supply `provider`'s key, if the provider uses keys.
    ///
    /// An unknown provider id is a usage error here, exactly as it is on the command line.
    pub fn env_var(provider: &str) -> Result<Option<&'static str>> {
        let id: ProviderId = provider.parse()?;
        Ok(id.metadata().key_env)
    }

    /// The named credential `name` refers to, case-insensitively; `None` when there is no such
    /// credential.
    #[must_use]
    pub fn named(name: &str) -> Option<&'static NamedCredential> {
        let name = name.trim();
        NAMED_CREDENTIALS
            .iter()
            .find(|credential| credential.name.eq_ignore_ascii_case(name))
    }

    /// The canonical spelling of `name`, whether it is a provider id or a named credential.
    ///
    /// This is what `key set`/`key rm` accept, so the two spellings of one provider (`open-weather-map`,
    /// `openweathermap`) cannot create two entries and a credential name is stored under its
    /// canonical form.
    pub fn canonical(name: &str) -> Result<String> {
        if let Some(credential) = Self::named(name) {
            return Ok(credential.name.to_owned());
        }
        let id: ProviderId = name.parse()?;
        Ok(id.as_str().to_owned())
    }

    /// Looks a credential up: environment first, then the key file.
    ///
    /// `name` is a provider id or one of [`NAMED_CREDENTIALS`].
    pub fn get(&self, name: &str) -> Result<Option<String>> {
        if let Some(credential) = Self::named(name) {
            if let Some(value) = super::env_value(credential.env) {
                return Ok(Some(value));
            }
            return Ok(named_entry(&self.read()?.keys, credential).cloned());
        }
        let id: ProviderId = name.parse()?;
        if let Some(env) = id.metadata().key_env
            && let Some(value) = super::env_value(env)
        {
            return Ok(Some(value));
        }
        Ok(entry(&self.read()?.keys, id).cloned())
    }

    /// The credential to present for `id`, in the order a request tries the forms.
    ///
    /// A provider with a JWT mode (step 27) resolves through four tiers, first complete set wins:
    /// the `CIRROCAST_<ID>_JWT_*` quartet, the `[jwt.<id>]` table, the API-key environment
    /// variable, then the `[keys]` entry. A partial JWT set is [`Error::Config`] naming what is
    /// missing, never a silent fall-through to the API key; a provider without a JWT mode has the
    /// two API-key tiers of [`KeyStore::get`].
    pub fn credential(&self, id: ProviderId) -> Result<Option<Credential>> {
        if let Some(env) = id.jwt_env() {
            if let Some(credential) = jwt_from_env(id, &env)? {
                return Ok(Some(Credential::QWeatherJwt(credential)));
            }
            let file = self.read()?;
            if let Some((credential, _)) = self.jwt_from_file(id, &file)? {
                return Ok(Some(Credential::QWeatherJwt(credential)));
            }
            return Ok(api_key(id, &file).map(|(value, _)| Credential::ApiKey(value)));
        }
        Ok(self.get(id.as_str())?.map(Credential::ApiKey))
    }

    /// Stores (or replaces) the JWT credential of a provider that accepts one.
    ///
    /// The caller validates the credential first (`auth::jwt::validate`); a file edited by hand is
    /// re-validated when a token is minted. Storing a JWT leaves any API key in place —
    /// [`KeyStore::credential`] prefers the JWT, and [`KeyStore::remove`] drops both forms.
    pub fn set_jwt(&self, provider: &str, credential: &JwtCredential) -> Result<()> {
        let id: ProviderId = provider.parse()?;
        if id.jwt_env().is_none() {
            return Err(Error::Usage(format!(
                "provider `{id}` has no JWT mode; `cirrocast key set {id}` stores an API key"
            )));
        }
        let mut file = self.read()?;
        // Replace any spelling of the same provider so the file cannot hold two credentials.
        file.jwt
            .retain(|name, _| name.parse::<ProviderId>().ok() != Some(id));
        file.jwt
            .insert(id.as_str().to_owned(), JwtTable::of(credential));
        self.write(&file)
    }

    /// The JWT credential the `[jwt.<id>]` table holds, and its source; `None` when there is no
    /// table for `id`.
    fn jwt_from_file(
        &self,
        id: ProviderId,
        file: &KeyFile,
    ) -> Result<Option<(JwtCredential, KeySource)>> {
        let Some(table) = jwt_entry(&file.jwt, id) else {
            return Ok(None);
        };
        let missing = table.missing();
        if !missing.is_empty() {
            return Err(self.incomplete_jwt(id, &missing));
        }
        Ok(table
            .credential()
            .map(|credential| (credential, KeySource::File)))
    }

    /// The error for a `[jwt.<id>]` table that is missing fields.
    fn incomplete_jwt(&self, id: ProviderId, missing: &[&str]) -> Error {
        Error::Config(format!(
            "[jwt.{id}] in {} is missing {}; every JWT field is required, and a partial credential \
             never falls back to the API key",
            self.path.display(),
            missing.join(", ")
        ))
    }

    /// Stores (or replaces) `name`'s credential in the key file.
    ///
    /// `name` is a provider id or one of [`NAMED_CREDENTIALS`]; a keyless provider is refused.
    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        if let Some(credential) = Self::named(name) {
            return self.set_named(credential, value);
        }
        let id: ProviderId = name.parse()?;
        if id.metadata().key_env.is_none() {
            return Err(Error::Usage(format!(
                "provider `{id}` does not use an API key"
            )));
        }
        let value = checked_value(value)?;

        let mut file = self.read()?;
        // Replace any spelling of the same provider so the file cannot hold two keys for one id.
        file.keys
            .retain(|name, _| name.parse::<ProviderId>().ok() != Some(id));
        file.keys.insert(id.as_str().to_owned(), value.to_owned());
        self.write(&file)
    }

    /// Removes every stored credential of `name`, reporting which forms were there.
    ///
    /// Both forms of a provider with a JWT mode go together: `key rm qweather` is the one command
    /// that removes a credential, and leaving a JWT behind because the user once stored an API key
    /// would keep a secret on disk they asked to be rid of.
    pub fn remove(&self, name: &str) -> Result<Removed> {
        if let Some(credential) = Self::named(name) {
            return self.remove_named(credential);
        }
        let id: ProviderId = name.parse()?;
        let mut file = self.read()?;
        let keys_before = file.keys.len();
        file.keys
            .retain(|name, _| name.parse::<ProviderId>().ok() != Some(id));
        let jwt_before = file.jwt.len();
        file.jwt
            .retain(|name, _| name.parse::<ProviderId>().ok() != Some(id));
        let removed = Removed {
            api_key: file.keys.len() != keys_before,
            jwt: file.jwt.len() != jwt_before,
        };
        if removed.any() {
            self.write(&file)?;
        }
        Ok(removed)
    }

    /// The named-credential arm of [`KeyStore::set`].
    fn set_named(&self, credential: &NamedCredential, value: &str) -> Result<()> {
        let value = checked_value(value)?;
        let mut file = self.read()?;
        // Any casing of the same credential is one entry, exactly as provider spellings are.
        file.keys
            .retain(|name, _| !name.trim().eq_ignore_ascii_case(credential.name));
        file.keys
            .insert(credential.name.to_owned(), value.to_owned());
        self.write(&file)
    }

    /// The named-credential arm of [`KeyStore::remove`].
    fn remove_named(&self, credential: &NamedCredential) -> Result<Removed> {
        let mut file = self.read()?;
        let before = file.keys.len();
        file.keys
            .retain(|name, _| !name.trim().eq_ignore_ascii_case(credential.name));
        let removed = Removed {
            api_key: file.keys.len() != before,
            jwt: false,
        };
        if removed.any() {
            self.write(&file)?;
        }
        Ok(removed)
    }

    /// Every configured credential, masked, in registry order with hand-written entries last.
    ///
    /// A provider with a JWT mode lists that form first — it is what a request would present — and
    /// the API key beside it when both are stored. A partial JWT set is the same
    /// [`Error::Config`] a fetch reports, so a broken credential is never hidden by a listing.
    pub fn list(&self) -> Result<Vec<KeySummary>> {
        let file = self.read()?;
        let mut summaries = Vec::new();
        for id in ProviderId::all() {
            let mut forms = Vec::new();
            if let Some(env) = id.jwt_env() {
                if let Some(credential) = jwt_from_env(id, &env)? {
                    forms.push(KeyForm::QWeatherJwt {
                        ids: credential.ids(),
                        source: KeySource::Env,
                    });
                } else if let Some((credential, source)) = self.jwt_from_file(id, &file)? {
                    forms.push(KeyForm::QWeatherJwt {
                        ids: credential.ids(),
                        source,
                    });
                }
            }
            if let Some((value, source)) = api_key(id, &file) {
                forms.push(KeyForm::ApiKey {
                    masked: Self::mask(&value),
                    source,
                });
            }
            if !forms.is_empty() {
                summaries.push(KeySummary {
                    provider: id.as_str().to_owned(),
                    forms,
                });
            }
        }
        for credential in NAMED_CREDENTIALS {
            let found = super::env_value(credential.env)
                .map(|value| (value, KeySource::Env))
                .or_else(|| {
                    named_entry(&file.keys, credential)
                        .cloned()
                        .map(|value| (value, KeySource::File))
                });
            if let Some((value, source)) = found {
                summaries.push(KeySummary {
                    provider: credential.name.to_owned(),
                    forms: vec![KeyForm::ApiKey {
                        masked: Self::mask(&value),
                        source,
                    }],
                });
            }
        }
        for (name, value) in &file.keys {
            if name.parse::<ProviderId>().is_err() && Self::named(name).is_none() {
                summaries.push(KeySummary {
                    provider: name.clone(),
                    forms: vec![KeyForm::ApiKey {
                        masked: Self::mask(value),
                        source: KeySource::File,
                    }],
                });
            }
        }
        Ok(summaries)
    }

    /// `abcd…yz`: the first four and last two characters, enough to tell two stored keys apart and
    /// useless to anyone reading over a shoulder. Anything shorter than eight characters is not
    /// recognisable at all and prints as `…`.
    pub fn mask(value: &str) -> String {
        let characters: Vec<char> = value.chars().collect();
        if characters.len() < 8 {
            return "…".to_owned();
        }
        let head: String = characters[..4].iter().collect();
        let tail: String = characters[characters.len() - 2..].iter().collect();
        format!("{head}…{tail}")
    }

    /// Reads the key file, refusing one that group or other can read.
    ///
    /// A missing file is an empty store, not an error; a *dangling symlink* is not missing — the
    /// path exists but points nowhere — so it is reported as the broken configuration it is
    /// instead of being mistaken for an absent file.
    fn read(&self) -> Result<KeyFile> {
        match fs::metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(&self.path).is_ok() {
                    return Err(Error::Config(format!(
                        "{} is a dangling symbolic link; remove it or point it at a file",
                        self.path.display()
                    )));
                }
                return Ok(KeyFile::default());
            }
            Err(error) => {
                return Err(Error::Config(format!("{}: {error}", self.path.display())));
            }
        }
        check_mode(&self.path)?;
        let text = fs::read_to_string(&self.path)
            .map_err(|error| Error::Config(format!("{}: {error}", self.path.display())))?;
        toml::from_str(&text)
            .map_err(|error| Error::Config(format!("{}: {error}", self.path.display())))
    }

    /// Writes the key file atomically with mode `0600`.
    fn write(&self, file: &KeyFile) -> Result<()> {
        let document = toml::to_string_pretty(file).map_err(|error| {
            Error::Config(format!("cannot serialise the API key store: {error}"))
        })?;
        atomic_write(&self.path, document.as_bytes(), KEYS_FILE_MODE)
    }
}

/// The entry for `id`, accepting any spelling of the provider id a user may have written.
fn entry(keys: &BTreeMap<String, String>, id: ProviderId) -> Option<&String> {
    keys.iter()
        .find(|(name, _)| name.parse::<ProviderId>().ok() == Some(id))
        .map(|(_, value)| value)
}

/// The entry for a named credential, accepting any casing the user may have written.
fn named_entry<'a>(
    keys: &'a BTreeMap<String, String>,
    credential: &NamedCredential,
) -> Option<&'a String> {
    keys.iter()
        .find(|(name, _)| name.trim().eq_ignore_ascii_case(credential.name))
        .map(|(_, value)| value)
}

/// The `[jwt.<id>]` table, accepting any spelling of the provider id a user may have written.
fn jwt_entry(tables: &BTreeMap<String, JwtTable>, id: ProviderId) -> Option<&JwtTable> {
    tables
        .iter()
        .find(|(name, _)| name.parse::<ProviderId>().ok() == Some(id))
        .map(|(_, table)| table)
}

/// The API key the environment or `file` supplies for `id`, and where it came from.
fn api_key(id: ProviderId, file: &KeyFile) -> Option<(String, KeySource)> {
    if let Some(variable) = id.metadata().key_env
        && let Some(value) = super::env_value(variable)
    {
        return Some((value, KeySource::Env));
    }
    entry(&file.keys, id)
        .cloned()
        .map(|value| (value, KeySource::File))
}

/// The JWT credential the environment supplies for `id`, `None` when none of the quartet is set.
///
/// A quartet that is present but incomplete is [`Error::Config`] naming the missing variables: a
/// half-exported credential must not fall through to the API key, which is exactly the confusion
/// the second mode is there to avoid.
fn jwt_from_env(id: ProviderId, env: &JwtEnv) -> Result<Option<JwtCredential>> {
    let mut values = BTreeMap::new();
    let mut missing = Vec::new();
    for (field, variable) in env.fields() {
        match super::env_value(variable) {
            Some(value) => {
                values.insert(field, value);
            }
            None => missing.push(variable),
        }
    }
    if values.is_empty() {
        return Ok(None);
    }
    if !missing.is_empty() {
        return Err(Error::Config(format!(
            "the JWT environment for `{id}` is incomplete: {} {} not set, and a partial credential \
             never falls back to the API key",
            missing.join(", "),
            if missing.len() == 1 { "is" } else { "are" }
        )));
    }
    Ok(JwtTable {
        credential_id: values.remove("credential_id"),
        developer_id: values.remove("developer_id"),
        project_id: values.remove("project_id"),
        private_key: values.remove("private_key"),
    }
    .credential())
}

/// Rejects an empty value, so a stray newline cannot store an unusable credential.
fn checked_value(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::Usage("the API key is empty".to_owned()));
    }
    Ok(value)
}

/// Refuses a key file that group or other can read.
#[cfg(unix)]
fn check_mode(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = fs::metadata(path)
        .map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(Error::Config(format!(
            "{} is readable by group/other (mode {mode:04o}); run `chmod 600 {}`",
            path.display(),
            path.display()
        )));
    }
    Ok(())
}

/// Non-Unix shim: POSIX permission bits do not exist there, so there is nothing to check.
#[cfg(not(unix))]
fn check_mode(path: &Path) -> Result<()> {
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{KeyFile, KeyStore, entry};

    fn store_at(path: &std::path::Path) -> KeyStore {
        KeyStore {
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn masking_keeps_the_key_unrecognisable() {
        assert_eq!(KeyStore::mask("sk-test-abcdef123456"), "sk-t…56");
        assert_eq!(KeyStore::mask("12345678"), "1234…78");
        assert_eq!(KeyStore::mask("短い"), "…");
        assert_eq!(KeyStore::mask(""), "…");
        assert_eq!(KeyStore::mask("abcdefg"), "…");
    }

    #[test]
    fn environment_variables_follow_the_provider_registry() {
        assert_eq!(
            KeyStore::env_var("openweathermap").expect("a known provider"),
            Some("CIRROCAST_OPENWEATHERMAP_KEY")
        );
        assert_eq!(KeyStore::env_var("smhi").expect("a keyless provider"), None);
        let error = KeyStore::env_var("nope").expect_err("an unknown provider");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn any_spelling_of_a_provider_finds_its_entry() {
        let mut keys = BTreeMap::new();
        keys.insert("open-weather-map".to_owned(), "secret".to_owned());
        assert_eq!(
            entry(&keys, crate::provider::ProviderId::OpenWeatherMap).map(String::as_str),
            Some("secret")
        );
        assert_eq!(entry(&keys, crate::provider::ProviderId::Smhi), None);
    }

    #[test]
    fn a_missing_key_file_is_an_empty_store() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = store_at(&directory.path().join("keys.toml"));
        assert_eq!(store.read().expect("a missing file is empty").keys.len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_reported_as_such_not_as_missing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("keys.toml");
        std::os::unix::fs::symlink(directory.path().join("gone.toml"), &path)
            .expect("the symlink is created");
        let store = store_at(&path);
        let error = store
            .read()
            .expect_err("a dangling symlink is not an empty store");
        assert!(
            error.to_string().contains("dangling symbolic link"),
            "{error}"
        );
    }

    #[test]
    fn the_key_file_round_trips_through_the_atomic_writer() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("keys.toml");
        let store = store_at(&path);

        let mut file = KeyFile::default();
        file.keys
            .insert("qweather".to_owned(), "a-secret-value".to_owned());
        store.write(&file).expect("the write succeeds");

        let read_back = store.read().expect("the file parses");
        assert_eq!(
            read_back.keys.get("qweather").map(String::as_str),
            Some("a-secret-value")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("the file exists")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "the key file must be owner-only");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_group_readable_key_file_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("keys.toml");
        let store = store_at(&path);
        store
            .write(&KeyFile::default())
            .expect("the write succeeds");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("chmod succeeds");
        let error = store.read().expect_err("a 0644 key file is refused");
        assert_eq!(error.exit_code(), 4);
        assert!(
            error.to_string().contains("mode 0644") && error.to_string().contains("chmod 600"),
            "{error}"
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("chmod succeeds");
        store.read().expect("a 0600 key file is accepted");
    }
}
