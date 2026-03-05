# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build                          # default build (uses DemoSuffixDb)
cargo build --features real-psl      # build with real Public Suffix List
cargo test                           # run all tests
cargo test --features real-psl       # run tests with real PSL
cargo test <test_name>               # run a single test by name
```

Android unit tests:
```sh
cargo build --release --features "real-psl jni-host-tests"
cd android && ./gradlew :ddg-url-predictor:testDebugUnitTest
```

## Architecture

Everything lives in `src/lib.rs` (intentionally single-module). The entry points are:

- `classify(input, policy)` — public Rust API, uses the global `DEFAULT_SUFFIX_DB`
- `classify_with_db(input, policy, db)` — same but accepts an explicit `SuffixDb` (used in tests)
- `ddg_up_classify_json(input_ptr, policy_ptr)` — C FFI wrapper, returns heap-allocated JSON string; caller must free with `ddg_up_free_string`

### Classification flow (`classify_with_db`)

1. **Reject embedded newlines/tabs** → `Search` (security fix: url crate silently strips these, which would misclassify `https://bbc.com\nfoo` as Navigate)
2. **Absolute URL with known scheme** → `Navigate`
3. **Absolute URL with unknown scheme** → `Search` with `unknown_scheme_navigation` populated
4. **Scheme-relative `//host`** → `Navigate`
5. **File paths** (if `policy.allow_file_paths`) → `Navigate`
6. **Multi-word input** → `Search`
7. **Host-like classification** → checks IPs, localhost, IDNA hostnames, PSL lookup, intranet rules

### Key types

- `Decision` — `Navigate { url }` or `Search { query, unknown_scheme_navigation? }`
- `Policy` — controls intranet, private suffixes, allowed schemes, file paths
- `SuffixDb` trait — abstraction over PSL backends
  - `DemoSuffixDb` — tiny hardcoded set for tests/default builds
  - `RealSuffixDb` — full PSL via `publicsuffix` crate (feature `real-psl`), reads from `assets/public_suffix_list.dat`

### Generated code

`src/generated_suffix_allowlist.rs` contains `ALWAYS_NAVIGATE_SUFFIX_ROOTS` — a sorted array of suffix roots (e.g. `blogspot.com`) that should always be treated as Navigate even though they're public suffixes. Regenerate it with:

```sh
python tools/generate_suffix_root_allowlist.py \
  --psl assets/public_suffix_list.dat \
  --rust-out src/generated_suffix_allowlist.rs \
  --json-out data/suffix_root_allowlist.json \
  --debug-out data/suffix_root_debug.json
```

This script probes the network, so it's slow. Run it only after updating `assets/public_suffix_list.dat` via `./scripts/update_psl.sh`.

### PSL FFI

When built with `real-psl`, the library also exposes `ddg_up_get_psl_ptr()` / `ddg_up_get_psl_len()` for zero-copy access to the embedded PSL data from native callers (iOS/Windows). Do NOT free that pointer.

### Platform builds

Scripts under `scripts/` cross-compile for Android (`.so`), iOS/macOS (`.a`/`.dylib`), and Windows (`.dll`). Output goes to `dist/` (not checked in).

### Features

- `real-psl` — enables `RealSuffixDb` backed by the full Public Suffix List
- `jni-host-tests` — enables JNI bindings for running Android tests on the host JVM

## Releasing (Android)

See `android/RELEASING.md` or use `android/release_android.sh`. Always do a dry run first:

```sh
cd android && ./release_android.sh --new-version X.Y.Z --dry-run
```

The `.so` binaries in `android/ddg-url-predictor/src/main/jniLibs` are checked in and must be rebuilt via `scripts/build_android.sh` and committed before releasing if there are any Rust changes.
