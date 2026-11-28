use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::time::Duration;

use base64::Engine as _;
use cookie_store::CookieStore;
use impasto::{ImpastoType, PluginContext};
use impasto_file_system::{Scope, resolve_path};
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::HttpError;

#[derive(ImpastoType)]
pub(crate) struct ClientOptions {
    pub(crate) cookie_jar: Option<CookieOptions>,
    pub(crate) proxy: Option<ProxyOptions>,
    pub(crate) pins: Option<Vec<(String, Vec<String>)>>,
}

#[derive(ImpastoType)]
pub(crate) struct ProxyOptions {
    pub(crate) url: String,
    pub(crate) bypass: Option<Vec<String>>,
}

#[derive(ImpastoType)]
pub(crate) struct CookieOptions {
    mode: String,
    name: Option<String>,
}

#[derive(ImpastoType)]
pub(crate) struct CookieEntry {
    name: String,
    value: String,
    domain: String,
    path: String,
    expires_at_ms: Option<f64>,
    http_only: bool,
    secure: bool,
}

type JarRegistry = Mutex<BTreeMap<PathBuf, Weak<CookieJar>>>;
static JARS: OnceLock<JarRegistry> = OnceLock::new();

pub(crate) struct CookieJar {
    store: Mutex<CookieStore>,
    persistence: Option<Persistence>,
}

struct Persistence {
    pending: mpsc::SyncSender<Arc<CookieJar>>,
    owner: Weak<CookieJar>,
}

impl CookieJar {
    pub(crate) fn new(ctx: &PluginContext, options: CookieOptions) -> Result<Arc<Self>, HttpError> {
        if options.mode == "memory" {
            return Ok(Arc::new(Self {
                store: Mutex::default(),
                persistence: None,
            }));
        }
        if options.mode != "persistent" {
            return Err(invalid("Invalid Cookie Jar mode"));
        }
        let name = options.name.as_deref().unwrap_or("default");
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(invalid("Invalid Cookie Jar name"));
        }
        let path = resolve_path(
            ctx,
            Scope::ApplicationSupport,
            &format!("impasto-http/cookies/{name}.json"),
        )
        .map_err(|_| error("outside-scope", "Cookie Jar path is outside its scope"))?;
        Self::persistent(path)
    }

    fn persistent(path: PathBuf) -> Result<Arc<Self>, HttpError> {
        let mut jars = JARS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| error("io", "Cookie Jar registry is unavailable"))?;
        if let Some(existing) = jars.get(&path).and_then(Weak::upgrade) {
            return Ok(existing);
        }
        jars.retain(|_, jar| jar.strong_count() > 0);
        let root = path
            .parent()
            .ok_or_else(|| error("io", "Cookie Jar directory is unavailable"))?;
        fs::create_dir_all(root).map_err(|_| error("io", "Cookie Jar directory is unavailable"))?;
        let store = File::open(&path)
            .ok()
            .and_then(|file| cookie_store::serde::json::load(BufReader::new(file)).ok())
            .unwrap_or_default();
        let (pending, writes) = mpsc::sync_channel::<Arc<Self>>(1);
        let output = path.clone();
        std::thread::Builder::new()
            .name("impasto-http-cookies".into())
            .spawn(move || {
                while let Ok(jar) = writes.recv() {
                    std::thread::sleep(Duration::from_millis(250));
                    while writes.try_recv().is_ok() {}
                    if let Some(snapshot) = jar.snapshot()
                        && let Some(directory) = output.parent()
                        && let Ok(mut file) = tempfile::NamedTempFile::new_in(directory)
                        && file.write_all(&snapshot).is_ok()
                        && file.as_file().sync_all().is_ok()
                    {
                        let _ = file.persist(&output);
                    }
                }
            })
            .map_err(|_| error("io", "Could not start Cookie Jar persistence"))?;
        let jar = Arc::new_cyclic(|owner| Self {
            store: Mutex::new(store),
            persistence: Some(Persistence {
                pending,
                owner: Weak::clone(owner),
            }),
        });
        jars.insert(path, Arc::downgrade(&jar));
        Ok(jar)
    }

    fn snapshot(&self) -> Option<Vec<u8>> {
        let store = self.store.lock().ok()?;
        let mut bytes = Vec::new();
        cookie_store::serde::json::save(&store, &mut bytes).ok()?;
        Some(bytes)
    }

    pub(crate) fn value(&self, url: &str) -> Option<String> {
        let url = Url::parse(url).ok()?;
        let store = self.store.lock().ok()?;
        let result = store
            .get_request_values(&url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        (!result.is_empty()).then_some(result)
    }

    pub(crate) fn update(&self, url: &str, fields: &[(String, String)]) {
        for (_, value) in fields
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        {
            let _ = self.set(url, value);
        }
    }
    pub(crate) fn set(&self, url: &str, value: &str) -> Result<(), HttpError> {
        let url = Url::parse(url).map_err(|_| invalid("Invalid Cookie Jar URL"))?;
        let raw = cookie::Cookie::parse(value.to_owned()).map_err(|_| invalid("Invalid cookie"))?;
        self.store
            .lock()
            .map_err(|_| error("io", "Cookie Jar is unavailable"))?
            .store_response_cookies(std::iter::once(raw), &url);
        self.dirty();
        Ok(())
    }
    pub(crate) fn clear(&self) -> Result<(), HttpError> {
        self.store
            .lock()
            .map_err(|_| error("io", "Cookie Jar is unavailable"))?
            .clear();
        self.dirty();
        Ok(())
    }
    pub(crate) fn get(&self, url: &str) -> Result<Vec<CookieEntry>, HttpError> {
        let url = Url::parse(url).map_err(|_| invalid("Invalid Cookie Jar URL"))?;
        let store = self
            .store
            .lock()
            .map_err(|_| error("io", "Cookie Jar is unavailable"))?;
        Ok(store
            .matches(&url)
            .into_iter()
            .map(|cookie| CookieEntry {
                name: cookie.name().to_owned(),
                value: cookie.value().to_owned(),
                domain: cookie
                    .domain()
                    .unwrap_or(url.host_str().unwrap_or_default())
                    .to_owned(),
                path: cookie.path().unwrap_or("/").to_owned(),
                expires_at_ms: cookie
                    .expires_datetime()
                    .map(|time| time.unix_timestamp_nanos() as f64 / 1_000_000.0),
                http_only: cookie.http_only().unwrap_or(false),
                secure: cookie.secure().unwrap_or(false),
            })
            .collect())
    }
    fn dirty(&self) {
        if let Some(persistence) = &self.persistence
            && let Some(jar) = persistence.owner.upgrade()
        {
            let _ = persistence.pending.try_send(jar);
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Pins(Vec<(String, Vec<[u8; 32]>)>);

impl Pins {
    pub(crate) fn new(pins: Vec<(String, Vec<String>)>) -> Result<Self, HttpError> {
        pins.into_iter()
            .map(|(host, values)| {
                let name = host.strip_prefix("*.").unwrap_or(&host);
                if values.is_empty()
                    || name.is_empty()
                    || name.contains('*')
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
                {
                    return Err(invalid("Invalid certificate pin host"));
                }
                let hashes = values
                    .into_iter()
                    .map(|value| {
                        use base64::engine::general_purpose::STANDARD;
                        value
                            .strip_prefix("sha256/")
                            .and_then(|value| STANDARD.decode(value).ok())
                            .and_then(|bytes| bytes.try_into().ok())
                            .ok_or_else(|| invalid("Invalid certificate pin"))
                    })
                    .collect::<Result<_, _>>()?;
                Ok((host.to_ascii_lowercase(), hashes))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }
    pub(crate) fn required(&self, host: &str) -> bool {
        self.0.iter().any(|(rule, _)| host_match(rule, host))
    }
    pub(crate) fn verify<'a>(
        &self,
        host: &str,
        certificates: impl Iterator<Item = &'a [u8]>,
    ) -> bool {
        if !self.required(host) {
            return true;
        }
        certificates
            .filter_map(|certificate| {
                let (_, parsed) = x509_parser::parse_x509_certificate(certificate).ok()?;
                Some(<[u8; 32]>::from(Sha256::digest(
                    parsed.tbs_certificate.subject_pki.raw,
                )))
            })
            .any(|digest| {
                self.0
                    .iter()
                    .any(|(rule, pins)| host_match(rule, host) && pins.contains(&digest))
            })
    }
}
fn host_match(rule: &str, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if let Some(suffix) = rule.strip_prefix('*') {
        return host
            .strip_suffix(suffix)
            .is_some_and(|label| !label.is_empty() && !label.contains('.'));
    }
    rule == host
}

fn invalid(message: &'static str) -> HttpError {
    error("bad-url", message)
}
pub(crate) fn error(code: &'static str, message: &'static str) -> HttpError {
    HttpError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_cookie_namespace_is_shared_across_clients() -> Result<(), HttpError> {
        let directory =
            tempfile::tempdir().map_err(|_| error("io", "Could not create test directory"))?;
        let path = directory.path().join("profile.json");
        let first = CookieJar::persistent(path.clone())?;
        let second = CookieJar::persistent(path.clone())?;
        assert!(Arc::ptr_eq(&first, &second));
        let url = "https://example.test/notes";
        first.set(url, "first=1; Path=/; Max-Age=300; Secure")?;
        second.set(url, "second=2; Path=/; Max-Age=300")?;
        drop(first);
        drop(second);
        let reopened = CookieJar::persistent(path)?;
        assert_eq!(reopened.get(url)?.len(), 2);
        assert_eq!(reopened.get("http://example.test/notes")?.len(), 1);
        Ok(())
    }
}
