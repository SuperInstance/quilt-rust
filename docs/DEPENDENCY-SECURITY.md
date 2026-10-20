# Dependency security, and one receipt

This file records one dependency substitution, why it was necessary, and how
it was proved to change nothing observable. It is not a general policy
document; it is the place to look when `cargo audit` goes red and you want
to know whether the fix is available or the ceiling is real.

## `serde_yml` -> `serde_norway`

**What `cargo audit` said**

    error: 1 vulnerability found!
    Crate:     noyalib
    Version:   0.0.5
    Title:     Resource budgets not enforced on the typed deserialization path
    ID:        RUSTSEC-2026-0333
    Solution:  Upgrade to >=0.0.53

**Why it was in the tree**

`noyalib` is not ours. It arrived transitively:

    quilt-core / quilt-cli
      -> serde_yml 0.0.13        (direct workspace dependency)
        -> noyalib 0.0.5         (pinned by serde_yml)

`serde_yml 0.0.13` is the newest release and is itself marked
**DEPRECATED** on crates.io. Its `noyalib` requirement has a ceiling below
0.0.53, so `cargo update -p noyalib --precise 0.0.58` is rejected with
`candidate versions found which didn't match: 0.0.58`. The fix is
genuinely unreachable from below. The other candidate,
`serde_yaml`, is also deprecated (`0.9.34+deprecated`).

So both doors out of the advisory were closed by upstream, and the only
remaining move was to stop using the crate.

**What replaced it**

`serde_norway` — the maintained fork of `serde_yaml`. After the swap:

    grep -c noyalib   Cargo.lock   -> 0
    grep -c serde_yml Cargo.lock   -> 0
    cargo audit                    -> exit 0

The one remaining advisory in the tree is `smartstring 1.0.1`
(RUSTSEC-2026-0249, *unmaintained*), which arrives via `rhai`. It is a
warning, not a vulnerability, and it is allow-listed.

## How the swap was proved to be a no-op

This repository's whole claim is byte-equivalence across tiers, so a YAML
backend swap is not allowed to quietly change a single byte of output.
That is not something a passing test suite establishes on its own — the
suite might simply not cover the shape where the two libraries disagree.

So both backends were built and run over one probe sheet, and the outputs
compared byte for byte. The probe deliberately includes the cases where
YAML libraries disagree:

| case | what it pins |
|---|---|
| `value: 42` | integer does not become `42.0` |
| `value: 40.0` | a real float stays `40.0` and does not collapse to `40` |
| `"quoted: \"yes\"\nnewline\ttab"` | quoting and escaping of structural characters |
| nested `{nested: {list: [1, 2, {deep: true}]}}` | block indentation and sequence nesting |
| `unicode: héllo 🌍 ñ` | non-ASCII quoting policy and block-scalar choice |
| a formula cell | the `kind`/`expr` key pair |

Result:

    md5(old serde_yml output)     = c393bf858d3485a98ecea72cf37bc8b3
    md5(new serde_norway output)  = c393bf858d3485a98ecea72cf37bc8b3
    diff                            -> no output

Identical bytes. Note that `40.0` surviving as `40.0` is the int/float
distinction that `compat/canonical-vectors.json` exists to protect; a YAML
backend that quietly normalised it would have passed a test suite that only
checked round-tripping.

Full receipts for the change:

    cargo build --workspace                       exit 0
    cargo test --workspace                        exit 0, 0 failed
    cargo clippy --workspace --all-targets -D warnings   0 errors
    cargo fmt --all -- --check                    clean
    cargo audit                                   exit 0

## The general lesson

An advisory whose fix is above your dependency's ceiling is not a
configuration problem. `cargo update --precise` failing with
`candidate versions found which didn't match` is the signal that the crate
must be replaced, not that the version number is wrong.

And when the replacement is a parser or serialiser in a project whose
central claim is byte-equivalence, the swap is not done until both
implementations have been run over the disagreeing cases and their bytes
compared. A green test suite after a dependency swap is evidence that
nothing crashed, not evidence that nothing changed.
