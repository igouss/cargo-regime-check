# AGENTS.md — `cargo-regime-check`

You are an agent running `cargo-regime-check` in CI or a pre-push hook. This file
is your contract: what the tool decides, how to react with **no human in the
loop**, and how to extend the tool correctly. Read the model once; after that,
branch on the exit code and act on stdout.

---

## 1. The model (what "residual" means)

A public-API change is **admissible** iff every line of the diff lies in the
image of the declared transport `u`. Anything outside that image is **residual**:

```
residual = (new public surface)  \  image(u applied to the old surface)
```

- The **renames** in `regime-transition.toml` *are* the functor `u` — its
  iso/relabelling part.
- `meta.kind = "refactor"`  ⇒ `u` must be an **endofunctor**: residual must be 0.
  Declaring *any* add/remove/change contradicts the claim — it is then a
  transition, not a refactor. (This is "Case C": the falsified-refactor catch.)
- `meta.kind = "transition"` ⇒ residual is allowed, but every residual line must
  be **declared** as `[[additive]]`, `[[removal]]`, or `[[change]]`.

This operationalizes "residual content beyond functorial transport" from Wang &
Buehler, *Self-Revising Discovery Systems for Science* (arXiv:2606.01444): old
artifacts are transported by the **left Kan extension** `Lan_u I` and compared to
the post-transition state; the residual *is* the discovery. The schema-as-
category / migration-as-functor machinery is Spivak's functorial data migration
(CQL). **The category theory defines the buckets; none of it is in the code** —
the implementation is set arithmetic over `cargo public-api`'s output.

The sharp consequence: **"refactor vs redesign" is mechanically falsifiable.**
The *same* diff PASSes as a declared transition and FAILs as a refactor.

---

## 2. The reaction contract

### Exit codes (branch on these)

| code | meaning | what you do |
|------|---------|-------------|
| `0`  | clean — every line transported or declared | proceed (commit / push / merge) |
| `1`  | undeclared / contradictory residual | **act on stdout** (see below), then re-run |
| `2`  | usage / I/O / parse error | fix the invocation or the regime file; stdout/stderr tells you how |

Re-runs are **idempotent**: same `(diff, regime)` ⇒ byte-identical output and the
same exit code. After you apply a fix, re-run and re-branch.

### On exit 1: act per residual line

Run with `--format json` and read `items[]`. Each failing item carries:

- `class` — one of `residual_additive`, `residual_removal`, `residual_change`
  (undeclared), or a `declared_*` class flagged only because `kind = "refactor"`.
- `required_action` — the prose directive (the *what to do*, both branches).
- `remediation` — a **copy-pasteable `regime-transition.toml` snippet** that, if
  appended verbatim, makes that line accounted-for.

| `class` | if the change is INTENTIONAL | if it is ACCIDENTAL |
|---------|------------------------------|---------------------|
| `residual_additive` | append the `[[additive]]` snippet; ensure `kind = "transition"` | make the item `pub(crate)` or remove it |
| `residual_removal`  | append the `[[removal]]` snippet; bump the **major** version | restore the removed item |
| `residual_change`   | append the `[[change]]` snippet with an ADR ref | revert the signature |
| `declared_*` under `refactor` | set `meta.kind = "transition"` — you declared surface, so it is a transition | (n/a) |

**The autonomous default:** if you believe the change is intentional (you made
it), append every `remediation` snippet to the regime file, replace the
`ADR-XXXX` / `REPLACE:` placeholders with the real issue/ADR reference and reason,
and re-run. The gate certifies that each line is *declared*; it does not validate
that your ADR exists — that is your responsibility, and a reviewer's.

`stdout` alone resolves a FAIL: the human report (default) prints the directive
and the indented fix under each `✗` line; the JSON form carries them as fields.

### The JSON schema (stable)

```jsonc
{
  "verdict": "pass" | "fail",
  "kind": "refactor" | "transition",
  "counts": { "total": N, "accounted": N, "residual": N, "violations": N },
  "items": [
    {
      "token": "pub fn kvstore::Reader::get<...> -> ...",
      "path": "kvstore::Reader::get",                    // resolved identity
      "class": "transported_iso",
      "detail": "ADR-ISP-001" | null,                    // ADR/reason that accounted it
      "required_action": null | "…directive…",
      "remediation": null | "[[additive]]\nitem = \"…\"\n…"
    }
  ]
}
```

On a code-`2` failure the JSON object is `{ "error", "hint", "template" }`
instead — `template` is a minimal valid `regime-transition.toml` you can write to
disk. `--format json` emits **no colour** and is deterministic.

### Workspace mode (aggregate)

`--workspace` gates **every** member carrying a `regime-transition.toml` in one run and
aggregates the outcomes. The aggregate exit code is the **max** over crates:
`error (2) > residual (1) > clean (0)`. One crate's tool-error is *recorded* (an
`errored` entry) and does **not** abort its siblings, yet still drives the run to `2`.
The refuse-a-false-green rules: **zero** gated crates discovered ⇒ exit `2`; a gated crate
with no `<crate>.diff` under `--diff-dir` ⇒ that crate is `errored` and the run exits `2`
(never a silent skip); every gated crate skipped by `--changed-only` (nothing relevant
changed) ⇒ exit `0`.

The aggregate JSON embeds each gated crate's single-crate report **unchanged** under
`report` (byte-equal to a standalone single-crate JSON run), so you parse one nested
schema:

```jsonc
{
  "verdict": "pass" | "fail" | "error",       // → exit 0 | 1 | 2
  "counts": { "crates": N, "clean": N, "residual": N, "errored": N, "skipped": N },
  "crates": [                                  // sorted by name
    { "name": "…", "status": "clean" | "residual", "report": { /* single-crate report */ } },
    { "name": "…", "status": "errored", "error": "…", "hint": "…" },
    { "name": "…", "status": "skipped", "reason": "…" }
  ]
}
```

`counts.crates = clean + residual + errored` (the evaluated crates; `skipped` are listed
but not counted). The machine-readable contract is in `cargo regime-check --capabilities`
under a `workspace` block. See §3 for invocation.

---

## 3. How to run it

`cargo-regime-check` builds and runs on **stable**. `cargo public-api` needs
**nightly** to emit rustdoc JSON, so always invoke it as a *separate* process:

```sh
# install the gate (once)
cargo install --path .          # or: just install

# CI / pre-push: gate a crate's public surface against a base ref
cargo +nightly public-api -p my-port-crate diff origin/main..HEAD \
  | cargo-regime-check --regime regime-transition.toml --format json
echo "exit: $?"   # 0 clean / 1 residual / 2 error
```

or via the recipe: `just regime-check my-port-crate origin/main`.

Notes:
- The `<base>..HEAD` form makes `cargo public-api` **git-checkout each commit
  in-tree** to build its rustdoc JSON — run it on a clean tree (it restores
  afterwards; pass nothing extra in CI). To avoid touching a working tree, build
  in a throwaway `git worktree`.
- Do **not** add `-s`/`--simplified` blindly: it omits blanket/auto-trait impls,
  which can hide an *intentional* blanket impl (the `kvstore` example adds one).
  The gate fails *closed* — prefer the full diff.
- Bootstrap a regime file: `cargo-regime-check --template > regime-transition.toml`.
- Read the model any time: `cargo-regime-check --explain`.

### Whole-workspace (gate every gated crate at once)

Gate every member carrying a `regime-transition.toml` in one invocation instead of
looping per crate:

```sh
# Default (process) mode: cargo public-api per gated crate, base..HEAD. Needs --base and
# a CLEAN tree — public-api git-checks-out each commit in-tree to build rustdoc JSON, so
# a dirty tree is refused (exit 2). Escape hatches: commit/stash, a throwaway
# `git worktree`, or --diff-dir.
cargo regime-check --workspace --base origin/main --format json

# --diff-dir mode: gate pre-captured diffs on stable — no nightly, no checkout, no
# dirty-tree check. A gated crate with no <crate>.diff is an error, never a skip.
cargo regime-check --workspace --diff-dir target/regime-diffs --format json
```

The CI-native split: a **nightly** stage writes one `cargo public-api … diff base..HEAD`
per crate into a directory; a **stable** stage gates them with `--workspace --diff-dir
<dir>`. `--base` is required in process mode and with `--changed-only`; it is ignored in
pure `--diff-dir` mode. A workspace flag without `--workspace` (or `--regime`/`--diff`
*with* `--workspace`) is a usage error (exit 2), never a silent no-op. `--changed-only`
skips (but still lists) crates no file under their own tree touched since `--base` — a
performance concession that can miss a crate re-exporting a changed dependency (§4).

---

## 4. Known approximations (where the code diverges from the theory)

These are not bugs; they are the honest gap between the cheap implementation and
the faithful construction. Each is a roadmap item (§5).

1. **Identity, not a structured ID.** The functor matches by the **resolved
   identity path** parsed out of `cargo public-api`'s already-normalized token —
   exact, not substring (the prototype's looseness). But it is parsed *text*:
   `cargo public-api` 0.52.0 has **no `--output json`**. When it gains one, the
   diff adapter should consume the structured identity directly. `--match
   substring` restores the legacy loose behaviour for tokens the parser can't
   resolve.
2. **`u` is declared, not inferred.** The tool *certifies* a declared `u`; it does
   not *discover* it. A wrong `u` produces a confident-but-wrong green. There is
   no structural-hash rename inference yet (no left-Kan auto-transport).
3. **One undifferentiated "change" bucket.** A widened signature (≈ additive, safe)
   and a narrowed one (≈ lossy, breaking) are both `residual_change`. Sub/supertype
   direction is not classified.
4. **`--changed-only` is a heuristic.** Whole-workspace mode ships (§2, §3); its
   `--changed-only` optimization assumes a crate's public API is a function of files in
   its own tree, so a crate re-exporting a changed dependency (`pub use dep::X`) can
   change surface with no edits to its own tree and be wrongly skipped. Gate-everything
   is the safe default; `--changed-only` is opt-in.
5. **Cross-section identity collisions are independent.** When the same `path`
   appears in both removed and added (e.g. a direct impl replaced by a blanket
   impl), each side is classified on its own and must be declared separately. The
   tool will not silently treat them as transported (that would be a false green).

---

## 5. Roadmap (ranked; each names the gap, not a vibe)

1. **JSON-ID matching = the faithful functor.** Consume `cargo public-api
   --output json` (or the `public-api` crate's structured tokens) so items match
   by resolved identity, not parsed text. Closes approximation #1.
2. **Structural-hash rename inference ≈ the left Kan extension.** Auto-propose
   `[[rename]]` entries from item-body hashes / signature similarity instead of
   requiring them by hand. Moves the tool from referee toward engine (#2).
3. **Change-direction classification via sub/supertype.** Split `residual_change`
   into widening (≈ additive) vs narrowing (≈ lossy) so the gate's severity
   matches semver reality. Closes #3.
4. **Emit the accepted `regime-transition.toml` as a changelog/ADR artifact.** The
   declared `u` is a version-controlled record of every intentional API change —
   render it into CHANGELOG/ADR entries on PASS.

---

## 6. How to extend it (architecture)

Hexagonal; dependencies point inward. Keep the domain framework-free.

- `src/domain/` — **pure** calculus. No serde/toml/IO.
  - `identity.rs` — parse a token into `ApiIdentity { kind, path, signature }`.
    *This is where faithful-matching work (roadmap #1) lands.*
  - `diff.rs` — `ApiItem`/`ApiChange`/`ApiDiff` (each item carries its identity).
  - `transition.rs` — the declared `u` (`RegimeKind`, `Rename`, `Additive`, …).
  - `classify.rs` — `classify(diff, u, MatchMode) -> [Classified]`. *Rename
    inference (roadmap #2) and change-direction (#3) extend here.*
  - `gate.rs` — `gate(...) -> GateResult` + `required_action(class, kind)`, the
    single source of truth for "is this a violation, and what's required".
- `src/adapters/` — driven adapters.
  - `public_api_diff.rs` — `cargo public-api diff` text → `ApiDiff` (dedups
    duplicate impl lines). *A JSON source (roadmap #1) is a sibling adapter here.*
  - `regime_file.rs` — owns the TOML format: parse, `template()`, `directive()`,
    `snippet()`. *New remediation forms land here, never in the domain.*
- `src/report/` — presentation: the stable `Report` view-model + `human` / `json`
  renderers.
- `src/bin/cargo-regime-check.rs` — the driving adapter (arg parsing, exit codes).

Rules of the house: no `unsafe`; explicit type annotations on bindings and lambda
params; tests have cyclomatic complexity 1 (test zero / one / many);
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` stay clean.
When you add a class or action, update `Class::as_str`, `required_action`, the
`regime_file` renderers, and add a 0/1/many test — the JSON schema is a contract.
