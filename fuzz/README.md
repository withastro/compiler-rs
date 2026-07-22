# Fuzz Testing

Fuzz targets for the Astro compiler using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer).

## Targets

| Target | What it checks |
|--------|---------------|
| `transform_no_panic` | `transform()` never panics on arbitrary input |
| `css_scope_no_panic` | `scope_css()` never panics on arbitrary input |
| `transform_valid_js_output` | valid Astro input → compiler output is valid JS |

## Prerequisites

cargo-fuzz requires nightly Rust:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Running locally

```sh
# Run a target (Ctrl-C to stop)
cargo +nightly fuzz run transform_no_panic

# Run with a memory limit (default is unbounded)
cargo +nightly fuzz run transform_no_panic -- -rss_limit_mb=4096

# Run for a fixed time (seconds)
cargo +nightly fuzz run transform_no_panic -- -max_total_time=300

# Run with increased verbosity to see coverage progress
cargo +nightly fuzz run transform_no_panic -- -print_final_stats=1
```

When a crash or OOM is found, the input is saved to `fuzz/artifacts/<target>/`.

## Minimizing a crash

`cargo fuzz tmin` reduces a crashing input to the smallest input that still
triggers the same crash. This makes it easier to understand the root cause and
write a clear bug report.

```sh
# Minimize a crash artifact
cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/<artifact>

# Example: minimize a transform_no_panic OOM
cargo +nightly fuzz tmin transform_no_panic \
    fuzz/artifacts/transform_no_panic/oom-900cd3cf6b6223c1f8772aef650e8f908b043084

# The minimized input is written to:
#   fuzz/artifacts/<target>/minimized-from-<original-hash>
```

Tips:
- tmin works by repeatedly trying to remove bytes or replace characters while
  checking that the crash still reproduces. It may take a few minutes.
- If tmin gets stuck at a large size, try running it again — libFuzzer uses
  randomness and a second pass sometimes finds a shorter path.
- For OOM crashes, tmin tries to find the smallest input that hits the same
  allocation limit, not necessarily the exact same code path.
- After minimization, inspect the artifact with `xxd` or `cat` to understand
  what the fuzzer found:
  ```sh
  xxd fuzz/artifacts/<target>/minimized-from-<hash>
  # or, if it's valid UTF-8:
  cat fuzz/artifacts/<target>/minimized-from-<hash>
  ```

## Reproducing a crash

To confirm a specific artifact still crashes the target:

```sh
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<artifact>
```

libFuzzer will run the input once, report the crash, and exit.

## CI

Fuzzing in CI runs through [ClusterFuzzLite](https://google.github.io/clusterfuzzlite/)
(`.clusterfuzzlite/` + the `cflite_*.yml` workflows):

- **PR fuzzing** (`cflite_pr.yml`) — code-change mode on PRs touching compiler
  code. Paired with continuous builds it suppresses pre-existing crashes, so
  only bugs introduced by the PR fail the check.
- **Continuous builds** (`cflite_build.yml`) — uploads a fuzzer build per push
  to `main`; this is what PR fuzzing diffs crashes against.
- **Batch fuzzing** (`cflite_batch.yml`) — nightly at 2am UTC, 1 hour, grows
  the corpus. Reports all crashes, including known ones (informational).
- **Corpus pruning + coverage** (`cflite_cron.yml`) — daily.

Crash artifacts and corpora are uploaded as GitHub Actions artifacts.

## Known findings

### 1. Astro parser OOM — `withastro/oxc`

**Target:** `transform_no_panic`, `transform_valid_js_output`
**Minimized input (3 bytes):** `<D}`
**Artifact:** `fuzz/artifacts/transform_no_panic/minimized-from-31fa1c120335eb994219313c5f26a18a09cb6fc6`

The oxc Astro parser (`parse_astro_jsx_expression_container` →
`parse_astro_jsx_element`) has no depth limit on malformed nested JSX. The
input `<D}` triggers unbounded allocation until the OOM killer fires.

**Upstream:** needs a depth limit in the `withastro/oxc` Astro JSX parser.

### 2. lightningcss integer overflow / stack overflow

**Target:** `css_scope_no_panic`
**Minimized input (~525 bytes):** `fuzz/artifacts/css_scope_no_panic/minimized-from-7788f3ffd0ed1cc8c2c58f7ee4435169fa29b8bd`
**Original crash:** `fuzz/artifacts/css_scope_no_panic/crash-7788f3ffd0ed1cc8c2c58f7ee4435169fa29b8bd`

Deeply nested `{` CSS causes a stack overflow in `StyleRule::to_css_base ↔
CssRuleList::to_css` (mutual recursion, 200+ frames on macOS). On Linux CI
the same input surfaces as `attempt to add with overflow`.

**Upstream:** needs recursion depth limiting in the lightningcss printer.
