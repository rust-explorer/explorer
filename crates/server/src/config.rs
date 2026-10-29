use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use openidconnect::reqwest::Certificate;
use secrecy::SecretString;
use url::Url;

const DEFAULT_SESSION_TTL: Duration = Duration::from_hours(7 * 24);

const VAR_PUBLIC_URL: &str = "EXPLORER_PUBLIC_URL";
const VAR_ISSUER: &str = "EXPLORER_OIDC_ISSUER";
const VAR_CLIENT_ID: &str = "EXPLORER_OIDC_CLIENT_ID";
const VAR_CLIENT_SECRET: &str = "EXPLORER_OIDC_CLIENT_SECRET";
const VAR_CLIENT_SECRET_FILE: &str = "EXPLORER_OIDC_CLIENT_SECRET_FILE";
const VAR_CA_FILE: &str = "EXPLORER_OIDC_CA_FILE";
const VAR_ALLOWED_GROUP: &str = "EXPLORER_ALLOWED_GROUP";
const VAR_GROUPS_CLAIM: &str = "EXPLORER_OIDC_GROUPS_CLAIM";
const VAR_SESSION_TTL: &str = "EXPLORER_SESSION_TTL";
const VAR_LOG: &str = "EXPLORER_LOG";
const VAR_LOG_FORMAT: &str = "EXPLORER_LOG_FORMAT";
const VAR_DATA_DIR: &str = "EXPLORER_DATA_DIR";
const VAR_CONFIG_DIR: &str = "EXPLORER_CONFIG_DIR";
const VAR_WEB_DIR: &str = "EXPLORER_WEB_DIR";

const ALL_VARS: [&str; 14] = [
    VAR_PUBLIC_URL,
    VAR_ISSUER,
    VAR_CLIENT_ID,
    VAR_CLIENT_SECRET,
    VAR_CLIENT_SECRET_FILE,
    VAR_CA_FILE,
    VAR_ALLOWED_GROUP,
    VAR_GROUPS_CLAIM,
    VAR_SESSION_TTL,
    VAR_LOG,
    VAR_LOG_FORMAT,
    VAR_DATA_DIR,
    VAR_CONFIG_DIR,
    VAR_WEB_DIR,
];

#[derive(Debug)]
pub struct Config {
    /// Scheme, host and optional port only; no path, no trailing slash beyond the root.
    pub public_url: Url,
    pub oidc: OidcConfig,
    pub session_ttl: Duration,
    pub log: LogConfig,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub web_dir: PathBuf,
}

#[derive(Debug)]
pub struct OidcConfig {
    /// Exactly as configured: discovery compares it byte-for-byte with the provider's issuer.
    pub issuer: String,
    pub client_id: String,
    pub client_secret: SecretString,
    /// Trusted in addition to the built-in roots when talking to the provider.
    pub ca_certs: Vec<Certificate>,
    pub allowed_group: String,
    pub groups_claim: String,
}

#[derive(Debug)]
pub struct LogConfig {
    pub filter: String,
    pub format: LogFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{var}: {reason}")]
    Invalid { var: &'static str, reason: String },
    #[error("cannot read secret file {}: {source}", path.display())]
    SecretFile { path: PathBuf, source: io::Error },
    #[error("cannot read {VAR_CA_FILE} {}: {source}", path.display())]
    CaFile { path: PathBuf, source: io::Error },
}

#[derive(Debug)]
pub struct ConfigErrors(pub Vec<ConfigError>);

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigErrors {}

impl Config {
    pub fn from_env() -> Result<Self, ConfigErrors> {
        Self::from_os_lookup(|key| std::env::var_os(key))
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigErrors> {
        Self::from_os_lookup(|key| lookup(key).map(OsString::from))
    }

    pub fn from_os_lookup(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self, ConfigErrors> {
        let mut errors = Vec::new();
        let mut values = HashMap::new();
        for var in ALL_VARS {
            match lookup(var).map(OsString::into_string) {
                None => {}
                Some(Ok(value)) => {
                    values.insert(var, value);
                }
                Some(Err(_)) => errors.push(invalid(var, "value is not valid UTF-8")),
            }
        }
        let non_utf8: Vec<&str> = errors
            .iter()
            .filter_map(|e| match e {
                ConfigError::Invalid { var, .. } => Some(*var),
                _ => None,
            })
            .collect();
        Self::from_values(|key| values.get(key).cloned(), errors, &non_utf8)
    }

    fn from_values(
        lookup: impl Fn(&str) -> Option<String>,
        mut errors: Vec<ConfigError>,
        non_utf8: &[&str],
    ) -> Result<Self, ConfigErrors> {
        let get = |key: &str| lookup(key).filter(|v| !v.is_empty());
        let mut required = |var: &'static str| {
            let value = get(var);
            if value.is_none() && !non_utf8.contains(&var) {
                errors.push(ConfigError::Missing(var));
            }
            value
        };

        let public_url = required(VAR_PUBLIC_URL);
        let issuer = required(VAR_ISSUER);
        let client_id = required(VAR_CLIENT_ID);
        let allowed_group = required(VAR_ALLOWED_GROUP);

        let public_url = public_url.and_then(|v| collect(&mut errors, parse_public_url(&v)));
        let issuer = issuer.and_then(|v| collect(&mut errors, parse_issuer(v)));
        let client_secret = collect(&mut errors, read_client_secret(&get));
        let ca_certs = collect(&mut errors, read_ca_certs(&get));

        let session_ttl = match get(VAR_SESSION_TTL) {
            None => Some(DEFAULT_SESSION_TTL),
            Some(v) => collect(&mut errors, parse_session_ttl(&v)),
        };
        let format = match get(VAR_LOG_FORMAT).as_deref() {
            None | Some("pretty") => Some(LogFormat::Pretty),
            Some("json") => Some(LogFormat::Json),
            Some(other) => {
                errors.push(ConfigError::Invalid {
                    var: VAR_LOG_FORMAT,
                    reason: format!("expected `pretty` or `json`, got `{other}`"),
                });
                None
            }
        };

        let log_filter = get(VAR_LOG).unwrap_or_else(|| "info".to_owned());
        if let Err(reason) = validate_log_filter(&log_filter) {
            errors.push(invalid(VAR_LOG, reason));
        }

        let path = |var: &str, default: &str| PathBuf::from(get(var).as_deref().unwrap_or(default));

        match (
            public_url,
            issuer,
            client_id,
            client_secret,
            ca_certs,
            allowed_group,
            session_ttl,
            format,
        ) {
            (
                Some(public_url),
                Some(issuer),
                Some(client_id),
                Some(client_secret),
                Some(ca_certs),
                Some(allowed_group),
                Some(session_ttl),
                Some(format),
            ) if errors.is_empty() => Ok(Self {
                public_url,
                oidc: OidcConfig {
                    issuer,
                    client_id,
                    client_secret,
                    ca_certs,
                    allowed_group,
                    groups_claim: get(VAR_GROUPS_CLAIM).unwrap_or_else(|| "groups".to_owned()),
                },
                session_ttl,
                log: LogConfig {
                    filter: log_filter,
                    format,
                },
                data_dir: path(VAR_DATA_DIR, "/data"),
                config_dir: path(VAR_CONFIG_DIR, "/config"),
                web_dir: path(VAR_WEB_DIR, "/usr/share/explorer/web"),
            }),
            _ => Err(ConfigErrors(errors)),
        }
    }
}

fn collect<T>(errors: &mut Vec<ConfigError>, result: Result<T, ConfigError>) -> Option<T> {
    result.map_err(|e| errors.push(e)).ok()
}

fn invalid(var: &'static str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        var,
        reason: reason.into(),
    }
}

fn validate_log_filter(value: &str) -> Result<(), String> {
    const LEVELS: [&str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
    let is_level = |s: &str| LEVELS.iter().any(|l| l.eq_ignore_ascii_case(s));
    for directive in value.split(',').map(str::trim) {
        let valid = match directive.split_once('=') {
            None => is_level(directive),
            Some((target, level)) => {
                !target.is_empty() && !target.contains(['[', ']']) && is_level(level)
            }
        };
        if !valid {
            return Err(format!(
                "invalid directive `{directive}`, expected a level ({}) or `target=level`",
                LEVELS.join("|")
            ));
        }
    }
    tracing_subscriber::EnvFilter::try_new(value)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn parse_http_url(var: &'static str, value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value).map_err(|e| invalid(var, e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(invalid(var, "must be an http(s) URL"));
    }
    Ok(url)
}

fn parse_public_url(value: &str) -> Result<Url, ConfigError> {
    let url = parse_http_url(VAR_PUBLIC_URL, value)?;
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid(
            VAR_PUBLIC_URL,
            "must contain only scheme, host and optional port (no path, query or credentials)",
        ));
    }
    Ok(url)
}

fn parse_issuer(value: String) -> Result<String, ConfigError> {
    parse_http_url(VAR_ISSUER, &value)?;
    Ok(value)
}

fn parse_session_ttl(value: &str) -> Result<Duration, ConfigError> {
    let ttl =
        humantime::parse_duration(value).map_err(|e| invalid(VAR_SESSION_TTL, e.to_string()))?;
    if ttl.is_zero() {
        return Err(invalid(VAR_SESSION_TTL, "must be greater than zero"));
    }
    Ok(ttl)
}

fn read_ca_certs(get: &impl Fn(&str) -> Option<String>) -> Result<Vec<Certificate>, ConfigError> {
    let Some(file) = get(VAR_CA_FILE) else {
        return Ok(Vec::new());
    };
    let path = PathBuf::from(file);
    let pem = std::fs::read(&path).map_err(|source| ConfigError::CaFile {
        path: path.clone(),
        source,
    })?;
    let certs = Certificate::from_pem_bundle(&pem)
        .map_err(|err| invalid(VAR_CA_FILE, format!("invalid PEM certificate: {err}")))?;
    if certs.is_empty() {
        return Err(invalid(VAR_CA_FILE, "no PEM certificate found in the file"));
    }
    Ok(certs)
}

fn read_client_secret(get: &impl Fn(&str) -> Option<String>) -> Result<SecretString, ConfigError> {
    match (get(VAR_CLIENT_SECRET), get(VAR_CLIENT_SECRET_FILE)) {
        (Some(_), Some(_)) => Err(invalid(
            VAR_CLIENT_SECRET,
            format!("set either {VAR_CLIENT_SECRET} or {VAR_CLIENT_SECRET_FILE}, not both"),
        )),
        (Some(secret), None) => Ok(SecretString::from(secret)),
        (None, Some(file)) => {
            let path = PathBuf::from(file);
            let content =
                std::fs::read_to_string(&path).map_err(|source| ConfigError::SecretFile {
                    path: path.clone(),
                    source,
                })?;
            let secret = content.trim_end_matches(['\r', '\n']);
            if secret.is_empty() {
                return Err(invalid(VAR_CLIENT_SECRET_FILE, "secret file is empty"));
            }
            Ok(SecretString::from(secret))
        }
        (None, None) => Err(ConfigError::Missing(VAR_CLIENT_SECRET)),
    }
}

/// A self-signed CA certificate (valid until 2126) shared by the tests that load roots.
#[cfg(test)]
pub(crate) const TEST_CA_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIBjDCCATOgAwIBAgIUcresLwbbHwTh4r1aZRgkT3VG0kQwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQRXhwbG9yZXIgdGVzdCBDQTAgFw0yNjEwMjkyMTI1MDlaGA8y
MTI2MTAwNTIxMjUwOVowGzEZMBcGA1UEAwwQRXhwbG9yZXIgdGVzdCBDQTBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABBs7qSrkP/HwphnUN7HXyPKB0SLYvLGFxQCB
GC6Lx08fosKJ6z11CivZZMgi4TbrZfLpqQcrdXIydivk5In+NoejUzBRMB0GA1Ud
DgQWBBQfN2QleOQhxBAT9cUd0xtEwYhxQDAfBgNVHSMEGDAWgBQfN2QleOQhxBAT
9cUd0xtEwYhxQDAPBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0cAMEQCIBlA
JCG0Ugig1/Xx1lN47S4+rR1LbDGhlsALlmfzA9vyAiAC1DMlNk/xfSVcwcKiaac3
/yhabvF0HpLwEoNmTcQoAw==
-----END CERTIFICATE-----
";

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;
    use std::time::Duration;

    use secrecy::ExposeSecret;

    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn valid() -> Vec<(&'static str, &'static str)> {
        vec![
            ("EXPLORER_PUBLIC_URL", "https://files.example.com"),
            ("EXPLORER_OIDC_ISSUER", "https://auth.example.com"),
            ("EXPLORER_OIDC_CLIENT_ID", "explorer"),
            ("EXPLORER_OIDC_CLIENT_SECRET", "s3cret"),
            ("EXPLORER_ALLOWED_GROUP", "explorer-users"),
        ]
    }

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut pairs: Vec<_> = valid()
            .into_iter()
            .filter(|(k, _)| !extra.iter().any(|(ek, _)| ek == k))
            .collect();
        pairs.extend_from_slice(extra);
        pairs
    }

    fn errors(pairs: &[(&str, &str)]) -> Vec<ConfigError> {
        Config::from_lookup(lookup(pairs)).unwrap_err().0
    }

    #[test]
    fn loads_valid_config_with_defaults() {
        let cfg = Config::from_lookup(lookup(&valid())).unwrap();
        assert_eq!(cfg.public_url.as_str(), "https://files.example.com/");
        assert_eq!(cfg.oidc.issuer, "https://auth.example.com");
        assert_eq!(cfg.oidc.client_id, "explorer");
        assert_eq!(cfg.oidc.client_secret.expose_secret(), "s3cret");
        assert_eq!(cfg.oidc.allowed_group, "explorer-users");
        assert_eq!(cfg.oidc.groups_claim, "groups");
        assert_eq!(cfg.session_ttl, Duration::from_hours(7 * 24));
        assert_eq!(cfg.log.filter, "info");
        assert_eq!(cfg.log.format, LogFormat::Pretty);
        assert_eq!(cfg.data_dir, PathBuf::from("/data"));
        assert_eq!(cfg.config_dir, PathBuf::from("/config"));
        assert_eq!(cfg.web_dir, PathBuf::from("/usr/share/explorer/web"));
    }

    #[test]
    fn reports_all_missing_required_vars_at_once() {
        let errs = errors(&[]);
        let mut missing: Vec<&str> = errs
            .iter()
            .map(|e| match e {
                ConfigError::Missing(var) => *var,
                other => panic!("unexpected error: {other}"),
            })
            .collect();
        missing.sort_unstable();
        assert_eq!(
            missing,
            [
                "EXPLORER_ALLOWED_GROUP",
                "EXPLORER_OIDC_CLIENT_ID",
                "EXPLORER_OIDC_CLIENT_SECRET",
                "EXPLORER_OIDC_ISSUER",
                "EXPLORER_PUBLIC_URL",
            ]
        );
    }

    #[test]
    fn empty_values_count_as_missing() {
        let errs = errors(&with(&[("EXPLORER_OIDC_CLIENT_ID", "")]));
        assert!(matches!(
            errs.as_slice(),
            [ConfigError::Missing("EXPLORER_OIDC_CLIENT_ID")]
        ));
    }

    #[test]
    fn reads_secret_from_file_and_trims_trailing_newline() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"s3cret\n").unwrap();
        let path = file.path().to_str().unwrap();
        let pairs: Vec<_> = valid()
            .into_iter()
            .filter(|(k, _)| *k != "EXPLORER_OIDC_CLIENT_SECRET")
            .chain([("EXPLORER_OIDC_CLIENT_SECRET_FILE", path)])
            .collect();
        let cfg = Config::from_lookup(lookup(&pairs)).unwrap();
        assert_eq!(cfg.oidc.client_secret.expose_secret(), "s3cret");
    }

    fn with_ca_file(content: &[u8]) -> Result<Config, Vec<ConfigError>> {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(content).unwrap();
        let path = file.path().to_str().unwrap();
        let pairs: Vec<_> = valid()
            .into_iter()
            .chain([("EXPLORER_OIDC_CA_FILE", path)])
            .collect();
        Config::from_lookup(lookup(&pairs)).map_err(|e| e.0)
    }

    #[test]
    fn no_ca_file_means_no_extra_roots() {
        let cfg = Config::from_lookup(lookup(&valid())).unwrap();
        assert!(cfg.oidc.ca_certs.is_empty());
    }

    #[test]
    fn loads_every_certificate_of_the_ca_file() {
        let bundle = format!("{TEST_CA_PEM}\n{TEST_CA_PEM}");
        let cfg = with_ca_file(bundle.as_bytes()).unwrap();
        assert_eq!(cfg.oidc.ca_certs.len(), 2);
    }

    #[test]
    fn rejects_ca_file_without_certificate() {
        for content in [&b""[..], b"not a certificate\n"] {
            let errs = with_ca_file(content).unwrap_err();
            assert!(matches!(
                errs.as_slice(),
                [ConfigError::Invalid {
                    var: "EXPLORER_OIDC_CA_FILE",
                    ..
                }]
            ));
        }
    }

    #[test]
    fn reports_unreadable_ca_file() {
        let pairs = with(&[("EXPLORER_OIDC_CA_FILE", "/nonexistent/ca.pem")]);
        let errs = errors(&pairs);
        assert!(matches!(errs.as_slice(), [ConfigError::CaFile { .. }]));
        assert!(errs[0].to_string().contains("EXPLORER_OIDC_CA_FILE"));
    }

    #[test]
    fn reports_unreadable_secret_file() {
        let pairs: Vec<_> = valid()
            .into_iter()
            .filter(|(k, _)| *k != "EXPLORER_OIDC_CLIENT_SECRET")
            .chain([("EXPLORER_OIDC_CLIENT_SECRET_FILE", "/nonexistent/secret")])
            .collect();
        let errs = errors(&pairs);
        assert!(matches!(errs.as_slice(), [ConfigError::SecretFile { .. }]));
    }

    #[test]
    fn rejects_both_secret_and_secret_file() {
        let mut pairs = valid();
        pairs.push(("EXPLORER_OIDC_CLIENT_SECRET_FILE", "/run/secrets/x"));
        let errs = errors(&pairs);
        assert!(matches!(
            errs.as_slice(),
            [ConfigError::Invalid {
                var: "EXPLORER_OIDC_CLIENT_SECRET",
                ..
            }]
        ));
    }

    #[test]
    fn normalises_public_url_trailing_slash() {
        for input in ["https://files.example.com", "https://files.example.com/"] {
            let cfg =
                Config::from_lookup(lookup(&with(&[("EXPLORER_PUBLIC_URL", input)]))).unwrap();
            assert_eq!(cfg.public_url.as_str(), "https://files.example.com/");
        }
    }

    #[test]
    fn keeps_port_in_public_url() {
        let cfg = Config::from_lookup(lookup(&with(&[(
            "EXPLORER_PUBLIC_URL",
            "http://localhost:8080",
        )])))
        .unwrap();
        assert_eq!(cfg.public_url.as_str(), "http://localhost:8080/");
    }

    #[test]
    fn rejects_public_url_with_path_or_query() {
        for input in ["https://example.com/explorer", "https://example.com/?a=1"] {
            let errs = errors(&with(&[("EXPLORER_PUBLIC_URL", input)]));
            assert!(
                matches!(
                    errs.as_slice(),
                    [ConfigError::Invalid {
                        var: "EXPLORER_PUBLIC_URL",
                        ..
                    }]
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_non_http_public_url() {
        let errs = errors(&with(&[("EXPLORER_PUBLIC_URL", "ftp://example.com")]));
        assert!(matches!(
            errs.as_slice(),
            [ConfigError::Invalid {
                var: "EXPLORER_PUBLIC_URL",
                ..
            }]
        ));
    }

    #[test]
    fn rejects_non_http_issuer() {
        let errs = errors(&with(&[("EXPLORER_OIDC_ISSUER", "not a url")]));
        assert!(matches!(
            errs.as_slice(),
            [ConfigError::Invalid {
                var: "EXPLORER_OIDC_ISSUER",
                ..
            }]
        ));
    }

    #[test]
    fn issuer_is_kept_verbatim() {
        for input in ["https://id.example.com", "https://id.example.com/"] {
            let cfg =
                Config::from_lookup(lookup(&with(&[("EXPLORER_OIDC_ISSUER", input)]))).unwrap();
            assert_eq!(cfg.oidc.issuer, input);
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_value_is_reported_invalid() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let map: HashMap<String, String> = valid()
            .into_iter()
            .filter(|(k, _)| *k != "EXPLORER_PUBLIC_URL")
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let errs = Config::from_os_lookup(|key| match key {
            "EXPLORER_PUBLIC_URL" => Some(OsString::from_vec(vec![0x68, 0xff, 0xfe])),
            _ => map.get(key).map(OsString::from),
        })
        .unwrap_err()
        .0;
        assert!(
            matches!(
                errs.as_slice(),
                [ConfigError::Invalid { var: "EXPLORER_PUBLIC_URL", reason }] if reason.contains("UTF-8")
            ),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_bad_session_ttl() {
        let errs = errors(&with(&[("EXPLORER_SESSION_TTL", "forever")]));
        assert!(matches!(
            errs.as_slice(),
            [ConfigError::Invalid {
                var: "EXPLORER_SESSION_TTL",
                ..
            }]
        ));
    }

    #[test]
    fn parses_session_ttl() {
        let cfg = Config::from_lookup(lookup(&with(&[("EXPLORER_SESSION_TTL", "12h")]))).unwrap();
        assert_eq!(cfg.session_ttl, Duration::from_hours(12));
    }

    #[test]
    fn rejects_invalid_log_filter() {
        for bad in [
            "debgu",
            "info,,",
            "explorer_server=debgu",
            "=debug",
            "a=b=c",
        ] {
            let errs = errors(&with(&[("EXPLORER_LOG", bad)]));
            assert!(
                matches!(
                    errs.as_slice(),
                    [ConfigError::Invalid {
                        var: "EXPLORER_LOG",
                        ..
                    }]
                ),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_valid_log_filters() {
        for good in [
            "info",
            "explorer_server=debug,tower_http=info",
            "warn,explorer_server=trace",
        ] {
            let config = Config::from_lookup(lookup(&with(&[("EXPLORER_LOG", good)]))).unwrap();
            assert_eq!(config.log.filter, good);
        }
    }

    #[test]
    fn rejects_bad_log_format() {
        let errs = errors(&with(&[("EXPLORER_LOG_FORMAT", "xml")]));
        assert!(matches!(
            errs.as_slice(),
            [ConfigError::Invalid {
                var: "EXPLORER_LOG_FORMAT",
                ..
            }]
        ));
    }

    #[test]
    fn accepts_json_log_format_and_path_overrides() {
        let cfg = Config::from_lookup(lookup(&with(&[
            ("EXPLORER_LOG_FORMAT", "json"),
            ("EXPLORER_DATA_DIR", "/tmp/d"),
            ("EXPLORER_CONFIG_DIR", "/tmp/c"),
            ("EXPLORER_WEB_DIR", "/tmp/w"),
        ])))
        .unwrap();
        assert_eq!(cfg.log.format, LogFormat::Json);
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/d"));
        assert_eq!(cfg.config_dir, PathBuf::from("/tmp/c"));
        assert_eq!(cfg.web_dir, PathBuf::from("/tmp/w"));
    }

    #[test]
    fn collects_multiple_errors_and_displays_one_per_line() {
        let errs = Config::from_lookup(lookup(&with(&[
            ("EXPLORER_PUBLIC_URL", "ftp://example.com"),
            ("EXPLORER_SESSION_TTL", "forever"),
        ])))
        .unwrap_err();
        assert_eq!(errs.0.len(), 2);
        assert_eq!(errs.to_string().lines().count(), 2);
    }

    #[test]
    fn debug_output_does_not_contain_secret() {
        let cfg = Config::from_lookup(lookup(&valid())).unwrap();
        assert!(!format!("{cfg:?}").contains("s3cret"));
    }
}
