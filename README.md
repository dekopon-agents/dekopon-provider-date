# dekopon-provider-date

A bounded `date` command for Dekopon agents. It reads the **broker's fresh clock**, not the
model's recollection, machine environment, or a timestamp supplied by the caller.

Requires the broker clock import `dekopon:clock/wall@1.0.0`; minimum tested host is **0.18.0**.
Guest SDK and clock bindings, native broker conformance, and shell tests pin published 0.18.0.
The component exports provider-cli 0.3.0. Version 0.2.0 uses SDK 0.18.0 with unchanged command behavior.

## Command

```sh
date                                             # UTC RFC3339: 2024-01-01T02:03:04Z
date -u +%F                                      # 2024-01-01
date --timezone America/New_York '+%F %H:%M %Z'    # 2023-12-31 21:03 EST
today=$(date --timezone America/New_York +%F)
week_ago=$(date --timezone America/New_York --days=-7 +%F)
date --days +7 +%F
date +%s                                         # Unix seconds, decimal text
date --help
```

Examples show one illustrative instant, not a fixed clock. Arguments are already shell-tokenized;
quote formats containing spaces. One optional `+FORMAT` may appear before or after options.
`-u` and `--utc` select UTC; `--timezone NAME` / `--timezone=NAME` select a case-sensitive IANA
zone. Repeated timezone or day options, conflicting UTC/zone options, extra arguments, unknown
options and missing values fail with status 2. Stdin is ignored. `--help` must stand alone and
prints without authorization or clock access.

Supported strftime conversions (Chrono performs calendar conversion and formatting):

| Conversion | Meaning |
| --- | --- |
| `%F` | ISO calendar date (`%Y-%m-%d`) |
| `%Y`, `%m`, `%d` | Four-digit year, two-digit month and day |
| `%H`, `%M`, `%S` | Two-digit 24-hour hour, minute and second |
| `%z` | Numeric UTC offset, e.g. `-0500`; historical offset seconds round to nearest minute |
| `%Z` | Timezone abbreviation, e.g. `EST` or `EDT` (not a unique zone identifier) |
| `%s` | Unix seconds of the resulting instant, floored to seconds |
| `%%` | Literal percent |

Other conversions, flags, field widths and modifiers are rejected, not guessed. Formats are
0–256 printable ASCII bytes (spaces allowed; control characters and Unicode rejected).
An empty `+` prints an empty line. Successful invocation returns a **JSON string with no newline**;
Dekopon's shell prints it verbatim with one newline and command substitution removes the line
terminator. It is not an object or JSON number. Default rendering is RFC3339 to whole seconds,
with `Z` at zero offset. Fractional milliseconds are not printed.

## Local calendar days and DST

`--days N` / `--days=N` accepts an integer in **-36600..36600** (roughly ±100 years). It changes
the date in the chosen timezone while preserving local hour/minute/second/millisecond. It does
**not** add `N * 86400` elapsed seconds: noon across a DST transition can be 23 or 25 hours apart.
Calendar handling is proleptic Gregorian, provided by Chrono. IANA rules are bundled by
`chrono-tz` 0.10.4 (IANA 2025b); updating timezone legislation requires a dependency update and provider release,
not access to system zoneinfo.

For **nonzero** offsets, a destination in a DST fold (ambiguous) or gap (nonexistent, including a
skipped civil day) fails with `ambiguous-local-time` or `nonexistent-local-time`. This applies even
with `+%F`: the provider never silently changes the target wall time. Zero preserves the original
instant, including either side of a fold. Host milliseconds beyond 9999-12-31T23:59:59.999Z fail;
resulting local and UTC years must both be 0001..9999. Negative offsets can produce pre-epoch dates
and negative `%s`. Overflow is checked and fails, not wrapped.

Historical zones can have second-resolution offsets. Default RFC3339 rejects those with
`unsupported-offset` rather than printing a timestamp that represents a different instant.
Explicit `%z` follows Chrono's conventional nearest-minute rounding (ties away from zero),
so it cannot preserve those historical seconds. `+%F` and `+%s` remain usable without rounding
calendar dates or Unix seconds. `%+` and other alternative RFC3339 conversions are not supported.

No GNU/BSD date compatibility, natural-language parsing, `-d`, environment `TZ`, locale, date
setting, arbitrary timestamps, HTTP, storage, files, secrets, or network access is implemented.

## Authority and bounds

Manifest: provider `date`, word `date`, sole capability **`clock.now`**, read-only / low risk.
`describe` and `run-command` are pure; only validated `invoke` reads the host clock, exactly once.
Every invocation is fresh. The command proposes; it does not authorize. Direct capability input
uses the same enforcing typed validation as argv, not merely the model-facing schema:

```json
{"timezone":"America/New_York","days":-7,"format":"%F"}
```

All fields are optional (UTC, zero days, RFC3339 defaults); unknown fields, wrong types and invalid
formats/zones fail. Bounds: 8 argv entries, 1024 total argv bytes, 64 timezone bytes, 256 format
bytes, 2048 rendered bytes. Errors never echo arbitrary argv. The host must also cap serialized
input/output, memory, fuel and time before the SDK parses untrusted wire input.

An operator may adapt the exact-principal [Cedar grant](examples/date.cedar), validated by the
native broker test. Pair it with an exact `clock.now` constraint set owned by provider `date`:

```yaml
clock.now:
  provider: date
  effect: read-only
  risk: Low
  constraints:
    timeoutMs: 10000
    maxOutputBytes: 4096
```

This is the capability entry, not an entire broker configuration. No HTTP/storage/secret grant
is needed or appropriate. Recommended host test bounds: 32 MiB memory, 32 million fuel, 4096
input/output bytes, 10 seconds. Agent catalog/session capability exposure must separately include
`clock.now`. No runtime configuration or deployment is performed by this repository.

## Build and validation

Install the exact `rust-toolchain.toml` toolchain and the tools pinned in
[provider-workflows](https://github.com/dekopon-agents/provider-workflows). From this repository:

```sh
cargo fmt --all --check
cargo test --locked --lib                  # pure injected-clock tests
../provider-workflows/build.sh             # reproducible component
DEKOPON_PROVIDER_COMPONENT="$PWD/date-provider.wasm" cargo test --locked --workspace
```

Integration tests **fail**, never skip, without `DEKOPON_PROVIDER_COMPONENT`. Tests cover UTC/IANA
rendering, boundaries, DST and range failures; an empty-linker refusal and fixed-clock component
plus real shell substitutions; and the published native 0.18.0 broker with Cedar allow/deny and
`provider_clock_read` trace evidence only inside invoke. Native formatter tests alone do not prove
the host import works.

## CI and release provenance

CI and release are minimal callers of `dekopon-agents/provider-workflows` at `@main`. No pipeline
is duplicated here.
Shared CI checks formatting, dependency policy, native/Wasm lint, byte-identical SDK WIT mirrors,
component imports, native tests, SBOM, checksum, and independently rebuilt identical bytes.

After independent review and an explicitly authorized release, shared release verifies an annotated
`vVERSION` tag's version/main ancestry, rebuilds/tests, attests the component and SBOM with GitHub
OIDC, publishes GitHub assets and the identical layer to `ghcr.io/dekopon-agents/provider-date`,
and verifies public provenance. Expected signer workflow:
`dekopon-agents/provider-workflows/.github/workflows/release.yml`, **not** the thin caller.
Verify with `gh attestation verify --repo dekopon-agents/dekopon-provider-date`, pinning that signer,
the release source ref/digest and denying self-hosted runners as in the shared recipe.

Release permissions: contents/packages/attestations write and id-token write. This component is `publish = false`: no crates.io publication/trusted publisher is
needed.

To release, commit the version bump to `main`, then push an annotated `vX.Y.Z` tag on that commit.
The tag triggers the shared release workflow.

## License

MIT OR Apache-2.0.
