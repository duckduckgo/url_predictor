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
    pub allow_intranet_single_label: bool,
    pub allow_private_suffix: bool,
    pub allowed_schemes: BTreeSet<String>,
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
    Search { query: String },
}
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
