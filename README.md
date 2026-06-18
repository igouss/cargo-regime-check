<div align="center">

# cargo-regime-check

**Is this change a refactor, or a redesign? Stop guessing — decide it.**

A CI gate that subtracts a *declared transition* from a `cargo public-api` diff and
fails on anything you didn't declare. It makes the sentence *"this PR is just a
refactor"* mechanically falsifiable.

[![CI](https://github.com/igouss/cargo-regime-check/actions/workflows/ci.yml/badge.svg)](https://github.com/igouss/cargo-regime-check/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Rust 1.74+](https://img.shields.io/badge/rust-1.74%2B-orange.svg)](#installation)
[![unsafe: forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](#design-philosophy)

```sh
cargo install --git https://github.com/igouss/cargo-regime-check
```

</div>

---

## TL;DR

**The problem.** A PR says *"just a refactor — renamed some things, no API change."* A
reviewer either takes that on faith or reads every line of a `cargo public-api` diff by
hand to check. Both are bad. Faith ships accidental breakage; hand-reading rename churn
is exactly the work humans are worst at.

**The solution.** You write down the changes you *intend* — renames, additions,
removals, signature changes — in a small `regime-transition.toml`. The tool diffs your
crate's public API, subtracts your declared changes, and **anything left over is
residual**. A `kind = "refactor"` allows **zero** residual; a `kind = "transition"`
allows residual only if **every** line of it is declared with an ADR or reason. The gate
names the exact offending line and prints the snippet that fixes it.

It's a **contract for API change**: state what you mean to change, and the tool refuses
to merge anything you didn't state.

| Why use it | What it gives you |
|---|---|
| **Falsifies "it's just a refactor"** | A `refactor` with any undeclared surface delta FAILs — the claim is now checkable, not a vibe |
| **Surfaces the real change under rename churn** | Declared renames are subtracted; only the lines they *don't* explain are shown |
| **Every intentional change is recorded** | Each addition/removal/change carries an ADR/reason, enforced complete |
| **Built for agents in CI** | Deterministic JSON, exit codes you branch on, and a copy-pasteable fix per failing line |
| **No-unsafe, domain-pure, stable Rust** | `unsafe` is `forbid`-en; the gate never needs nightly |

---

## Quick example

Two bundled fixtures, same diff, opposite verdicts. This is the whole idea in ten
seconds:

```console
$ cargo-regime-check --regime examples/transition.toml --diff tests/fixtures/demo.diff
regime-check: transition (residual must be declared)
  5 item(s) — 5 accounted, 0 residual

accounted (no review needed):
  + pub fn demo::brand_new() -> i32   [declared discovery: ADR-0001]
  ~ pub fn demo::new_name(u8) -> u8   [rename (transported)]
  - pub fn demo::doomed()   [declared removal: unused since v1; semver major bump]
  ~ pub fn demo::old_name(u8) -> u8   [rename (transported)]
  ! pub fn demo::widening(u64) -> u64   [declared change: ADR-0002]

verdict: PASS — every line is transported or declared. (exit 0)
```

The **same diff**, re-claimed as a pure refactor that declares only the rename, FAILs —
and tells you exactly what to do about each line:

```console
$ cargo-regime-check --regime examples/refactor.toml --diff tests/fixtures/demo.diff
regime-check: refactor (residual must be 0)
  5 item(s) — 2 accounted, 3 residual

accounted (no review needed):
  ~ pub fn demo::new_name(u8) -> u8   [rename (transported)]
  ~ pub fn demo::old_name(u8) -> u8   [rename (transported)]

RESIDUAL — 3 line(s) need action:

  ✗ pub fn demo::brand_new() -> i32
      → undeclared added surface. If intentional: append the [[additive]] block below
        and ensure meta.kind = "transition". If accidental: make the item pub(crate) or remove it.
      fix (append to your regime-transition.toml):
    [[additive]]
    item = "demo::brand_new"
    adr  = "ADR-XXXX"  # replace with the ADR/issue justifying this new item
  ... (2 more)
verdict: FAIL — 3 undeclared/contradictory line(s). (exit 1)
```

Append the three printed snippets, flip `kind` to `transition`, re-run → clean PASS. An
agent can close that loop with no human in it.

---

## When to use it

| Reach for it when… | Skip it when… |
|---|---|
| You're refactoring a public API and want to **prove** the surface didn't change | The change is purely internal — no `pub` surface delta, nothing to gate |
| A real change is buried in **rename churn** (trait/module splits, moves) and you want only the meaningful lines surfaced | You only need *"is this breaking?"* — `cargo-semver-checks` answers that directly |
| You want every intentional API change **recorded with an ADR/reason**, enforced complete | You can't or won't declare intent up front — the gate certifies *declared* intent, it can't infer it |
| An **AI agent** is making the change and you want a hands-off CI gate it can act on | It's a throwaway crate where API stability doesn't matter |

Use it as a **PR or pre-push gate** on crates whose public API is load-bearing —
published libraries, port/adapter crates other code depends on, anything where *"we
didn't mean to change that"* is expensive.

---

## Design philosophy

1. **Declared, not inferred.** You write `u`, the functor of intended change. The tool
   *certifies* that the diff stays inside it; it does not *guess* your renames. A wrong
   `u` is your bug, not the gate's — and an honest one a reviewer can read. (Rename
   inference is on the roadmap; today the contract is "certify what you declared.")
2. **Fail closed.** When the tool can't account for a line, it FAILs and tells you why.
   It never silently transports an ambiguous match into "looks fine" — a false green is
   the one outcome worse than a false red.
3. **Deterministic and agent-first.** `--format json` is colourless and byte-identical
   across re-runs. Branch on the exit code; act on stdout. No prompts, no spinners, no
   timestamps. See [`AGENTS.md`](./AGENTS.md).
4. **Stand on `cargo public-api`'s shoulders.** The hard part — resolving a crate's
   public surface into normalized, comparable tokens — is solved. This tool consumes
   that output and adds the one slice nobody else does: the declared-`u` residual gate.
5. **Domain-pure, no `unsafe`.** Hexagonal architecture; the calculus is framework-free
   set arithmetic. `unsafe_code = "forbid"` in `Cargo.toml`. Dependencies point inward.

---

## Comparison vs alternatives

| | Question it answers | Certifies *intent*? | Refactor vs redesign? |
|---|---|---|---|
| **cargo-regime-check** | *Did this change stay inside what I declared?* | **yes** | **yes — mechanically** |
| [`cargo-semver-checks`](https://github.com/obi1kenobi/cargo-semver-checks) | *Is this change breaking (semver)?* | no | no |
| [`cargo-public-api`](https://github.com/enselic/cargo-public-api) | *What is the public surface, and what changed?* | no | no |
| Manual review | *…whatever the reviewer remembers to check* | no (faith) | no |

`cargo-semver-checks` and `cargo-public-api` are excellent and this tool **depends on**
the latter. The novel part is the **declared `u` + residual-must-be-justified** gate:
the *same* diff PASSes as a declared transition and FAILs as a refactor, isolating
exactly the lines a rename can't transport. Neither other tool does that.

---

## Installation

The gate builds and runs on **stable** Rust (1.74+). `cargo public-api` needs **nightly**
to emit rustdoc JSON — it's a *separate* process you pipe in; this tool never needs
nightly itself.

```sh
# From git (recommended — not yet on crates.io)
cargo install --git https://github.com/igouss/cargo-regime-check

# From a local checkout
git clone https://github.com/igouss/cargo-regime-check
cd cargo-regime-check
cargo install --path .        # or: just install

# You also need cargo public-api (this is on crates.io)
cargo install cargo-public-api --locked
```

> **crates.io:** not yet published. Install from git or source for now. (Tracked on the
> roadmap.)

---

## Quick start

```sh
# 1. Scaffold a regime file at your crate root (next to Cargo.toml).
cargo-regime-check --template > regime-transition.toml

# 2. Edit it: declare the renames/additions/removals/changes you intend.
#    For a pure rename-only refactor, set kind = "refactor" and list [[rename]]s.

# 3. Gate the crate's public API against a base ref.
cargo +nightly public-api -p <crate> diff origin/main..HEAD \
  | cargo-regime-check --regime regime-transition.toml

# 4. Branch on the exit code: 0 = clean, 1 = act on stdout, 2 = fix the invocation.
echo "exit: $?"
```

Or try it against the bundled fixtures with no nightly toolchain at all:

```sh
cargo-regime-check --regime examples/transition.toml --diff tests/fixtures/demo.diff   # PASS, exit 0
cargo-regime-check --regime examples/refactor.toml  --diff tests/fixtures/demo.diff    # FAIL, exit 1
```

---

## Command reference

```
cargo regime-check --regime <FILE> [--diff <FILE|->] [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-r, --regime <FILE>` | The declared transition (`regime-transition.toml`). Required unless `--explain`/`--template`/`--capabilities`. |
| `-d, --diff <FILE\|->` | `cargo public-api diff` text. Default: stdin (`-`). |
| `--format <FMT>` | `human` (default) or `json`. |
| `--json` | Alias for `--format json`. |
| `--match <MODE>` | `identity` (default; exact resolved-path match) or `substring` (looser legacy fallback). |
| `--explain` | Print the model + known approximations, then exit 0. |
| `--template` | Print a minimal `regime-transition.toml`, then exit 0. |
| `--capabilities` | Print the machine-readable contract (schema, exit codes, classes) as JSON, then exit 0. |
| `-h, --help` | Print help, then exit 0. |

Examples:

```sh
# JSON for an agent to parse
cargo-regime-check --regime regime-transition.toml --diff api.diff --json

# Read the model and the honest gap between it and the implementation
cargo-regime-check --explain

# Read the contract straight from the tool instead of remembering it
cargo-regime-check --capabilities
```

Mistype a flag and the tool suggests the nearest one (`unknown argument '--regimen' →
did you mean '--regime'?`), so an agent that fat-fingers once learns the spelling.

### Exit codes

| code | meaning | what you do |
|------|---------|-------------|
| `0`  | clean — every line transported or declared | proceed (commit / push / merge) |
| `1`  | undeclared / contradictory residual | **act on stdout**, then re-run |
| `2`  | usage / I/O / parse error | fix the invocation or the regime file; stdout/stderr says how |

### JSON output

`--format json` emits no colour and is byte-stable across re-runs. Each item carries a
`class`, the prose `required_action`, and a copy-pasteable `remediation` snippet:

```jsonc
{
  "verdict": "fail",
  "kind": "refactor",
  "counts": { "total": 5, "accounted": 2, "residual": 3, "violations": 3 },
  "items": [
    {
      "token": "pub fn demo::brand_new() -> i32",
      "path": "demo::brand_new",            // resolved identity
      "class": "residual_additive",
      "detail": null,                        // ADR/reason once declared
      "required_action": "undeclared added surface. If intentional: append the [[additive]] block …",
      "remediation": "[[additive]]\nitem = \"demo::brand_new\"\nadr  = \"ADR-XXXX\"  # …\n"
    }
    // …
  ]
}
```

On an exit-`2` failure the object is `{ "error", "hint", "template" }` instead, where
`template` is a minimal valid regime file you can write straight to disk.

---

## Configuration: `regime-transition.toml`

The regime file *is* the functor `u` — your declaration of intended change. Place it at
the crate root. Bootstrap one with `cargo-regime-check --template`.

```toml
[meta]
# "refactor"   -> the public API may only be RENAMED/MOVED. Any added/removed/changed
#                 surface FAILS the gate (declaring any of the blocks below contradicts
#                 the claim — it's then a transition).
# "transition" -> added/removed/changed surface is allowed, but every item must be
#                 declared below.
kind = "transition"

# Renames/moves — the iso part of `u`. Honoured only when BOTH the old item is removed
# AND the new item is added in the diff.
[[rename]]
from = "kvstore::Store::get"
to   = "kvstore::Reader::get"

# Intentional new public items (declared discovery). Carries an ADR/issue reference.
[[additive]]
item = "kvstore::Reader"
adr  = "ADR-ISP-001"

# Intentional removals — breaking; bump the major version.
[[removal]]
item   = "kvstore::legacy_stats"
reason = "unused since v1; folded into capacity()"

# Intentional signature changes.
[[change]]
item = "kvstore::capacity"
adr  = "ADR-0002"
```

### The canonical case: an Interface-Segregation split

[`examples/kvstore-isp-split.toml`](./examples/kvstore-isp-split.toml) is a **real**
regime file for splitting a fat `Store` trait into `Reader` / `Writer` / `Maintenance`
capability traits (the diff is genuine `cargo public-api` output, captured in
[`tests/fixtures/kvstore-isp-split.diff`](./tests/fixtures/kvstore-isp-split.diff)).

- Declared honestly as a **transition** — the four method renames + three new traits +
  their impls + the blanket impl + the dropped helper + the supertrait/signature
  changes — it PASSes: **20/20 accounted, 0 residual**.
- Re-claimed as `kind = "refactor"`, it FAILs with **12 Case-C violations**.

That's the guard: a trait split is *not* a refactor, and the tool proves it.

---

## Architecture

Hexagonal, dependencies pointing inward. The domain is pure set arithmetic — no serde,
no toml, no I/O. The category theory ([arXiv:2606.01444][paper]; Spivak's functorial
data migration / CQL) defines the buckets and then leaves the codebase.

```
   cargo +nightly public-api -p <crate> diff base..HEAD
                     │  (text diff, piped to stdin)
                     ▼
 ┌─────────────────────────────────────────────────────────────┐
 │ src/bin/cargo-regime-check.rs   driving adapter (args, exit)  │
 └──────────────┬──────────────────────────────┬────────────────┘
                │ diff text                     │ regime-transition.toml
                ▼                                ▼
   adapters/public_api_diff.rs        adapters/regime_file.rs
   text → ApiDiff (dedup impls)       TOML → RegimeTransition  (the functor u)
                │                                │
                └───────────────┬────────────────┘
                                ▼
              ─── domain/ (pure, framework-free) ───
              classify(diff, u, mode) → [Classified]
              gate(classified, kind)  → GateResult
                                │
                                ▼
                  report::build → Report (view-model)
                          │                  │
                    report::human      report::json
                          ▼                  ▼
                   exit 0 / 1 / 2   +   stdout you act on
```

| Layer | Responsibility |
|-------|----------------|
| `domain/` | `identity.rs` (token → `ApiIdentity{kind,path,signature}`), `diff.rs`, `transition.rs`, `classify.rs`, `gate.rs` — the single source of truth for "is this a violation, and what's required". |
| `adapters/` | `public_api_diff.rs` (text → `ApiDiff`), `regime_file.rs` (owns the TOML format + remediation rendering). |
| `report/` | Stable view-model + `human`/`json` renderers. |
| `bin/` | Arg parsing, exit codes, `--capabilities`. |

Extending it (new class, new action) is documented in [`AGENTS.md` §6](./AGENTS.md).

---

## CI

The short version — `cargo public-api` (nightly) piped into the gate (stable):

```yaml
- uses: dtolnay/rust-toolchain@nightly
- uses: dtolnay/rust-toolchain@stable
- run: cargo install cargo-public-api --locked
- run: cargo install --git https://github.com/igouss/cargo-regime-check --locked
- run: |
    cargo +nightly public-api -p my-crate diff origin/main..HEAD \
      | cargo-regime-check --regime regime-transition.toml --format json
```

A working `justfile` recipe (`just regime-check my-crate origin/main`) and a full
[GitHub Actions workflow](./.github/workflows/ci.yml) ship in the repo.

> The `base..HEAD` form makes `cargo public-api` git-checkout each commit in-tree to
> build rustdoc JSON — run it on a clean tree (it restores afterward), or build in a
> throwaway `git worktree`. Don't add `-s`/`--simplified` blindly: it omits
> blanket/auto-trait impls, which can hide an *intentional* blanket impl.

---

## Troubleshooting

| Symptom | Cause & fix |
|---------|-------------|
| `error: cannot read regime file …` (exit 2) | The file doesn't exist. Run `cargo-regime-check --template > regime-transition.toml` and declare your intent. |
| `error: cannot read diff …` (exit 2) | Nothing on stdin. Pipe `cargo +nightly public-api … diff …` in, or pass `--diff <file>`. |
| `unknown argument '--xyz'` (exit 2) | Typo — the tool prints the nearest valid flag. Run `--help` for the full set. |
| **Gate PASSes but you know the API changed** | Your `u` is wrong or over-broad. The tool certifies *declared* intent — it can't catch a lie in your own declaration. Re-read what you declared; that's the bug. |
| Diff is empty or surprising | You probably ran `cargo public-api -s`. Drop `-s` (it hides blanket impls). Also ensure the tree is clean and the base ref is fetched (`fetch-depth: 0` in CI). |
| A token isn't matched and you expected it to be | `cargo public-api` produced a token the identity parser can't resolve. Try `--match substring` as a looser fallback, and file an issue with the token. |

---

## Limitations

Honest about the gap between the cheap implementation and the faithful construction —
each is a ranked roadmap item in [`AGENTS.md` §5](./AGENTS.md):

1. **Matches by parsed text, not a structured ID.** The functor matches on the resolved
   identity path *parsed* out of `cargo public-api`'s token, because `cargo public-api`
   0.52 has no `--output json`. When it gains one, the adapter should consume the
   structured identity directly.
2. **`u` is declared, not inferred.** No structural-hash rename inference yet — a wrong
   `u` yields a confident-but-wrong green.
3. **One undifferentiated "change" bucket.** A widened signature (≈ safe) and a narrowed
   one (≈ breaking) are both `residual_change`; sub/supertype direction isn't classified.
4. **Single crate only.** No whole-workspace mode yet.
5. **It gates declared intent, not truth.** It proves the diff matches what you *said*;
   it does not validate that your ADR exists or that your reason is good. That's a
   reviewer's job — and now a small, well-defined one.

---

## FAQ

**Do I need a nightly toolchain?**
Not for this tool — it builds and runs on stable. Only `cargo public-api` needs nightly,
and it's a separate process you pipe in.

**Does it infer my renames for me?**
No. You declare them. The tool certifies that the diff stays inside your declared `u`; a
wrong declaration is your bug. (Auto-proposing renames from item-body hashes is on the
roadmap.)

**How is this different from `cargo-semver-checks`?**
`cargo-semver-checks` answers *"is this breaking?"*. This answers *"did you change only
what you declared?"*. Different question — a non-breaking change can still be an
undeclared surprise, and a breaking one can be fully intended and declared.

**Why does a "refactor" fail when I declared one harmless addition?**
Because declaring *any* added/removed/changed surface means it's a transition by
definition — that's the "Case C" catch. Set `meta.kind = "transition"`.

**Can it gate a whole workspace at once?**
Not yet — single crate per run. Whole-workspace mode is roadmap item #4.

**Is it on crates.io?**
Not yet. Install with `cargo install --git https://github.com/igouss/cargo-regime-check`.

**Where's the theory?**
`--explain` prints the model inline. The full treatment (residual after functorial
transport; the left Kan extension `Lan_u I`) is in [`AGENTS.md`](./AGENTS.md) and
[arXiv:2606.01444][paper]. None of the category theory is in the code — it's set
arithmetic over `cargo public-api`'s output.

[paper]: https://arxiv.org/abs/2606.01444

---

## Development

```sh
just check        # cargo test + clippy -D warnings + fmt --check
```

60 tests (unit + agent-loop + exit-code/ergonomics). No `unsafe` (`forbid`-en). Domain
stays framework-free. Tests follow zero/one/many with cyclomatic complexity 1. The
agent-loop test (`tests/agent_loop.rs`) is load-bearing: apply the tool's own emitted
remediation verbatim → re-run → exit 0.

---

## About Contributions

*About Contributions:* Please don't take this the wrong way, but I do not accept outside
contributions for any of my projects. I simply don't have the mental bandwidth to review
anything, and it's my name on the thing, so I'm responsible for any problems it causes;
thus, the risk-reward is highly asymmetric from my perspective. I'd also have to worry
about other "stakeholders," which seems unwise for tools I mostly make for myself for
free. Feel free to submit issues, and even PRs if you want to illustrate a proposed fix,
but know I won't merge them directly. Instead, I'll have Claude or Codex review
submissions via `gh` and independently decide whether and how to address them. Bug
reports in particular are welcome. Sorry if this offends, but I want to avoid wasted time
and hurt feelings. I understand this isn't in sync with the prevailing open-source ethos
that seeks community contributions, but it's the only way I can move at this velocity and
keep my sanity.

---

## License

Dual-licensed under [MIT](./LICENSE-MIT) OR [Apache-2.0](./LICENSE-APACHE), at your
option.
