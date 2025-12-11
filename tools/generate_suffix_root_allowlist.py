#!/usr/bin/env python3
"""
Generate an allowlist of public-suffix "root" domains that appear to be
real HTML pages, suitable for treating as "always navigate" candidates
in a URL predictor.

Now supports generating a Rust module directly:

    pub const ALWAYS_NAVIGATE_SUFFIX_ROOTS: &[&str] = &["blogspot.com", ...];

Usage examples:

    # Generate Rust file only
    python tools/generate_suffix_root_allowlist.py \
        --psl public_suffix_list.dat \
        --rust-out src/generated_suffix_allowlist.rs

    # Generate both Rust + JSON + debug info
    python tools/generate_suffix_root_allowlist.py \
        --psl public_suffix_list.dat \
        --rust-out src/generated_suffix_allowlist.rs \
        --json-out data/suffix_root_allowlist.json \
        --debug-out data/suffix_root_debug.json
"""

import argparse
import concurrent.futures
import json
import sys
import time
from dataclasses import dataclass
from typing import Iterable, List, Optional, Set, Tuple

import requests

DEFAULT_TIMEOUT = 3.0
DEFAULT_MAX_WORKERS = 16


@dataclass(frozen=True)
class Candidate:
    suffix: str        # PSL entry as-is (e.g., "blogspot.com", "*.hosted.app")
    root_domain: str   # Derived (e.g., "blogspot.com", "hosted.app")


def parse_psl(psl_text: str) -> List[Candidate]:
    candidates: List[Candidate] = []

    for line in psl_text.splitlines():
        line = line.strip()
        if not line:
            continue
        if line.startswith("//"):
            continue
        if line.startswith("!"):
            continue

        # Only multi-label suffixes
        if "." not in line:
            continue

        if line.startswith("*."):
            root = line[2:]
        else:
            root = line

        if " " in root:
            continue
        if root.count(".") < 1:
            continue

        candidates.append(Candidate(suffix=line, root_domain=root))

    # Deduplicate by root domain
    unique = {}
    for c in candidates:
        key = c.root_domain.lower()
        if key not in unique:
            unique[key] = c

    return list(unique.values())


def looks_like_html(resp: requests.Response, body: bytes) -> bool:
    ctype = resp.headers.get("Content-Type", "").lower()
    if "text/html" in ctype or "application/xhtml+xml" in ctype:
        return True

    snippet = body[:2048].lstrip().lower()
    if snippet.startswith(b"<!doctype html") or snippet.startswith(b"<html"):
        return True

    return False


def check_domain(domain: str) -> Tuple[str, bool, Optional[str]]:
    urls = [f"https://{domain}", f"http://{domain}"]
    session = requests.Session()
    session.headers["User-Agent"] = (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/125.0.0.0 Safari/537.36"
    )   

    for url in urls:
        try:
            resp = session.get(url, timeout=DEFAULT_TIMEOUT, allow_redirects=True)
        except requests.RequestException as e:
            return domain, False, f"{url}: error {e}"

        if 200 <= resp.status_code < 300:
            body = resp.content
            if looks_like_html(resp, body):
                return domain, True, f"{url}: {resp.status_code} HTML"
            else:
                return domain, False, f"{url}: {resp.status_code} non-HTML"
        else:
            return domain, False, f"{url}: status {resp.status_code}"

    return domain, False, "unreachable"


def generate_allowlist(
    candidates: Iterable[Candidate],
    max_workers: int,
    limit: Optional[int] = None,
) -> Tuple[Set[str], dict]:
    roots = [c.root_domain.lower() for c in candidates]
    if limit is not None:
        roots = roots[:limit]

    allowlist: Set[str] = set()
    info: dict[str, str] = {}

    total = len(roots)
    print(f"Checking {total} candidate domains with up to {max_workers} workers...")

    with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as executor:
        futures = {
            executor.submit(check_domain, domain): domain for domain in roots
        }

        done = 0
        last_print = time.time()

        for future in concurrent.futures.as_completed(futures):
            domain, is_html, msg = future.result()
            done += 1
            if msg:
                info[domain] = msg
            if is_html:
                allowlist.add(domain)

            now = time.time()
            if now - last_print > 2.0:
                print(f"  Progress: {done}/{total} ({done * 100.0 / total:.1f}%)")
                last_print = now

    return allowlist, info

def to_idna_ascii(domain: str) -> str:
    # Convert Unicode domain to IDNA ASCII (punycode) so it matches Rust IDNA
    try:
        return domain.encode("idna").decode("ascii").lower()
    except UnicodeError:
        # If somehow encoding fails, fall back to lowercase unicode
        return domain.lower()


def write_rust_allowlist(domains: Set[str], path: str) -> None:
    """Write a Rust module with ALWAYS_NAVIGATE_SUFFIX_ROOTS in IDNA ASCII."""
    ascii_domains = {to_idna_ascii(d) for d in domains}
    sorted_domains = sorted(ascii_domains)

    lines = []
    lines.append("// @generated by generate_suffix_root_allowlist.py\n")
    lines.append("// Do not edit by hand.\n\n")
    lines.append("pub const ALWAYS_NAVIGATE_SUFFIX_ROOTS: &[&str] = &[\n")
    for d in sorted_domains:
        lines.append(f"    \"{d}\",\n")
    lines.append("];\n")

    with open(path, "w", encoding="utf-8") as f:
        f.writelines(lines)

    print(f"Wrote Rust allowlist to {path}")

def old_write_rust_allowlist(domains: Set[str], path: str) -> None:
    """Write a Rust module with ALWAYS_NAVIGATE_SUFFIX_ROOTS."""
    sorted_domains = sorted(d.lower() for d in domains)

    lines = []
    lines.append("// @generated by generate_suffix_root_allowlist.py\n")
    lines.append("// Do not edit by hand.\n\n")
    lines.append("pub const ALWAYS_NAVIGATE_SUFFIX_ROOTS: &[&str] = &[\n")
    for d in sorted_domains:
        lines.append(f"    \"{d}\",\n")
    lines.append("];\n")

    with open(path, "w", encoding="utf-8") as f:
        f.writelines(lines)

    print(f"Wrote Rust allowlist to {path}")


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Generate allowlist of public-suffix root domains that look like HTML pages"
    )
    parser.add_argument("--psl", required=True, help="Path to public_suffix_list.dat")
    parser.add_argument("--rust-out", help="Output Rust file (generated module)")
    parser.add_argument("--json-out", help="Optional output JSON file (allowlist)")
    parser.add_argument("--debug-out", help="Optional debug JSON (domain -> message)")
    parser.add_argument("--max-workers", type=int, default=DEFAULT_MAX_WORKERS)
    parser.add_argument("--limit", type=int, default=None)

    args = parser.parse_args(argv)

    if not args.rust_out and not args.json_out:
        print("Error: at least one of --rust-out or --json-out must be provided", file=sys.stderr)
        return 1

    with open(args.psl, "r", encoding="utf-8") as f:
        psl_text = f.read()

    candidates = parse_psl(psl_text)
    print(f"Found {len(candidates)} unique PSL multi-label candidates.")

    allowlist, info = generate_allowlist(
        candidates,
        max_workers=args.max_workers,
        limit=args.limit,
    )

    print(f"\nHTML-like domains found: {len(allowlist)}")

    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as f:
            json.dump(sorted(list(allowlist)), f, indent=2)
        print(f"Wrote JSON allowlist to {args.json_out}")

    if args.debug_out:
        with open(args.debug_out, "w", encoding="utf-8") as f:
            json.dump(info, f, indent=2)
        print(f"Wrote debug info to {args.debug_out}")

    if args.rust_out:
        write_rust_allowlist(allowlist, args.rust_out)

    return 0


if __name__ == "__main__":
    sys.exit(main())

