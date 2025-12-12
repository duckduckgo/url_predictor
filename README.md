# URL Predictor

A small Rust library that decides whether a piece of text from an address bar should be treated as:

- **Navigate** → open as a URL  
- **Search** → send to the search engine  

The logic is deterministic and tries to stay minimal so it can be audited and reused across platforms.

---

## Why

Browsers and apps often need to guess what the user meant when typing into the omnibox. For example:

- `example.com` → Navigate  
- `what is my ip` → Search  

This crate provides a reference implementation with a configurable `Policy` struct so each app can tweak behavior.

---

## Features

- Distinguishes between **Navigate / Search
- Handles schemes (`https://`, `ftp://`, `edge://`, etc.)
- Understands hostnames, localhost, IPs, intranet single-labels
- Optional integration with the [Public Suffix List](https://publicsuffix.org/) (via the `real-psl` feature)
- Cross-platform FFI (Android/iOS/Windows)

---

## Policy

Behavior is tuned via a `Policy`:

```rust
#[derive(Serialize, Deserialize)]
pub struct Policy {
    pub allow_intranet_multi_label: bool,
    pub allow_intranet_single_label: bool,
    pub allow_private_suffix: bool,
    pub allowed_schemes: BTreeSet<String>,
    pub allow_file_paths: bool,
}
```

Example (Rust):

```rust
let mut policy = Policy::default();
policy.allow_intranet_single_label = true;

let decision = classify("duck ai translate hello", &policy);
```

---

## Decisions

```rust
pub enum Decision {
    Navigate { url: String },
    Search { 
        query: String,
        unknown_scheme_navigation: Option<String>,
    },
}
```

### `unknown_scheme_navigation`

When the input looks like a valid URL but uses a scheme not in `allowed_schemes`, the `Search` variant includes `unknown_scheme_navigation` with the parsed URL. This lets the caller decide whether to offer navigation as an option.

Examples:
- `tel:+123456789` → `Search { query: "tel:+123456789", unknown_scheme_navigation: Some("tel:+123456789") }`
- `spotify:track:123` → `Search { query: "spotify:track:123", unknown_scheme_navigation: Some("spotify:track:123") }`
- `hello world` → `Search { query: "hello world", unknown_scheme_navigation: None }`

In JSON, the field is omitted when `None`:
```json
{"Search":{"query":"tel:+123456789","unknown_scheme_navigation":"tel:+123456789"}}
{"Search":{"query":"hello world"}}
```

---

## Platform Integration

- **Rust** → use `classify(&str, &Policy)` directly  
- **C/FFI** → call `ddg_up_classify_json`, which returns JSON-encoded `Decision`  
- **Android (JNI)** → `UrlPredictor.classify(input)` in Kotlin  
- **iOS** → expose `ddg_up_classify_json` via a bridging header and wrap it in Swift  
- **Windows** → link against the `.dll` and call `ddg_up_classify_json`

### Memory management

`ddg_up_classify_json` returns a heap-allocated string.  
Call `ddg_up_free_string(ptr)` once you’re done with it to avoid memory leaks.

Example in C:

```c
char* result = ddg_up_classify_json(input, policy_json);
printf("%s\n", result);
ddg_up_free_string(result); // free it!
```

---

### Accessing the Public Suffix List (PSL) via FFI

When built with `--features real-psl`, the library exposes a **zero-copy** API:

C (no copy):
```c
const char *p = ddg_up_get_psl_ptr();
size_t len    = ddg_up_get_psl_len();
/* Use as a read-only buffer of length `len`, or as a C string (NUL at p[len]). */
/* Do NOT free(p). Memory lives for the process lifetime. */

```

Swift (no copy):
```swift
let p = ddg_up_get_psl_ptr()
let n = ddg_up_get_psl_len()
let data = Data(bytesNoCopy: UnsafeMutableRawPointer(mutating: p),
                count: Int(n),
                deallocator: .none) // do not free
let text = String(data: data, encoding: .utf8)!
```

## Building

Default (uses a small demo suffix DB):

```sh
cargo build
```

With real PSL:

```sh
cargo build --features real-psl
```
---

## Building for Platforms

This repo ships helper scripts under [`scripts/`](scripts/) to cross-compile the Rust core into
platform-specific libraries:

- **Android:** `scripts/build-android.sh`  
  Produces `liburl_predictor.so` for all standard ABIs and drops them under `android/ddg-url-predictor/src/main/jniLibs`.

- **iOS/macOS:** `scripts/build-ios.sh`, `scripts/build-macos.sh`  
  Produces `.a` / `.dylib` artifacts for Xcode integration. Requires full Xcode installation (`xcode-select`).

- **Windows:** `scripts/build-windows.sh`  
  Produces `url_predictor.dll` for MSVC targets.

Outputs land under `dist/` by default. These aren’t checked into git — run the scripts yourself.

---

## Tests

Run the Rust test suite:

```sh
cargo test
```
Or run tests using the real PSL:

```sh
cargo test --features real-psl
```

To run the Android unit tests:
```
cargo build --release --features "real-psl jni-host-tests"
cd android
./gradlew :ddg-url-predictor:testDebugUnitTest
```

---

## Updating the Public Suffix List

Fetch the latest PSL copy (writes to `assets/public_suffix_list.dat`):

```sh
./scripts/update_psl.sh
```

After updating the PSL you should regenerate the Root Allowlist Generator. See next section.

## Suffix Root Allowlist Generator

The URL predictor keeps a list of public-suffix roots that should always count as “navigate” candidates (e.g., `blogspot.com`). That list lives in `src/generated_suffix_allowlist.rs` as `ALWAYS_NAVIGATE_SUFFIX_ROOTS` and is produced by `tools/generate_suffix_root_allowlist.py`.

What the script does:
- Reads a `public_suffix_list.dat` (the repo ships one at `assets/public_suffix_list.dat`).
- For each multi-label suffix root, probes `https://` / `http://` and keeps domains that return HTML. (This touches the network and can take a while.)
- Writes the allowlist as Rust plus optional JSON/debug artifacts.

Typical regeneration (updates the Rust module, a JSON copy, and per-domain diagnostics):

```sh
python tools/generate_suffix_root_allowlist.py \
  --psl assets/public_suffix_list.dat \
  --rust-out src/generated_suffix_allowlist.rs \
  --json-out data/suffix_root_allowlist.json \
  --debug-out data/suffix_root_debug.json
```

Outputs:
- `src/generated_suffix_allowlist.rs`: Rust module consumed at runtime.
- `data/suffix_root_allowlist.json`: plain list of allowed roots (helpful for inspection).
- `data/suffix_root_debug.json`: map of every checked domain to its HTTP result for troubleshooting.

Use `--max-workers` to tune parallelism or `--limit` for quick spot checks while iterating.

---

## Notes

- The included `DemoSuffixDb` is intentionally tiny. For production, enable the `real-psl` feature and ship a PSL file.  
- The project does not do DNS or network lookups. Everything is local and deterministic.  
- Error cases (like bad policy JSON) fall back to `Policy::default()`.

---

## Example (Kotlin wrapper)

If you’re calling from Android, the Kotlin helper wraps the JNI call and returns a type-safe `Decision`:

```kotlin
object UrlPredictor {
    init { System.loadLibrary("url_predictor") }

    // JNI call into Rust
    @JvmStatic private external fun ddgClassifyJni(input: String, policyJson: String): String

    // Type-safe API
    @JvmStatic
    fun classify(input: String, policy: DecisionJson.Policy = DecisionJson.Policy()): Decision {
        val policyJson = DecisionJson.encodePolicy(policy)
        val decisionJson = ddgClassifyJni(input, policyJson)
        return DecisionJson.decodeDecision(decisionJson)
    }
}
```

Usage:

```kotlin
val result = UrlPredictor.classify("duck ai hello world")
when (result) {
    is Decision.Navigate -> println("Navigate to ${result.url}")
    is Decision.Search -> println("Search for ${result.query}")
}
```
