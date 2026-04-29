# Bash instrumentation — `InstrumentationSpec`, `CaptureSpec`, harvest

## What it does

`InstrumentationSpec` is the unified, declarative description of bash
code to inject into a target shell — capture trie, function defs,
variable exports, init lines, sources, entries, and pre-seeded calls.
The same spec can be run two ways: against an existing `BashSession`
(`run_on`) or via a child bash process with `BASH_ENV` injection plus
EXIT-trap harvest (`run_bash`). All captured calls — DSL invocations,
BASHCAP snapshots, anything else — flow through one shared bash array,
`__dsl_calls`, harvested at the end as a flat ordered list.

## Minified Rust problem statement

```rust
// src/bash/instrumentation.rs:81-98
#[derive(Debug, Clone)]
pub struct InstrumentationSpec {
    pub captures: Vec<CaptureSpec>,
    pub function_defs: Vec<(String, String)>,
    pub vars: IndexMap<String, String>,
    pub init_lines: Vec<String>,
    pub sources: Vec<PathBuf>,
    pub entries: Vec<String>,
    pub pre_calls: Vec<CapturedCall>,
}
```

A pure-data builder. Domain modules (`mb_base()`, `bashcap_spec()`)
construct one of these; runners turn it into bash code and execute it.

## Top-level flow

```text
                 ┌──────────────────────┐
   build spec ──▶│ InstrumentationSpec  │
                 └──────────┬───────────┘
                            │
                ┌───────────┴────────────┐
                ▼                        ▼
       run_on(session)            run_bash(args)
       persistent PTY+FIFO        child bash + BASH_ENV
       harvest via                harvest via
       read_indexed_array         EXIT trap → file
                            │
                            ▼
                   parse_harvest(raw[])
                   → CapturedCalls
```

The codegen step (`InstrumentationSpec::codegen`) is shared. Only the
transport (how the script reaches bash, how the harvest comes back)
differs.

## `CapturedCall` — the harvest unit

```rust
// src/bash/instrumentation.rs:15-36
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedCall {
    pub commandlist: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CapturedCalls {
    pub calls: Vec<CapturedCall>,
}

impl CapturedCalls {
    pub fn get(&self, prefix: &[&str]) -> Vec<&CapturedCall> {
        self.calls.iter()
            .filter(|c| {
                prefix.len() <= c.commandlist.len()
                    && c.commandlist.iter().zip(prefix).all(|(a, b)| a.as_str() == *b)
            })
            .collect()
    }
}
```

A `CapturedCall` is a flat `Vec<String>` — what bash would call a
"commandlist". The first element is conventionally a tag (`"DSL"`,
`"__SNAP__"`, etc.); `get(prefix)` filters by that. This keeps the
post-harvest API identical regardless of which capture produced the
call.

## The `__dsl_calls` accumulator protocol

Every captured call appends a single `${array[*]@Q}`-encoded string to
the bash array `__dsl_calls`. At harvest time, Rust reads that array
as an indexed array, then re-parses each element as another indexed
array (the inner commandlist). The outer parse is "one element per
captured call"; the inner parse is "the commandlist for that call".

```rust
// src/bash/instrumentation.rs:288-302
pub fn parse_harvest(raw_elements: &[String]) -> Result<CapturedCalls, ParseError> {
    let mut calls = Vec::new();
    for el in raw_elements {
        match parse_typed_value(BashType::IndexedArray, el) {
            Ok(BashValue::IndexedArray(commandlist)) => {
                calls.push(CapturedCall { commandlist });
            }
            Ok(_) => unreachable!(),
            Err(e) => return Err(e),
        }
    }
    Ok(CapturedCalls { calls })
}
```

This is shared between `run_on` and `run_bash` — the array element
shape is the same, only the transport that delivers it changes.

## Codegen ordering

```rust
// src/bash/instrumentation.rs:250-282
impl InstrumentationSpec {
    pub fn codegen(&self) -> String {
        let mut s = String::new();

        // 1. Init lines
        for line in &self.init_lines {
            s.push_str(line);
            s.push('\n');
        }

        // 2. Capture dispatch (includes __dsl_calls=() init)
        s.push_str(&generate_capture_script(&self.captures));

        // 3. Function definitions
        for (name, body) in &self.function_defs {
            s.push_str(&format!("{name}() {{ {body}; }}\n"));
        }

        // 4. Variable exports
        for (k, v) in &self.vars {
            s.push_str(&format!("export {}={}\n", k, encode_scalar(v)));
        }

        s
    }
}
```

Order matters:

1. **Init lines** must come first — they enable `shopt -s expand_aliases`
   (so subsequent `alias BAIL=…` works), `set` flags, and any traps
   that should run during the rest of injection.
2. **Capture dispatch + `__dsl_calls=()`.** The capture script
   *unconditionally* initialises the array, even when `captures` is
   empty (`src/bash/capture.rs:36`). This means a spec with only
   `init_lines` (like bashcap) still has an `__dsl_calls` to append to.
3. **Function defs** layer on top — they may reference capture
   functions or be referenced by entries.
4. **`export K=V`** — set last so all earlier code sees the unexported
   defaults if any (in practice, defaults aren't unexported; this is
   just a stable order).

## `CaptureSpec` and the dispatch trie

```rust
// src/bash/capture.rs:14-23
#[derive(Debug, Clone)]
pub struct CaptureSpec {
    pub prefix_source: Vec<String>,
    pub prefix_emitted: Vec<String>,
}
```

Given `CaptureSpec { prefix_source: ["DSL", "^"], prefix_emitted: ["DSL"] }`,
the generated bash code defines a function `DSL()` that:

1. matches its first argument against `^` via `case`,
2. shifts that discriminator out,
3. appends `["DSL"] ++ remaining_args` to `__dsl_calls`.

When multiple specs share a function name (same `prefix_source[0]`),
they are merged into one function with nested case dispatch:

```rust
// src/bash/capture.rs:35-47
pub fn generate_capture_script(specs: &[CaptureSpec]) -> String {
    let mut s = String::from("__dsl_calls=()\n");
    if specs.is_empty() {
        return s;
    }
    let trie = build_trie(specs);
    for (func_name, node) in &trie {
        s.push_str(&format!("{func_name}() {{\n"));
        emit_node(&mut s, node, 1);
        s.push_str("}\n");
    }
    s
}
```

The trie itself is built recursively from `prefix_source` indices
(`src/bash/capture.rs:51-88`). Two safety asserts catch
mis-specifications:

```rust
// src/bash/capture.rs:68-71
assert!(leaves.len() <= 1,
    "duplicate capture specs at depth {depth}");
assert!(leaves.is_empty() || branches.is_empty(),
    "conflicting capture specs: leaf and branch at depth {depth}");
```

Emitted code at a leaf:

```rust
// src/bash/capture.rs:93-105
DispatchNode::Leaf { emitted } => {
    let recording = if emitted.is_empty() {
        "\"${*@Q}\"".to_string()
    } else {
        let prefix = emitted.iter()
            .map(|e| format!("'{e}'"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("\"{prefix} ${{*@Q}}\"")
    };
    s.push_str(&format!("{indent}__dsl_calls+=({recording});\n"));
}
```

A branch emits a `case` block keyed on `$1` after a `shift`:

```rust
// src/bash/capture.rs:106-117
DispatchNode::Branch { cases } => {
    let var = format!("_d{depth}");
    s.push_str(&format!("{indent}declare -- {var}=\"$1\"; shift || THROW;\n"));
    s.push_str(&format!("{indent}case \"${var}\" in\n"));
    for (pattern, child) in cases {
        s.push_str(&format!("{indent}  '{pattern}')\n"));
        emit_node(s, child, depth + 1);
        s.push_str(&format!("{indent}    ;;\n"));
    }
    s.push_str(&format!("{indent}esac\n"));
}
```

The `THROW` here is an alias defined by `mb_base()` (it returns 112 to
short-circuit caller chains) — codegen and domain presets cooperate.

## Runner 1: `run_on(session)` — persistent session, FIFO harvest

```rust
// src/bash/instrumentation.rs:308-329
impl InstrumentationSpec {
    pub fn run_on(&self, session: &mut BashSession) -> Result<CapturedCalls, BashSessionError> {
        session.run(&self.codegen())?;
        for path in &self.sources {
            session.run(&format!("source {}", encode_scalar(&path.to_string_lossy())))?;
        }
        for cmd in &self.entries {
            session.run(cmd)?;
        }
        let harvested = Self::harvest_from_session(session)?;
        let mut all = self.pre_calls.clone();
        all.extend(harvested.calls);
        Ok(CapturedCalls { calls: all })
    }

    fn harvest_from_session(session: &mut BashSession) -> Result<CapturedCalls, BashSessionError> {
        let raw = session.read_indexed_array("__dsl_calls")?;
        Ok(parse_harvest(&raw)?)
    }
}
```

The session is reusable across many `run_on` calls — useful for bulk
extraction, where MB resolution opens one session and runs `init_spec`
plus several `aspect_spec` extractions back-to-back. Each run is a
fresh codegen injection; each clears `__dsl_calls=()` (re-issued by
`generate_capture_script`); each harvests only its own results.

`pre_calls` is merged in *before* the harvested calls (line 319), so
parsed-from-disk dependency declarations land alongside captured DSL
calls (used by `MBModule::init_spec` to inject `dependencies.list`
content as if it had been captured).

## Runner 2: `run_bash(args)` — child bash, BASH_ENV, EXIT trap

```rust
// src/bash/instrumentation.rs:422-477
impl InstrumentationSpec {
    pub fn run_bash(&self, bash_args: &[String]) -> Result<RunBashResult, RunBashError> {
        let tmp_dir = tempfile::tempdir()?;

        let harvest_file = tmp_dir.path().join("harvest.txt");
        fs::write(&harvest_file, "")?;

        // Build injection: codegen + EXIT trap finalization
        let mut injection = self.codegen();
        injection.push_str(&format!(
            concat!(
                "__HARVEST_FILE='{}'\n",
                "trap '",
                "if (( ${{#__dsl_calls[@]}} > 0 )); then ",
                "echo \"${{__dsl_calls[*]@Q}}\" >> \"$__HARVEST_FILE\"; ",
                "fi",
                "' EXIT\n",
            ),
            harvest_file.to_str().unwrap(),
        ));

        let injection_file = tmp_dir.path().join("bashcap_env.bash");
        fs::write(&injection_file, &injection)?;

        // Spawn bash with BASH_ENV pointing to injection
        let status = std::process::Command::new("bash")
            .args(bash_args)
            .env("BASH_ENV", injection_file.to_str().unwrap())
            .status()?;

        // Harvest from file
        let raw_data = fs::read_to_string(&harvest_file)?;
        let mut all_calls = self.pre_calls.clone();
        for line in raw_data.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            let raw = match parse_typed_value(BashType::IndexedArray, line) {
                Ok(BashValue::IndexedArray(v)) => v,
                Ok(_) => continue,
                Err(e) => return Err(e.into()),
            };
            let harvested = parse_harvest(&raw)?;
            all_calls.extend(harvested.calls);
        }

        Ok(RunBashResult {
            calls: CapturedCalls { calls: all_calls },
            exit_code: status.code(),
        })
    }
}
```

Two delivery mechanisms wrapped together:

1. **`BASH_ENV` for injection.** Bash sources the file pointed to by
   `BASH_ENV` at non-interactive startup. The child gets its own
   private copy of `__dsl_calls`, capture functions, and the EXIT
   trap. This works for *every* bash invocation — subshells via `(…)`,
   `bash -c`, command substitutions — because each new bash process
   re-sources `BASH_ENV`.
2. **EXIT trap → shared file for harvest.** Each bash that exits
   appends its `__dsl_calls` to one shared file. Rust reads the file
   line-by-line after the parent bash exits; each line is one bash
   process's worth of captures. Order is process-exit order.

The `sources` and `entries` fields of the spec are *not* used by
`run_bash` — the bash args ARE the execution. This is the bashcap
mode: wrap an arbitrary command transparently.

## A third runner: `extract` (one-shot session)

```rust
// src/bash/instrumentation.rs:335-384 (excerpted)
impl InstrumentationSpec {
    pub fn extract(self) -> Result<InstrumentationResult, InstrumentationError> {
        let mut session = match BashSession::new(BashSessionConfig::default()) {
            Ok(s) => s,
            Err(e) => return Err(InstrumentationError {
                spec: self,
                phase: InstrumentationPhase::SessionCreate,
                cause: e.into(),
            }),
        };
        // … codegen, sources, entries, harvest, all phase-tagged
    }
}
```

Convenience wrapper: makes a fresh session, runs the spec, drops it.
Errors carry the spec back via `InstrumentationError.spec` so the
caller can dump the failing configuration in diagnostic output. Used
by `MBModule::extract_init` and `MBModule::extract_aspect`
(`src/mb/module.rs:147-154`).

## Harvest file format (run_bash)

One bash process = one line of harvest output. The line is
`${__dsl_calls[*]@Q}` — a space-separated, `@Q`-quoted concatenation of
the entire array. Within Rust:

- Each line is parsed as a `BashType::IndexedArray` (giving a
  `Vec<String>` of array elements, one element per captured call).
- Each element is itself parsed as another `BashType::IndexedArray`
  (giving the commandlist for that call).

Empty arrays produce no line; the trap guards against this with
`if (( ${#__dsl_calls[@]} > 0 ))`. Trailing blanks are ignored on
parse.

## Errors and provenance

`InstrumentationError` carries the failing spec, the phase
(`SessionCreate | Injection | Source(path) | Entry(cmd) | Harvest`),
and the underlying `BashSessionError`. The `spec` field is the
self-describing root — combined with the optional `dump()` method on
`InstrumentationResult`, you can print the full configuration that
caused a failure for after-the-fact debugging.

```rust
// src/bash/instrumentation.rs:42-58
#[derive(Debug, Clone)]
pub enum InstrumentationPhase {
    SessionCreate,
    Injection,
    Source(PathBuf),
    Entry(String),
    Harvest,
}

#[derive(Debug)]
pub struct InstrumentationError {
    pub spec: InstrumentationSpec,
    pub phase: InstrumentationPhase,
    pub cause: BashSessionError,
}
```

## See also

- [`bash-interop.md`](bash-interop.md) — the `BashSession` plumbing
  this layer rides on
- [`mb-extraction.md`](mb-extraction.md) — first heavy consumer:
  `mb_base()` builds a CaptureSpec; `MBModule::*_spec()` builds an
  InstrumentationSpec
- [`bashcap.md`](bashcap.md) — the second consumer; uses `init_lines`
  only, no capture trie
- [Bash manual: `BASH_ENV`](https://www.gnu.org/software/bash/manual/html_node/Bash-Variables.html)
- [Bash manual: `${array[*]@Q}` and ANSI-C quoting](https://www.gnu.org/software/bash/manual/html_node/Shell-Parameter-Expansion.html)
