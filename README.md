# cargo-regime-check

**Is this change a refactor, or a redesign? Stop guessing — decide it.**

`cargo-regime-check` gates a Rust crate's public-API diff against a *declared
transition* you write in `regime-transition.toml`. Declare the renames/moves you
intend (the functor `u`), subtract them from a `cargo public-api diff`, and any
**residual** — added, removed, or changed surface your renames don't account for —
must be either declared or it fails the gate.

- `meta.kind = "refactor"`  → the API may only be **renamed/moved**. *Any* residual
  fails. A "refactor" with residual > 0 is a **falsified claim**.
- `meta.kind = "transition"` → residual is allowed, but **every** residual line must
  be declared (`[[additive]]` / `[[removal]]` / `[[change]]`) with an ADR or reason.

It is built for **AI agents in CI**: deterministic JSON, exit codes you branch on,
and — on failure — a copy-pasteable `regime-transition.toml` snippet per line that,
applied verbatim, makes the gate pass. See [`AGENTS.md`](./AGENTS.md).

```console
$ cargo +nightly public-api -p kvstore diff origin/main..HEAD \
    | cargo-regime-check --regime regime-transition.toml   # kind="refactor", only renames declared
regime-check: refactor (residual must be 0)
  20 item(s) — 8 accounted, 12 residual

RESIDUAL — 12 line(s) need action:
  ✗ pub trait kvstore::Reader
      → undeclared added surface. If intentional: append the [[additive]] block below
        and ensure meta.kind = "transition". If accidental: make it pub(crate) or remove it.
      fix (append to your regime-transition.toml):
    [[additive]]
    item = "kvstore::Reader"
    adr  = "ADR-XXXX"  # replace with the ADR/issue justifying this new item
  ... (11 more)
verdict: FAIL — 12 undeclared/contradictory line(s). (exit 1)
```

The four method moves are transported by the declared renames (8 lines accounted);
the three *new* traits, their impls, the blanket impl, the old impl, the dropped
helper, and the supertrait/signature changes are the residual a "refactor" can't
explain. Declare them (and flip to `kind = "transition"`) and it's a clean PASS —
see the worked example below.

## Why not just `cargo-semver-checks` / `cargo-public-api`?

They answer *"did the API change, and is it breaking?"* — and they're excellent at
it; this tool **stands on `cargo public-api`'s shoulders** for the typed
normalization. The novel slice is the **declared `u` + residual-must-be-justified
gate**: it certifies *intent*. It mechanically falsifies the sentence "it's just a
refactor" — the same diff PASSes as a declared transition and FAILs as a refactor,
isolating exactly the lines the rename doesn't transport. That is a thing neither
of the other tools does.

## Install

```sh
cargo install --path .          # or: just install
```

The tool builds on **stable**. `cargo public-api` needs **nightly** to emit
rustdoc JSON — it's invoked as a separate process and piped in; this tool never
needs nightly itself.

## Quickstart

```sh
# 1. scaffold a regime file at your crate root
cargo-regime-check --template > regime-transition.toml

# 2. declare your intended renames/additions/removals/changes, then gate:
cargo +nightly public-api -p <crate> diff origin/main..HEAD \
  | cargo-regime-check --regime regime-transition.toml
```

Or with the bundled fixtures:

```sh
cargo-regime-check --regime examples/transition.toml --diff tests/fixtures/demo.diff   # PASS, exit 0
cargo-regime-check --regime examples/refactor.toml  --diff tests/fixtures/demo.diff   # FAIL, exit 1
```

## The canonical case: an Interface-Segregation split

[`examples/kvstore-isp-split.toml`](./examples/kvstore-isp-split.toml) is a **real**
`regime-transition.toml` for splitting a fat `Store` trait into `Reader` / `Writer`
/ `Maintenance` capability traits (the diff is genuine `cargo public-api` output,
captured in
[`tests/fixtures/kvstore-isp-split.diff`](./tests/fixtures/kvstore-isp-split.diff)).
Declared honestly as a transition — the four method renames + the new
traits/impls + the blanket impl + the dropped helper + the supertrait/signature
changes — it PASSes (20/20). Re-claimed as `kind = "refactor"`, it FAILs with 12
Case-C violations. That's the guard: a trait split is *not* a refactor, and the
tool proves it.

## Exit codes

| code | meaning |
|------|---------|
| `0`  | clean — every line transported or declared |
| `1`  | undeclared / contradictory residual (act on stdout) |
| `2`  | usage / I/O / parse error (stdout/stderr says how to fix) |

`--format json` is deterministic and colourless; re-runs are byte-identical. Run
`cargo-regime-check --explain` for the model, `--help` for the full flag set.

## CI

A `justfile` recipe and a GitHub Actions workflow
([`.github/workflows/ci.yml`](./.github/workflows/ci.yml)) show the nightly
invocation. The short version:

```yaml
- run: cargo install cargo-public-api cargo-regime-check --locked
- run: |
    cargo +nightly public-api -p my-crate diff origin/main..HEAD \
      | cargo-regime-check --regime regime-transition.toml --format json
```

## How it works

Hexagonal, domain-pure. `cargo public-api` resolves the API surface into one
normalized token per item and diffs old↔new by identity; this tool parses each
token into a resolved `(kind, path, signature)` identity and classifies every diff
line as transported-by-rename / declared / **residual**, then gates. The category
theory (residual = the new surface outside the image of the declared transport;
Wang & Buehler [arXiv:2606.01444], Spivak's functorial data migration / CQL)
defines the buckets and then leaves the codebase — the implementation is set
arithmetic. Full theory, the agent contract, the known approximations, and the
roadmap are in [`AGENTS.md`](./AGENTS.md).

## Development

```sh
just check        # cargo test + clippy -D warnings + fmt --check
```

No `unsafe`. Domain stays framework-free. Tests follow zero/one/many.

## License

MIT OR Apache-2.0.
