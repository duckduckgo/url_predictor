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

// -----------------------------------------------------------------------------
// C FFI
// -----------------------------------------------------------------------------

#[unsafe(no_mangle)]
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

#[unsafe(no_mangle)]
pub extern "C" fn ddg_up_free_string(ptr: *mut c_char) {
    if ptr.is_null() { return; }
    unsafe { let _ = CString::from_raw(ptr); }
}

// -----------------------------------------------------------------------------
// JNI (Android only)
// -----------------------------------------------------------------------------
#[cfg(target_os = "android")]
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

}

