//! URL Predictor
//!
//! Purpose:
//! - Decide between **Navigate / Search
//! - Cross-platform via Rust core + FFI
//! - Supports pluggable Public Suffix List (PSL)
//!
//! This file is kept single-module for clarity. In production it can be split out.

use std::collections::{BTreeSet, HashSet};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::net::Ipv4Addr;

use idna::domain_to_ascii;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use url::Url;


// -----------------------------------------------------------------------------
// Optional PSL backend (enabled with feature = "real-psl")
// -----------------------------------------------------------------------------
#[cfg(feature = "real-psl")]
mod real_psl {
    use super::SuffixDb;
    use publicsuffix::{List as PslList, Psl, Type as SuffixType};

    pub struct RealSuffixDb {
        list: PslList,
    }

    impl RealSuffixDb {
        /// Build from PSL data (string or file).
        pub fn from_psl_string(psl_data: &str) -> Result<Self, String> {
            PslList::from_bytes(psl_data.as_bytes())
                .map(|list| Self { list })
                .map_err(|e| e.to_string())
        }

        #[allow(dead_code)]
        pub fn from_psl_file(path: &std::path::Path) -> Result<Self, String> {
            let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            Self::from_psl_string(&data)
        }
    }

    impl Default for RealSuffixDb {
        fn default() -> Self {
            // Uses vendored PSL in assets
            const PSL: &str = include_str!("../assets/public_suffix_list.dat");
            RealSuffixDb::from_psl_string(PSL).expect("failed to parse PSL")
        }
    }

    impl SuffixDb for RealSuffixDb {
        fn has_known_suffix(&self, host: &str, allow_private: bool) -> bool {
            if host.is_empty() {
                return false;
            }
            if let Some(domain) = self.list.domain(host.as_bytes()) {
                let sfx = domain.suffix();
                match sfx.typ() {
                    None => {
                        // e.g. ".test"
                        let sfx_name = std::str::from_utf8(sfx.as_bytes())
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        matches!(sfx_name.as_str(), "test")
                            | matches!(sfx_name.as_str(), "example")
                            | matches!(sfx_name.as_str(), "local")
                            | matches!(sfx_name.as_str(), "localhost")
                    }
                    Some(SuffixType::Private) => allow_private,
                    Some(_) => true,
                }
            } else {
                false
            }
        }
    }

    pub use RealSuffixDb as DefaultDb;
}

// -----------------------------------------------------------------------------
// Core API
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    /// Navigate to normalized URL
    Navigate { url: String },
    /// Otherwise: search
    Search { query: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub allow_intranet_multi_label: bool,
    pub allow_intranet_single_label: bool,
    pub allow_private_suffix: bool,
    pub allowed_schemes: BTreeSet<String>,
    #[serde(default)]
    pub allow_file_paths: bool,
}

impl Default for Policy {
    fn default() -> Self {
        let mut allowed = BTreeSet::new();
        for s in ["http", "https", "ftp", "file", "about", "view-source", "duck", "edge", "chrome"] {
            allowed.insert(s.to_string());
        }
        Self {
            allow_intranet_multi_label: false,
            allow_intranet_single_label: false,
            allow_private_suffix: true,
            allowed_schemes: allowed,
            allow_file_paths: false,
        }
    }
}

// -----------------------------------------------------------------------------
// PSL abstraction
// -----------------------------------------------------------------------------

pub trait SuffixDb: Send + Sync + 'static {
    fn has_known_suffix(&self, host: &str, allow_private: bool) -> bool;
}

/// Minimal demo suffix DB for tests
pub struct DemoSuffixDb {
    icann: HashSet<String>,
    private: HashSet<String>,
}

impl DemoSuffixDb {
    pub fn new() -> Self {
        let icann: HashSet<String> = [
            "com","org","net","edu","gov","mil","int","info","io","co",
            "uk","pt","de","fr","es","it","ru","cn","jp","br","in","test"
        ]
        .into_iter().map(|s| s.to_string()).collect();

        let private: HashSet<String> = ["appspot.com", "github.io", "pages.dev"]
            .into_iter().map(|s| s.to_string()).collect();

        Self { icann, private }
    }
}

impl Default for DemoSuffixDb {
    fn default() -> Self { Self::new() }
}

impl SuffixDb for DemoSuffixDb {
    fn has_known_suffix(&self, host: &str, allow_private: bool) -> bool {
        // Naive PSL emulation:
        // - lower case
        // - drop trailing dots (DNS absolute name marker)
        // There's a bunch of things that won't work in the demo PSL, like wildcar/exception
        // semantics
        let h = host.trim_end_matches('.').to_ascii_lowercase();
        let labels: Vec<&str> = h.split('.').collect();
        if labels.len() < 2 {
            return false;
        }
        let tld = labels.last().unwrap();
        if self.icann.contains(*tld) {
            return true;
        }
        if allow_private && labels.len() >= 2 {
            let last2 = format!("{}.{}", labels[labels.len()-2], labels[labels.len()-1]);
            return self.private.contains(&last2);
        }
        false
    }
}

// -----------------------------------------------------------------------------
// Default DB choice
// -----------------------------------------------------------------------------
#[cfg(feature = "real-psl")]
use real_psl::DefaultDb as DefaultSuffixDb;
#[cfg(not(feature = "real-psl"))]
type DefaultSuffixDb = DemoSuffixDb;

static DEFAULT_SUFFIX_DB: Lazy<DefaultSuffixDb> = Lazy::new(DefaultSuffixDb::default);

// -----------------------------------------------------------------------------
// Classification
// -----------------------------------------------------------------------------

pub fn classify(input: &str, policy: &Policy) -> Decision {
    classify_with_db(input, policy, &*DEFAULT_SUFFIX_DB)
}

pub fn classify_with_db(input: &str, policy: &Policy, db: &dyn SuffixDb) -> Decision {
    let original = input.trim();
    if original.is_empty() {
        return Decision::Search { query: String::new() };
    }

    // Absolute URL
    if let Some(abs) = parse_absolute_url_if_allowed(original, policy) {
        return Decision::Navigate { url: abs };
    }

    // Scheme-relative
    if original.starts_with("//") {
        let candidate = format!("https:{}", original);
        if let Ok(u) = Url::parse(&candidate) {
            if let Some(host) = u.host_str() {
                if host_like_valid(host) {
                    return Decision::Navigate { url: u.to_string() };
                }
            }
        }
    }

    // File path, e.g. "C:\Users\Username\Documents\file.html"
    if policy.allow_file_paths {
        if let Some(url) = is_file_path(original) {
            return Decision::Navigate { url };
        }
    }

    // Whitespace → search
    if original.split_whitespace().count() > 1 {
        return Decision::Search { query: original.to_string() };
    }

    // Host-like?
    if let Some(nav) = classify_host_like(original, policy, db) {
        return nav;
    }

    // Fallback
    Decision::Search { query: original.to_string() }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

fn parse_absolute_url_if_allowed(input: &str, policy: &Policy) -> Option<String> {
    if let Some(colon) = input.find(':') {
        let scheme = &input[..colon];
        if is_valid_scheme(scheme) && policy.allowed_schemes.contains(&scheme.to_ascii_lowercase()) {
            if let Ok(u) = Url::parse(input) {
                return Some(u.to_string());
            }
        }
    }
    None
}

fn classify_host_like(input: &str, policy: &Policy, db: &dyn SuffixDb) -> Option<Decision> {
    if let Some(nav) = ip_or_localhost_navigate(input) {
        return Some(nav);
    }

    let candidate = format!("http://{}", input);
    let u = Url::parse(&candidate).ok()?;
    let host = u.host_str()?;
    let ascii_host = to_idna_ascii(host)?;

    if !host_like_valid(&ascii_host) {
        return None;
    }

    let is_ipv4 = ascii_host.parse::<Ipv4Addr>().is_ok();
    if is_ipv4 {
        let raw_host = input.split('/').next().unwrap_or(input);
        // If the parsed host is a valid IPv4 address, but the host extracted from raw input is not,
        // then the raw input was filled with `0` octets - we don't want it unless the input contains a scheme,
        // otherwise we treat it as a search query.
        if !raw_host.parse::<Ipv4Addr>().is_ok() {
            return None;
        }
    }

    let has_dot = ascii_host.contains('.');
    let has_username = !u.username().is_empty();
    let has_port = u.port().is_some();
    let has_path = !u.path().is_empty() && u.path() != "/";
    let has_fragment = !u.fragment().unwrap_or("").is_empty();
    let ends_with_slash = input.ends_with('/');

    if has_username {
        let has_password = !u.password().unwrap_or("").is_empty();

        if !has_password && !has_path && !has_port && !has_fragment {
            return None;
        }
    }

    if has_dot {
        if policy.allow_intranet_multi_label && !has_path && !has_fragment {
            let has_query = !u.query().unwrap_or("").is_empty();
            if !has_query {
                return Some(Decision::Navigate { url: u.to_string() });
            }
        }
        if db.has_known_suffix(&ascii_host, policy.allow_private_suffix) {
            return Some(Decision::Navigate { url: u.to_string() });
        }
    }

    if ascii_host.starts_with("www.") {
        let rest = &ascii_host[4..];
        if rest.contains('.') && db.has_known_suffix(rest, policy.allow_private_suffix) {
            return Some(Decision::Navigate { url: u.to_string() });
        }
    }

    if !has_dot && (policy.allow_intranet_single_label || has_port) {
        return Some(Decision::Navigate { url: u.to_string() });
    }

    if (has_dot || has_port) && (has_path || ends_with_slash) {
        return Some(Decision::Navigate { url: u.to_string() });
    }

    None
}

// IP/localhost handling
fn ip_or_localhost_navigate(input: &str) -> Option<Decision> {
    let s = input.trim();
    let s = s.strip_prefix("//").unwrap_or(s);

    let (authority, rest) = match s.split_once('/') {
        Some((a, r)) => (a, Some(r)),
        None => (s, None),
    };

    let (host_part, _port_part) = if authority.starts_with('[') {
        if let Some(end) = authority.find(']') {
            let host = &authority[1..end];
            let after = &authority[end + 1..];
            let _port = after.strip_prefix(':');
            (host, _port)
        } else {
            return None;
        }
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
            _ => (authority, None),
        }
    };

    let host = host_part;

    if host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok() {
        let mut url = String::from("http://");
        if host.contains(':') && !host.starts_with('[') {
            url.push('[');
            url.push_str(host);
            url.push(']');
        } else {
            url.push_str(host);
        }
        if authority.contains(':') && !authority.starts_with('[') {
            if let Some((_, p)) = authority.rsplit_once(':') {
                if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
                    url.push(':');
                    url.push_str(p);
                }
            }
        } else if authority.starts_with('[') {
            if let Some(end) = authority.find(']') {
                let after = &authority[end + 1..];
                if let Some(port) = after.strip_prefix(':') {
                    url.push(':');
                    url.push_str(port);
                }
            }
        }
        if let Some(r) = rest {
            url.push('/');
            url.push_str(r);
        }
        if rest.is_none() {
            url.push('/');
        }
        return Some(Decision::Navigate { url });
    }

    None
}

fn is_valid_scheme(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() {
        return false;
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return false;
        }
    }
    true
}

fn to_idna_ascii(host: &str) -> Option<String> {
    domain_to_ascii(host).ok()
}

fn host_like_valid(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    let h = host.trim_end_matches('.');
    if h.is_empty() {
        return false;
    }
    for label in h.split('.') {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        let bytes = label.as_bytes();
        if bytes[0] == b'-' || bytes[label.len() - 1] == b'-' {
            return false;
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return false;
        }
    }
    if h.len() > 253 {
        return false;
    }
    true
}

fn is_file_path(input: &str) -> Option<String> {
    if let Ok(u) = Url::from_file_path(&input) {
        Some(u.to_string())
    } else {
        None
    }
}

#[cfg(feature = "real-psl")]
mod psl_buf {
    use std::sync::OnceLock;

    // Compile-time include of the PSL bytes
    const PSL_BYTES: &[u8] = include_bytes!("../assets/public_suffix_list.dat");

    // We expose a NUL-terminated view so C can treat it as a C string if desired.
    // Stored in a static so we never re-allocate and lifetime is 'static.
    static PSL_NUL: OnceLock<Box<[u8]>> = OnceLock::new();

    pub fn buf_with_trailing_nul() -> &'static [u8] {
        PSL_NUL.get_or_init(|| {
            let mut v = Vec::with_capacity(PSL_BYTES.len() + 1);
            v.extend_from_slice(PSL_BYTES);
            v.push(0);
            v.into_boxed_slice()
        })
    }
}

// -----------------------------------------------------------------------------
// C FFI
// -----------------------------------------------------------------------------

/// Classify an input string (URL-ish or search) using a JSON-encoded `Policy`.
///
/// # Parameters
/// - `input`: UTF-8 C string (NUL-terminated).
/// - `policy_json`: UTF-8 C string with a JSON object for `Policy`.
///
/// # Returns
/// A newly allocated UTF-8 JSON C string with a `Decision`.
/// Must be freed with [`ddg_up_free_string`].
///
/// # Safety
/// - `input` and `policy_json` must be valid pointers to NUL-terminated byte strings.
/// - The returned pointer must be freed only via [`ddg_up_free_string`].
#[no_mangle]
pub extern "C" fn ddg_up_classify_json(input: *const c_char, policy_json: *const c_char) -> *mut c_char {
    let input = unsafe { CStr::from_ptr(input) }.to_string_lossy().to_string();
    let policy_json = unsafe { CStr::from_ptr(policy_json) }.to_string_lossy().to_string();

    let policy: Policy = match serde_json::from_str(&policy_json) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("url_predictor: policy JSON parse error: {e}. Using defaults.");
            Policy::default()
        }
    };

    let decision = classify(&input, &policy);
    let json = serde_json::to_string(&decision)
        .unwrap_or_else(|_| "{\"Search\":{\"query\":\"\"}}".to_string());
    CString::new(json).unwrap().into_raw()
}

/// Free a string returned by this library (e.g., from [`ddg_up_classify_json`]).
///
/// Safe to call with NULL; it will do nothing.
///
/// # Safety
/// - `ptr` must be a pointer previously returned by this library.
///   Do **not** pass a pointer from `malloc`/`new`/stack.
#[no_mangle]
pub extern "C" fn ddg_up_free_string(ptr: *mut c_char) {
    if ptr.is_null() { return; }
    unsafe { let _ = CString::from_raw(ptr); }
}

/// Get a pointer to the in-memory Public Suffix List (PSL) bytes.
///
/// Available only when built with the `real-psl` feature.
///
/// The memory is **owned by the library** and is valid for the lifetime of the process.
/// Do **not** free it. The buffer is NUL-terminated for convenience.
///
/// Use together with [`ddg_up_get_psl_len`] to know the logical length (without the trailing NUL).
///
/// # Returns
/// `*const c_char` pointing to a read-only, NUL-terminated buffer.
#[cfg(feature = "real-psl")]
#[no_mangle]
pub extern "C" fn ddg_up_get_psl_ptr() -> *const c_char {
    psl_buf::buf_with_trailing_nul().as_ptr() as *const c_char
}

/// Get the length (in bytes) of the PSL buffer returned by [`ddg_up_get_psl_ptr`].
///
/// The length **excludes** the trailing NUL.
///
/// # Returns
/// `usize` length in bytes.
#[cfg(feature = "real-psl")]
#[no_mangle]
pub extern "C" fn ddg_up_get_psl_len() -> usize {
    // length *excluding* the trailing NUL
    psl_buf::buf_with_trailing_nul().len().saturating_sub(1)
}


// -----------------------------------------------------------------------------
// JNI (Android only)
// -----------------------------------------------------------------------------
#[cfg(any(target_os = "android", feature = "jni-host-tests"))]
#[no_mangle]
pub extern "system" fn Java_com_duckduckgo_urlpredictor_UrlPredictor_ddgClassifyJni(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    jinput: jni::objects::JString,
    jpolicy: jni::objects::JString,
) -> jni::sys::jstring {
    let input: String = env.get_string(&jinput).unwrap().into();
    let policy_json: String = env.get_string(&jpolicy).unwrap().into();
    let policy: Policy = serde_json::from_str(&policy_json).unwrap_or_default();

    let decision = classify(&input, &policy);
    let json = serde_json::to_string(&decision)
        .unwrap_or_else(|_| "{\"Search\":{\"query\":\"\"}}".into());
    env.new_string(json).unwrap().into_raw()
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn policy_default_inet() -> Policy {
        let p = Policy::default();
        p
    }

    #[test]
    fn absolute_urls() {
        let p = policy_default_inet();
        assert!(matches!(
            classify("https://例え.テスト/path?q=1", &p),
            Decision::Navigate { .. }
        ));
        assert!(matches!(
            classify("view-source:https://example.com", &p),
            Decision::Navigate { .. }
        ));
    }

    #[test]
    fn scheme_relative() {
        let p = policy_default_inet();
        let d = classify("//example.com/path", &p);
        assert!(matches!(d, Decision::Navigate { .. }));
    }

    #[test]
    fn ip_and_localhost() {
        let p = policy_default_inet();
        assert!(matches!(
            classify("127.0.0.1", &p),
            Decision::Navigate { .. }
        ));
        assert!(matches!(
            classify("127.0.0.1:3000/", &p),
            Decision::Navigate { .. }
        ));
        assert!(matches!(
            classify("[2001:db8::1]/a", &p),
            Decision::Navigate { .. }
        ));
        assert!(matches!(
            classify("localhost:8080/health", &p),
            Decision::Navigate { .. }
        ));
    }

    #[test]
    fn search_bias() {
        let p = policy_default_inet();
        assert!(matches!(
            classify("node.js tutorial", &p),
            Decision::Search { .. }
        ));
        assert!(matches!(
            classify("what.is my ip", &p),
            Decision::Search { .. }
        ));
        assert!(matches!(
            classify("something.orother", &p),
            Decision::Search { .. }
        ));
    }

    #[test]
    fn search_bias_with_allow_intranet_multi_label() {
        let mut p = policy_default_inet();
        p.allow_intranet_multi_label = true;
        assert!(matches!(
            classify("node.js tutorial", &p),
            Decision::Search { .. }
        ));
        assert!(matches!(
            classify("what.is my ip", &p),
            Decision::Search { .. }
        ));
        assert!(matches!(
            classify("something.orother", &p),
            Decision::Navigate { .. }
        ));
    }

    #[test]
    fn psl_driven() {
        let mut p = policy_default_inet();
        p.allow_intranet_single_label = false;
        let d = classify("example.com", &p);
        assert!(matches!(d, Decision::Navigate { .. }));

        let d = classify("www.test", &p); // no PSL
        assert!(matches!(d, Decision::Navigate { .. }));

        let d = classify("foo.github.io", &p); // private suffix demo
        assert!(matches!(d, Decision::Navigate { .. }));
    }

    #[test]
    fn intranet_single_label_policy() {
        let mut p = policy_default_inet();
        p.allow_intranet_single_label = false;
        assert!(matches!(classify("dev", &p), Decision::Search { .. }));
        p.allow_intranet_single_label = true;
        assert!(matches!(classify("dev", &p), Decision::Navigate { .. }));
        // single label with port navigates even if policy disallows
        p.allow_intranet_single_label = false;
        assert!(matches!(
            classify("dev:5173", &p),
            Decision::Navigate { .. }
        ));
    }

    #[test]
    fn intranet_multi_label_policy() {
        let mut p = Policy::default();
        p.allow_intranet_multi_label = true;
        assert!(matches!(classify("nas.local", &p), Decision::Navigate { url } if url == "http://nas.local/"));
        assert!(matches!(classify("nas.local:5000", &p), Decision::Navigate { url } if url == "http://nas.local:5000/"));
        assert!(matches!(classify("nas.local/login", &p), Decision::Navigate { url } if url == "http://nas.local/login"));
        assert!(matches!(classify("package.json", &p), Decision::Navigate { url } if url == "http://package.json/"));
    }

    #[test]
    fn ports_and_userinfo() {
        let p = Policy::default();
        assert!(matches!(classify("example.com:80", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("example.com:abc", &p), Decision::Search { .. }));
        assert!(matches!(classify("http://user:pass@example.com", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("http://user:@example.com", &p), Decision::Navigate { url } if url == "http://user@example.com/"));
        assert!(matches!(classify("user:pass@example.com", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("user@example.com", &p), Decision::Search { query } if query == "user@example.com"));
    }

    #[test]
    fn unicode_idna_and_invalid_labels() {
        let p = Policy::default();
        assert!(matches!(classify("bücher.de", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("xn--bcher-kva.de", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("-badlabel.com", &p), Decision::Search { .. }));
    }

    #[test]
    fn trailing_dot_and_weird_chars() {
        let p = Policy::default();
        assert!(matches!(classify("example.com.", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("exa_mple.com", &p), Decision::Search { .. }));
    }

    #[test]
    fn ipv4_require_scheme_or_4_octets() {
        let p = Policy::default();
        assert!(matches!(classify("127.0.0.1", &p), Decision::Navigate { url } if url == "http://127.0.0.1/"));
        assert!(matches!(classify("http://1.2.7", &p), Decision::Navigate { url } if url == "http://1.2.0.7/"));
        assert!(matches!(classify("1.2.7", &p), Decision::Search { query } if query == "1.2.7"));
        assert!(matches!(classify("1.2", &p), Decision::Search { query } if query == "1.2"));
        assert!(matches!(classify("127.1/3.4", &p), Decision::Search { query } if query == "127.1/3.4"));
    }

    #[test]
    fn macos_specific() {
        let p = Policy::default();
        assert!(matches!(classify("regular-domain.com/path/to/directory/", &p), Decision::Navigate { url } if url == "http://regular-domain.com/path/to/directory/"));
        assert!(matches!(classify("regular-domain.com/path/to/directory/", &p), Decision::Navigate { url } if url == "http://regular-domain.com/path/to/directory/"));
        assert!(matches!(classify("regular-domain.com", &p), Decision::Navigate { url } if url == "http://regular-domain.com/"));
        assert!(matches!(classify("regular-domain.com/", &p), Decision::Navigate { url } if url == "http://regular-domain.com/"));
        assert!(matches!(classify("regular-domain.com/filename", &p), Decision::Navigate { url } if url == "http://regular-domain.com/filename"));
        assert!(matches!(classify("regular-domain.com/filename?a=b&b=c", &p), Decision::Navigate { url } if url == "http://regular-domain.com/filename?a=b&b=c"));
        assert!(matches!(classify("regular-domain.com/filename/?a=b&b=c", &p), Decision::Navigate { url } if url == "http://regular-domain.com/filename/?a=b&b=c"));
        assert!(matches!(classify("http://regular-domain.com?a=b&b=c", &p), Decision::Navigate { url } if url == "http://regular-domain.com/?a=b&b=c"));
        assert!(matches!(classify("http://regular-domain.com/?a=b&b=c", &p), Decision::Navigate { url } if url == "http://regular-domain.com/?a=b&b=c"));
        assert!(matches!(classify("https://hexfiend.com/file?q=a", &p), Decision::Navigate { url } if url == "https://hexfiend.com/file?q=a"));
        assert!(matches!(classify("https://hexfiend.com/file/?q=a", &p), Decision::Navigate { url } if url == "https://hexfiend.com/file/?q=a"));
        assert!(matches!(classify("https://hexfiend.com/?q=a", &p), Decision::Navigate { url } if url == "https://hexfiend.com/?q=a"));
        assert!(matches!(classify("https://hexfiend.com?q=a", &p), Decision::Navigate { url } if url == "https://hexfiend.com/?q=a"));
        assert!(matches!(classify("regular-domain.com/path/to/file ", &p), Decision::Navigate { url } if url == "http://regular-domain.com/path/to/file"));
        assert!(matches!(classify("search string with spaces", &p), Decision::Search { query } if query == "search string with spaces"));
        assert!(matches!(classify("https://duckduckgo.com/?q=search string with spaces&arg 2=val 2", &p), Decision::Navigate { url } if url == "https://duckduckgo.com/?q=search%20string%20with%20spaces&arg%202=val%202"));
        assert!(matches!(classify("https://duckduckgo.com/?q=search+string+with+spaces", &p), Decision::Navigate { url } if url == "https://duckduckgo.com/?q=search+string+with+spaces"));
        assert!(matches!(classify("https://screwjankgames.github.io/engine programming/2020/09/24/writing-your.html", &p), Decision::Navigate { url } if url == "https://screwjankgames.github.io/engine%20programming/2020/09/24/writing-your.html"));
        assert!(matches!(classify("define: foo", &p), Decision::Search { query } if query == "define: foo"));
        assert!(matches!(classify("   http://example.com\n", &p), Decision::Navigate { url } if url == "http://example.com/"));
        assert!(matches!(classify(" duckduckgo.com", &p), Decision::Navigate { url } if url == "http://duckduckgo.com/"));
        assert!(matches!(classify(" duck duck go.c ", &p), Decision::Search { query } if query == "duck duck go.c"));
        assert!(matches!(classify("localhost ", &p), Decision::Navigate { url } if url == "http://localhost/"));
        assert!(matches!(classify("local ", &p), Decision::Search { query } if query == "local"));
        assert!(matches!(classify("test string with spaces", &p), Decision::Search { query } if query == "test string with spaces"));
        assert!(matches!(classify("http://💩.la:8080 ", &p), Decision::Navigate { url } if url == "http://xn--ls8h.la:8080/"));
        assert!(matches!(classify("http:// 💩.la:8080 ", &p), Decision::Search { query } if query == "http:// 💩.la:8080"));
        assert!(matches!(classify("https://xn--ls8h.la/path/to/resource", &p), Decision::Navigate { url } if url == "https://xn--ls8h.la/path/to/resource"));
        assert!(matches!(classify("16385-12228.72", &p), Decision::Search { query } if query == "16385-12228.72"));
        assert!(matches!(classify("user@localhost", &p), Decision::Search { query } if query == "user@localhost"));
        assert!(matches!(classify("http://user@domain.com", &p), Decision::Navigate { url } if url == "http://user@domain.com/"));
        assert!(matches!(classify("http://user: @domain.com", &p), Decision::Navigate { url } if url == "http://user:%20@domain.com/"));
        assert!(matches!(classify("http://user:,,@domain.com", &p), Decision::Navigate { url } if url == "http://user:,,@domain.com/"));
        assert!(matches!(classify("http://user:pass@domain.com", &p), Decision::Navigate { url } if url == "http://user:pass@domain.com/"));
        assert!(matches!(classify("http://user name:pass word@domain.com/folder name/file name/", &p), Decision::Navigate { url } if url == "http://user%20name:pass%20word@domain.com/folder%20name/file%20name/"));
        assert!(matches!(classify("1+(3+4*2)", &p), Decision::Search { query } if query == "1+(3+4*2)"));
        assert!(matches!(classify("localdomain", &p), Decision::Search { query } if query == "localdomain"));
        // different from macOS
        assert!(matches!(classify("test://hello/", &p), Decision::Search { query } if query == "test://hello/")); // navigate on macOS
    }

    #[test]
    fn windows_specific() {
        let p = Policy::default();
        assert!(matches!(classify("apple.com/mac/", &p), Decision::Navigate { url } if url == "http://apple.com/mac/"));
        assert!(matches!(classify("duckduckgo.com", &p), Decision::Navigate { url } if url == "http://duckduckgo.com/"));
        assert!(matches!(classify("duckduckgo", &p), Decision::Search { query } if query == "duckduckgo"));
        assert!(matches!(classify("www.duckduckgo.com", &p), Decision::Navigate { url } if url == "http://www.duckduckgo.com/"));
        assert!(matches!(classify("http://www.duckduckgo.com", &p), Decision::Navigate { url } if url == "http://www.duckduckgo.com/"));
        assert!(matches!(classify("https://www.duckduckgo.com", &p), Decision::Navigate { url } if url == "https://www.duckduckgo.com/"));
        assert!(matches!(classify("stuff.stor", &p), Decision::Search { query } if query == "stuff.stor"));
        assert!(matches!(classify("https://stuff.or", &p), Decision::Navigate { url } if url == "https://stuff.or/"));
        assert!(matches!(classify("stuff.org", &p), Decision::Navigate { url } if url == "http://stuff.org/"));
        assert!(matches!(classify("windows.applicationmodel.store.dll", &p), Decision::Search { query } if query == "windows.applicationmodel.store.dll"));
        assert!(matches!(classify("user:pass@domain.com", &p), Decision::Navigate { url } if url == "http://user:pass@domain.com/"));
        assert!(matches!(classify("user: @domain.com", &p), Decision::Search { query } if query == "user: @domain.com"));
        assert!(matches!(classify("user:,,@domain.com", &p), Decision::Navigate { url } if url == "http://user:,,@domain.com/"));
        assert!(matches!(classify("user:::@domain.com", &p), Decision::Navigate { url } if url == "http://user:%3A%3A@domain.com/"));
        assert!(matches!(classify("https://user@domain.com", &p), Decision::Navigate { url } if url == "https://user@domain.com/"));
        assert!(matches!(classify("https://user:pass@domain.com", &p), Decision::Navigate { url } if url == "https://user:pass@domain.com/"));
        assert!(matches!(classify("https://user: @domain.com", &p), Decision::Navigate { url } if url == "https://user:%20@domain.com/"));
        assert!(matches!(classify("https://user:,,@domain.com", &p), Decision::Navigate { url } if url == "https://user:,,@domain.com/"));
        assert!(matches!(classify("https://user:::@domain.com", &p), Decision::Navigate { url } if url == "https://user:%3A%3A@domain.com/"));

        // different from Windows
        assert!(matches!(classify("user@domain.com", &p), Decision::Search { query } if query == "user@domain.com"));
    }

    // ---------------------------
    // PSL wildcard/exception tests (real-psl only)
    // ---------------------------
    #[cfg(feature = "real-psl")]
    #[test]
    fn psl_wildcard_kawasaki_jp() {
        // PSL has a wildcard for *.kawasaki.jp (municipalities in Japan),
        // so domains like foo.kawasaki.jp should be recognized as having a known suffix.
        let mut p = Policy::default();
        p.allow_intranet_single_label = false;

        assert!(matches!(classify("foo.kawasaki.jp", &p), Decision::Search { .. }),
            "expected wildcard under *.kawasaki.jp to navigate");
        assert!(matches!(classify("bar.baz.kawasaki.jp", &p), Decision::Navigate { .. }),
            "deeper labels under *.kawasaki.jp should still navigate");
    }

    #[cfg(feature = "real-psl")]
    #[test]
    fn psl_exception_city_kawasaki_jp() {
        // PSL also has an exception rule: !city.kawasaki.jp
        // That means "city.kawasaki.jp" itself is a public suffix, and registrations happen under it.
        // In practice:
        // - "city.kawasaki.jp" is recognized as a public suffix (Navigate when typed)
        // - "foo.city.kawasaki.jp" is a registrable domain (Navigate as well)
        let mut p = Policy::default();
        p.allow_intranet_single_label = false;

        assert!(matches!(classify("city.kawasaki.jp", &p), Decision::Navigate { .. }),
            "exception rule makes city.kawasaki.jp a known suffix");
        assert!(matches!(classify("foo.city.kawasaki.jp", &p), Decision::Navigate { .. }),
            "labels under the exception should also navigate");
    }

    #[cfg(feature = "real-psl")]
    #[test]
    fn psl_private_suffix_still_respects_policy() {
        // Sanity: PRIVATE suffix like github.io should navigate when allowed,
        // and flip to Search when PRIVATE is disabled.
        let mut p = Policy::default();
        p.allow_intranet_single_label = false;

        assert!(matches!(classify("foo.github.io", &p), Decision::Navigate { .. }));

        p.allow_private_suffix = false;
        assert!(matches!(classify("foo.github.io", &p), Decision::Search { .. }),
            "when private suffixes are disallowed, treat as Search");
    }

    #[cfg(feature = "real-psl")]
    #[test]
    fn psl_rfc_reserved_internal_domains() {
        // RFC 6761 reserves some domains for internal use
        // These should be treated as known suffixes
        let p = Policy::default();
        assert!(matches!(classify("foo.test", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("foo.example", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("foo.local", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("foo.localhost", &p), Decision::Navigate { .. }));
    }
    #[test]
    fn telephone_number_is_search() {
        let p = policy_default_inet();
        assert!(matches!(
            classify("tel:+123456789", &p),
            Decision::Search { query } if query == "tel:+123456789"
        ));
        assert!(matches!(
            classify("tel:+4123423465", &p),
            Decision::Search { query } if query == "tel:+4123423465"
        ));
        assert!(matches!(
            classify("912345678", &p),
            Decision::Search { query } if query == "912345678"
        ));
        assert!(matches!(
            classify("+351 912 345 678", &p),
            Decision::Search { query } if query == "+351 912 345 678"
        ));
    }

    #[cfg(feature = "real-psl")]
    #[test]
    fn mailto_urls_become_search() {
        let p = Policy::default();
        assert!(matches!(classify("mailto:test@google.com", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("mailto:test@yahoo.com", &p), Decision::Navigate { .. }));
        assert!(matches!(classify("mailto:test@hotmail.com", &p), Decision::Navigate { .. }));
    }

    #[test]
    fn ipv6_formats() {
        let p = policy_default_inet();
        
        // IPv6 format variations
        let ipv6_formats = [
            // 1. Standard Full Representation (Canonical)
            // 8 groups of 4 hexadecimal digits, including leading zeros.
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334",

            // 2. Leading Zeros Omitted
            // Zeros at the start of any group are removed.
            "2001:db8:85a3:0:0:8a2e:370:7334",

            // 3. Compressed (Double Colon)
            // Continuous blocks of zeros are replaced by '::'. Used in the middle.
            "2001:db8:85a3::8a2e:370:7334",

            // 4. Leading Compression
            // The address starts with zeros, replaced by '::'.
            "::8a2e:370:7334",

            // 5. Trailing Compression
            // The address ends with zeros, replaced by '::'.
            "2001:db8:85a3::",

            // 6. Unspecified Address
            // Represents 0.0.0.0 in IPv6 (absence of an address).
            "::",

            // 7. Loopback Address (Compressed)
            // Represents localhost (127.0.0.1).
            "::1",

            // 8. Loopback Address (Full)
            // The non-compressed version of localhost.
            "0000:0000:0000:0000:0000:0000:0000:0001",

            // 9. IPv4-Mapped IPv6 Address
            // Used by dual-stack software; the last 32 bits are decimal.
            "::ffff:192.168.1.1",

            // 10. IPv4-Compatible IPv6 Address (Deprecated)
            // Older format, rarely used now, but syntactically valid.
            "::192.168.1.1",

            // Currently not supported

            // 11. Link-Local with Zone ID (Linux/Unix)
            // Includes the '%' separator and the interface name (Scope ID).
            //"fe80::1ff:fe23:4567:890a%eth0",

            // 12. Link-Local with Zone ID (Windows)
            // Includes the '%' separator and the numeric interface index.
            //"fe80::1ff:fe23:4567:890a%3",

            // 13. CIDR Notation (Network Prefix)
            // Address followed by '/' and the routing prefix length.
            //"2001:db8:abcd:0012::0/64",
        ];

        // Test cases that are currently not supported
        let disabled_test_cases = vec![
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
            "2001:db8:85a3:0:0:8a2e:370:7334",
            "::1",
            "0000:0000:0000:0000:0000:0000:0000:0001",
        ];

        // Generate test cases
        let test_cases: Vec<String> = ipv6_formats
            .iter()
            .flat_map(|ip| {
                vec![
                    ip.to_string(),
                    format!("[{ip}]"),
                    format!("[{ip}]:80"),
                    format!("http://[{ip}]"),
                    format!("http://[{ip}]:80"),
                ]
            })
            .collect();

        // Filter out disabled test cases
        let enabled_test_cases: Vec<&String> = test_cases
            .iter()
            .filter(|tc| !disabled_test_cases.contains(&tc.as_str()))
            .collect();

        // Collect all failures instead of stopping at the first one
        let mut failures = Vec::new();
        for test_case in &enabled_test_cases {
            let result = classify(test_case, &p);
            if !matches!(result, Decision::Navigate { .. }) {
                failures.push(format!("  - Input: '{}' -> Result: {:?}", test_case, result));
            }
        }

        // Report all failures at once
        if !failures.is_empty() {
            panic!(
                "IPv6 test failures ({} out of {} cases):\n{}",
                failures.len(),
                enabled_test_cases.len(),
                failures.join("\n")
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_file_paths_with_policy() {
        let test_cases = vec![
            // Basic absolute paths
            ("/etc/test.html", "file:///etc/test.html"),
            ("/etc/test", "file:///etc/test"),
            ("/", "file:///"),
            
            // Paths with special characters (no spaces)
            ("/path/to/file?query=value", "file:///path/to/file%3Fquery=value"),
            ("/path/to/file#anchor", "file:///path/to/file%23anchor"),
            
            // Paths with Unicode characters
            ("/Users/用户/文件.html", "file:///Users/%E7%94%A8%E6%88%B7/%E6%96%87%E4%BB%B6.html"),
            ("/путь/к/файлу.txt", "file:///%D0%BF%D1%83%D1%82%D1%8C/%D0%BA/%D1%84%D0%B0%D0%B9%D0%BB%D1%83.txt"),
            
            // Paths with dots
            ("/path/../other/file.html", "file:///path/../other/file.html"),
            ("/./file.html", "file:///file.html"),
            
            // Paths with encoded characters (treated as literals, not decoded)
            ("/path%20with%20spaces.html", "file:///path%2520with%2520spaces.html"), 
            
            // Paths with spaces
            ("/path with spaces.html", "file:///path%20with%20spaces.html"),
            ("/path with spaces/file.html", "file:///path%20with%20spaces/file.html"),
        ];

        for (input, expected) in &test_cases {
            let mut p = Policy::default();
            p.allow_file_paths = false;
            
            let result = classify(input, &p);
            assert!(matches!(result, Decision::Search { .. }),
                "Expected Search for '{}' when allow_file_paths=false, got {:?}", input, result);
            
            p.allow_file_paths = true;
            
            let result = classify(input, &p);
            match result {
                Decision::Navigate { ref url } => {
                    assert_eq!(url, expected,
                        "Expected '{}' for input '{}', got '{}'", expected, input, url);
                },
                Decision::Search { ref query } => {
                    panic!("Expected Navigate for '{}' when allow_file_paths=true, got Search with query '{}'", input, query);
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_file_paths_with_policy() {
        let test_cases = vec![
            // Basic drive paths with backslashes
            (r"C:\foo\bar.html", "file:///C:/foo/bar.html"),
            (r"c:\foo\bar.html", "file:///C:/foo/bar.html"),
            
            // Drive paths with forward slashes
            ("C:/foo/bar.html", "file:///C:/foo/bar.html"),
            ("c:/foo/bar.html", "file:///C:/foo/bar.html"),
            
            // UNC paths (only with backslashes, forward slashes are scheme-relative URLs)
            (r"\\foo\bar.html", "file://foo/bar.html"),
            (r"\\server\share\file.txt", "file://server/share/file.txt"),
            
            // Paths with special characters
            (r"C:\path\file?query=1", "file:///C:/path/file%3Fquery=1"),
            (r"C:\path\file#anchor", "file:///C:/path/file%23anchor"),
            
            // Paths with Unicode characters
            (r"C:\Users\用户\文件.html", "file:///C:/Users/%E7%94%A8%E6%88%B7/%E6%96%87%E4%BB%B6.html"),
            (r"C:\путь\к\файлу.txt", "file:///C:/%D0%BF%D1%83%D1%82%D1%8C/%D0%BA/%D1%84%D0%B0%D0%B9%D0%BB%D1%83.txt"),
            
            // Different drive letters
            (r"D:\data\file.txt", "file:///D:/data/file.txt"),
            (r"E:\backup\archive.zip", "file:///E:/backup/archive.zip"),
            (r"Z:\network\share.doc", "file:///Z:/network/share.doc"),
            
            // Root drive path (exactly 3 characters - edge case)
            (r"C:\", "file:///C:/"),
            (r"c:\", "file:///C:/"),
            
            // Mixed slashes
            (r"C:\foo/bar\baz.html", "file:///C:/foo/bar/baz.html"),
            (r"\\server/share\file.html", "file://server/share/file.html"),
            
            // Paths with dots
            (r"C:\path\..\other\file.html", "file:///C:/path/../other/file.html"),
            (r"C:\.\file.html", "file:///C:/file.html"),
            
            // Paths with encoded characters (treated as literals)
            (r"C:\path%20with%20spaces.html", "file:///C:/path%2520with%2520spaces.html"),

            // Paths with spaces
            (r"C:\path with spaces.html", "file:///C:/path%20with%20spaces.html"),
            (r"C:\path with spaces\file.html", "file:///C:/path%20with%20spaces/file.html")
        ];

        for (input, expected) in &test_cases {
            let mut p = Policy::default();
            p.allow_file_paths = false;
            
            let result = classify(input, &p);
            assert!(matches!(result, Decision::Search { .. }),
                "Expected Search for '{}' when allow_file_paths=false, got {:?}", input, result);
            
            p.allow_file_paths = true;
            
            let result = classify(input, &p);
            match result {
                Decision::Navigate { ref url } => {
                    assert_eq!(url, expected,
                        "Expected '{}' for input '{}', got '{}'", expected, input, url);
                },
                Decision::Search { ref query } => {
                    panic!("Expected Navigate for '{}' when allow_file_paths=true, got Search with query '{}'", input, query);
                }
            }
        }
    }
}

