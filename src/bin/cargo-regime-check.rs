//! `cargo regime-check --regime regime-transition.toml [--diff <file>|-] [--format json]`
//!
//! Reads a `cargo public-api diff` (stdin by default), subtracts the declared
//! transition `u`, and exits non-zero if any residual is undeclared. On FAIL it
//! prints exactly what to DO per residual line — stdout alone resolves it.

use std::io::Read;
use std::process::ExitCode;

use cargo_regime_check::adapters::{public_api_diff, regime_file};
use cargo_regime_check::domain::classify::{classify, Classified, MatchMode};
use cargo_regime_check::domain::gate::{gate, GateResult};
use cargo_regime_check::domain::transition::RegimeTransition;
use cargo_regime_check::report::{self, Report};

const USAGE: &str = "\
cargo-regime-check — gate a public-API diff against a declared transition `u`.

A change is admissible iff every diff line is transported by a declared rename or
declared as additive/removal/change. `meta.kind = \"refactor\"` forbids any
declared residual (it must be an endofunctor); `\"transition\"` allows declared
residual only.

USAGE:
  cargo +nightly public-api -p <crate> diff <old>..<new> \\
    | cargo regime-check --regime regime-transition.toml [OPTIONS]
  cargo regime-check --regime <toml> --diff <file>

OPTIONS:
  -r, --regime <FILE>   declared transition (regime-transition.toml).
                        Required unless --explain / --template.
  -d, --diff <FILE|->   cargo public-api diff text; default: stdin (-).
      --format <FMT>    output format: human (default) | json
      --json            alias for --format json
      --match <MODE>    identity (default; exact resolved-path) | substring (legacy)
      --explain         print the model + known approximations, then exit 0
      --template        print a minimal regime-transition.toml, then exit 0
      --capabilities    print the machine-readable contract as JSON, then exit 0
  -h, --help            print this help, then exit 0

EXIT CODES:
  0  clean — every line transported or declared
  1  undeclared / contradictory residual — act on the directives printed to stdout
  2  usage / I/O / parse error

The tool never prompts and emits no colour in --format json: deterministic,
re-runnable, branch on the exit code. See AGENTS.md for the full contract.";

const EXPLAIN: &str = "\
cargo-regime-check — the model
==============================

A public-API change is admissible iff every line of the diff lies in the IMAGE of
the declared transport `u`. Anything outside that image is RESIDUAL:

  residual = (new public surface)  \\  image(u applied to the old surface)

  - the renames in regime-transition.toml ARE the functor `u` (its iso part).
  - meta.kind = \"refactor\"   => `u` must be an ENDOFUNCTOR: residual must be 0.
                                Declaring any add/remove/change contradicts the
                                claim — it is then a transition, not a refactor.
  - meta.kind = \"transition\" => residual is allowed, but every residual line must
                                be DECLARED ([[additive]] / [[removal]] / [[change]]).

This operationalizes \"residual content beyond functorial transport\" from Wang &
Buehler, arXiv:2606.01444 (old artifacts transported by the left Kan extension and
compared to the post-transition state), and the schema-as-category /
migration-as-functor machinery of Spivak's functorial data migration (CQL). The
category theory defines the buckets; none of it is in the code.

Known approximations (where the implementation diverges from the theory)
------------------------------------------------------------------------
1. The functor matches by RESOLVED IDENTITY PATH, parsed from `cargo public-api`'s
   already-normalized token — not from a structured identity field.
   `cargo public-api` 0.52.0 has no `--output json`; when it gains one, the diff
   adapter should consume it directly. `--match substring` restores the looser
   legacy behaviour for tokens the parser cannot resolve.
2. Renames are DECLARED, not INFERRED. The tool certifies a declared `u`; it does
   not discover it (no structural-hash rename inference / left Kan extension yet).
   A wrong `u` yields a confident-but-wrong green.
3. A changed signature is one undifferentiated bucket; widening (≈ additive) vs
   narrowing (≈ lossy) is not yet told apart by sub/supertype.
4. Single-crate only; whole-workspace mode is future work.

See AGENTS.md for the full contract and the ranked roadmap.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Human,
    Json,
}

/// Parsed command line. `None` results from `--help/--explain/--template`, which
/// print and exit 0 during parsing.
struct Args {
    regime_path: Option<String>,
    diff_path: Option<String>,
    format: Format,
    match_mode: MatchMode,
}

/// A usage/IO/parse failure: message, what to do about it, and whether to show
/// the template.
struct Failure {
    message: String,
    hint: String,
    show_template: bool,
}

fn main() -> ExitCode {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    // Invoked as `cargo regime-check ...`: cargo passes the subcommand name.
    if argv.first().map(String::as_str) == Some("regime-check") {
        argv.remove(0);
    }

    // Bare invocation: show help and exit 0 — the first thing an agent tries works.
    if argv.is_empty() {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let args: Args = match parse_args(&argv) {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(f) => return emit_failure(&f, Format::Human),
    };

    match run(&args) {
        Ok(report) => {
            print!("{}", render(&report, args.format));
            if matches!(report.verdict, report::Verdict::Pass) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE // exit 1
            }
        }
        Err(f) => emit_failure(&f, args.format),
    }
}

/// Parse argv. Returns `Ok(None)` when a self-contained flag already handled the
/// whole invocation (help/explain/template) and the process should exit 0.
fn parse_args(argv: &[String]) -> Result<Option<Args>, Failure> {
    let mut regime_path: Option<String> = None;
    let mut diff_path: Option<String> = None;
    let mut format: Format = Format::Human;
    let mut match_mode: MatchMode = MatchMode::Identity;

    let mut i: usize = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--explain" => {
                println!("{EXPLAIN}");
                return Ok(None);
            }
            "--template" => {
                print!("{}", regime_file::template());
                return Ok(None);
            }
            "--capabilities" => {
                println!("{}", capabilities());
                return Ok(None);
            }
            // Intent inference: `--json` is the obvious guess for JSON output —
            // accept it as an alias for `--format json` instead of erroring.
            "--json" => format = Format::Json,
            "-r" | "--regime" => {
                i += 1;
                regime_path = Some(value(argv, i, "--regime")?);
            }
            "-d" | "--diff" => {
                i += 1;
                diff_path = Some(value(argv, i, "--diff")?);
            }
            "--format" => {
                i += 1;
                format = match value(argv, i, "--format")?.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unknown --format `{other}`"))),
                };
            }
            "--match" => {
                i += 1;
                match_mode = match value(argv, i, "--match")?.as_str() {
                    "identity" => MatchMode::Identity,
                    "substring" => MatchMode::Substring,
                    other => return Err(usage(format!("unknown --match `{other}`"))),
                };
            }
            other => return Err(unknown_arg(other)),
        }
        i += 1;
    }

    Ok(Some(Args {
        regime_path,
        diff_path,
        format,
        match_mode,
    }))
}

/// The classify→gate→build pipeline, with all I/O and parse failures surfaced.
fn run(args: &Args) -> Result<Report, Failure> {
    let Some(regime_path) = args.regime_path.as_deref() else {
        return Err(usage("--regime <toml> is required".to_owned()));
    };

    let regime_text: String =
        std::fs::read_to_string(regime_path).map_err(|e: std::io::Error| Failure {
            message: format!("cannot read regime file `{regime_path}`: {e}"),
            hint: format!(
                "create it — write the template below to `{regime_path}` (next to Cargo.toml) \
                 or run `cargo regime-check --template > {regime_path}`."
            ),
            show_template: true,
        })?;

    let u: RegimeTransition = regime_file::parse(&regime_text).map_err(|e| Failure {
        message: format!("malformed regime file `{regime_path}`: {e}"),
        hint: "fix it against the minimal valid template below.".to_owned(),
        show_template: true,
    })?;

    let diff_text: String = read_diff(args.diff_path.as_deref()).map_err(|e| Failure {
        message: format!("cannot read diff: {e}"),
        hint: "pipe `cargo +nightly public-api -p <crate> diff <old>..<new>` into stdin, \
               or pass --diff <file>."
            .to_owned(),
        show_template: false,
    })?;

    let classified: Vec<Classified> =
        classify(&public_api_diff::parse(&diff_text), &u, args.match_mode);
    let result: GateResult = gate(&classified, u.kind);
    Ok(report::build(&classified, &result, u.kind))
}

fn read_diff(path: Option<&str>) -> std::io::Result<String> {
    match path {
        None | Some("-") => {
            let mut buf: String = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
        Some(p) => std::fs::read_to_string(p),
    }
}

fn render(report: &Report, format: Format) -> String {
    match format {
        Format::Human => report::human::render(report),
        Format::Json => format!("{}\n", report::json::render(report)),
    }
}

fn value(argv: &[String], i: usize, flag: &str) -> Result<String, Failure> {
    argv.get(i)
        .cloned()
        .ok_or_else(|| usage(format!("{flag} needs a value")))
}

fn usage(message: String) -> Failure {
    Failure {
        message,
        hint: "run `cargo regime-check --help` for usage.".to_owned(),
        show_template: false,
    }
}

/// Every flag the parser accepts — the corpus for "did you mean" suggestions.
const KNOWN_FLAGS: &[&str] = &[
    "--regime",
    "--diff",
    "--format",
    "--match",
    "--json",
    "--explain",
    "--template",
    "--capabilities",
    "--help",
];

/// An unknown argument, with a Levenshtein-nearest "did you mean" when one is
/// close enough — an agent that mistypes once learns the spelling.
fn unknown_arg(arg: &str) -> Failure {
    let suggestion: Option<&&str> = KNOWN_FLAGS
        .iter()
        .map(|flag: &&str| (levenshtein(arg, flag), flag))
        .filter(|(d, _): &(usize, &&str)| *d <= 3)
        .min_by_key(|(d, _): &(usize, &&str)| *d)
        .map(|(_, flag): (usize, &&str)| flag);
    let hint: String = match suggestion {
        Some(flag) => format!("did you mean `{flag}`? run `cargo regime-check --help` for usage."),
        None => "run `cargo regime-check --help` for usage.".to_owned(),
    };
    Failure {
        message: format!("unknown argument `{arg}`"),
        hint,
        show_template: false,
    }
}

/// Plain Levenshtein edit distance (no unsafe, small inputs).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost: usize = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// The machine-readable contract: version, exit codes, formats, match modes, and
/// the report schema. Lets an agent read the contract straight from the tool
/// instead of remembering it. Deterministic (a static value).
fn capabilities() -> String {
    let value: serde_json::Value = serde_json::json!({
        "tool": "cargo-regime-check",
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": 1,
        "exit_codes": {
            "0": "clean — every line transported or declared",
            "1": "undeclared/contradictory residual",
            "2": "usage/IO/parse error"
        },
        "formats": ["human", "json"],
        "match_modes": ["identity", "substring"],
        "regime_kinds": ["refactor", "transition"],
        "classes": [
            "transported_iso",
            "declared_additive",
            "declared_removal",
            "declared_change",
            "residual_additive",
            "residual_removal",
            "residual_change"
        ],
        "required_actions": [
            "DeclareAdditive",
            "DeclareRemoval",
            "DeclareChange",
            "ReclassifyAsTransition"
        ],
        "report_schema": {
            "verdict": "pass|fail",
            "kind": "refactor|transition",
            "counts": ["total", "accounted", "residual", "violations"],
            "items": ["token", "path", "class", "detail", "required_action", "remediation"]
        },
        "requires_nightly_for": "cargo public-api (run separately and piped in; this tool runs on stable)"
    });
    serde_json::to_string_pretty(&value).expect("capabilities value serializes")
}

/// Print a failure (exit 2). In JSON mode it goes to stdout as a structured
/// object so an agent can parse it; in human mode it goes to stderr with the
/// template inline when relevant.
fn emit_failure(f: &Failure, format: Format) -> ExitCode {
    match format {
        Format::Json => {
            let template: Option<String> = f.show_template.then(regime_file::template);
            let value: serde_json::Value = serde_json::json!({
                "error": f.message,
                "hint": f.hint,
                "template": template,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("error value serializes")
            );
        }
        Format::Human => {
            eprintln!("error: {}", f.message);
            eprintln!("hint: {}", f.hint);
            if f.show_template {
                eprintln!("\n--- minimal regime-transition.toml ---");
                eprint!("{}", regime_file::template());
            }
        }
    }
    ExitCode::from(2)
}
