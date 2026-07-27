# AGENTS.md — `matugen-ffi`

This file orients coding agents (and humans) working in this crate.
`matugen-ffi` was originally forked from [`matugen`](https://github.com/InioX/matugen)
to add an FFI surface on top of its color-extraction and template-rendering
logic. That's the only thing worth remembering about its origin — this is
now the project actively developed and used; there is no sibling `matugen`
project to keep in sync with, diff against, or avoid touching. Treat this
crate as the source of truth.

## What this project is

`matugen-ffi` is matugen's engine (color extraction + templating), with:

- The core logic (`State`, all modules) living in a library (`matugen-ffi/src/lib.rs`),
  so it can be reused as a dependency and wrapped in a C ABI.
- A `matugen-ffi/src/ffi.rs` module exposing 6 `extern "C"` functions
  (`matugen_extract_colors`, `matugen_get_source_colors`,
  `matugen_render_template`, `matugen_render_from_image`,
  `matugen_write_output`, `matugen_free_string`) so non-Rust host
  applications (originally: a Bun/TypeScript theme manager) can reuse
  matugen's color-extraction and template-rendering logic without going
  through matugen's own `config.toml`.
- A CLI binary (`matugen`, `matugen-ffi/src/main.rs`) that's a thin wrapper
  around `State`, kept for convenience/testing.
- Bun/TypeScript bindings and examples at the project root (`src/`, `examples/`).

The design brief is [`matugen-ffi/matugen-ffi-prd.md`](./matugen-ffi/matugen-ffi-prd.md) — read it
first if you need the *why* behind any of the choices below.

## Project layout

```
matugen-ts/                      ← project root (Bun/TypeScript package)
├── src/                         ← Bun/TypeScript bindings
│   ├── ffi/index.ts             ← low-level FFI symbols, library resolution, callFfi
│   ├── matugen.ts               ← Matugen class (typed wrapper)
│   ├── api.ts                   ← module-level singleton API (primary entry point)
│   ├── types.ts                 ← request/response types mirroring ffi.rs JSON contract
│   ├── error.ts                 ← MatugenFfiError, unwrap helper
│   └── index.ts                 ← re-exports everything
├── examples/                    ← runnable Bun examples
│   ├── example.ts               ← explicit color → extract → render → write
│   ├── example-image.ts         ← image → extract → render (dark + light)
│   ├── example-buffer.ts        ← base64 image bytes pipeline
│   └── load-template.ts         ← load templates from disk, single-call & two-step
├── templates/                   ← sample templates for examples
├── images/                      ← sample images for examples
├── matugen-ffi/                 ← Rust crate (the engine)
│   ├── Cargo.toml
│   ├── cbindgen.toml
│   ├── matugen.h                ← generated C header (regenerate after FFI signature changes)
│   └── src/
│       ├── lib.rs               ← State, pub mod declarations
│       ├── main.rs              ← thin CLI wrapper
│       ├── ffi.rs               ← 6 extern "C" functions (the FFI surface)
│       ├── template.rs          ← TemplateFile, write_to_disk, format_hook
│       ├── helpers.rs           ← generate_schemes_and_theme, merge_json_source, etc.
│       ├── scheme.rs            ← SchemeTypes, Schemes, SchemesEnum
│       ├── parser/              ← Template engine (Engine, parser, filters)
│       ├── color/               ← Color extraction, format, parsing
│       ├── filters/             ← Built-in template filters
│       └── ...
├── package.json
└── tsconfig.json
```

| | |
|---|---|
| Crate type | `bin` (`matugen`) + `lib` (`matugen_ffi`, `crate-type = ["cdylib", "rlib"]`) |
| Entry point | `matugen-ffi/src/main.rs` is a thin CLI wrapper around `matugen-ffi/src/lib.rs::State` |
| FFI surface | `matugen-ffi/src/ffi.rs` (`extern "C"`, behind the `ffi` feature, on by default) — 6 functions |
| `std::process::exit()` | only in the CLI-only `Engine::add_template`; every FFI-reachable path returns `Result` instead |
| Bindings | `src/` (Bun `bun:ffi` + TypeScript) |
| Examples | `examples/` (runnable with `bun run`) |

## The 6 FFI functions

| Function | Purpose |
|---|---|
| `matugen_extract_colors` | Generate a Material You + base16 palette from an image or explicit color. Returns color strings in the requested format. |
| `matugen_get_source_colors` | Return the ranked list of candidate source colors from an image, without picking one. |
| `matugen_render_template` | Render a template to a string using a palette (never touches disk). Accepts either a previous `extractColors` result or an embedded `source` to regenerate. |
| `matugen_render_from_image` | **Single-call extract+render**: takes an image source + template, runs extraction internally, and renders the template. Color format is auto-inferred by the template engine. Returns `{ rendered, colors, base16, source_color }`. |
| `matugen_write_output` | Write rendered content to disk, optionally running `pre_hook`/`post_hook`. |
| `matugen_free_string` | Release a `char*` returned by any of the above. |

## Notable implementation details

### `Cargo.toml`
- `[lib]` with `crate-type = ["cdylib", "rlib"]`, lib name `matugen_ffi`.
- `[[bin]] name = "matugen"` pointing at `src/main.rs`.
- An `ffi` feature (in `default`) gates `pub mod ffi;`.
- `serde_json` is a plain (non-optional) dependency — the FFI
  request/response structs need it unconditionally.

### `matugen-ffi/src/main.rs`
A ~20-line CLI wrapper:
```rust
let args = Cli::parse();
setup_logging(&args)?;
let state = State::new(args.clone())?;
if args.show_source_colors.is_some_and(|x| x) { return Ok(()); }
state.run_in_term()?;
```
All the actual logic lives in `matugen-ffi/src/lib.rs`.

### `matugen-ffi/src/lib.rs`
`pub mod` declarations for every module, `struct State` + `impl State`
(`new`, `init_engine`, `get_render_data`, `add_engine_filters`,
`run_in_term`, etc.), and `normalize_path_to_forward_slash`. Also:
```rust
#[cfg(feature = "ffi")]
pub mod ffi;
```

### `matugen-ffi/src/ffi.rs`
The entire FFI surface. Key points if you're editing it:
- Every `extern "C"` function is wrapped in `run_ffi()`, which does
  `catch_unwind`, converts the incoming `*const c_char` to a `&str`, and
  always returns a well-formed `{"ok": true, ...}` / `{"ok": false, "error":
  "..."}` JSON string as a freshly `CString::into_raw`'d pointer — even on
  panic (`{"ok":false,"error":"internal panic: ..."}`).
- `matugen_free_string` is the only way to release those pointers
  (`CString::from_raw` + drop). Never let a `char*` from this module reach a
  host-side `free()`.
- Local `Ffi*` wrapper enums (`FfiSchemeType`, `FfiSchemesEnum`,
  `FfiFilterType`, `FfiBackend`) exist because the "real" `SchemeTypes` /
  `SchemesEnum` / `FilterType` / `Backend` types derive a plain
  (PascalCase) `serde::Deserialize` (they're primarily driven by
  `clap::ValueEnum`), whereas the JSON contract uses kebab-case/
  lowercase strings (`"scheme-tonal-spot"`, `"dark"`, `"catmull-rom"`,
  `"wal"`). If you add a new CLI enum to the request JSON, you'll likely
  need the same kind of wrapper — don't just reuse the clap type directly.
- `run_extraction()` is the single source of truth for turning an
  `ExtractRequest` into `Option<Schemes>`/`Option<Theme>`/etc. It's called
  by `matugen_extract_colors`, by `resolve_render_context()` when a
  `matugen_render_template` request embeds a `"source"`, and by
  `render_from_image_impl()` for the combined extract+render path.
- `render_from_image_impl()` implements `matugen_render_from_image`:
  runs `run_extraction()` to get raw `Argb` schemes, builds the template
  context via `merge_json_source()`, sets up an `Engine` with filters and
  syntax, then delegates to the shared `run_render()` helper.
- `run_render()` is a helper shared by `render_template_impl()` and
  `render_from_image_impl()`. It reads the template source, adds it to
  the engine via `try_add_template`, and renders it.
- `parse_color_value()` / `schemes_from_color_map()` parse color strings
  (hex, rgb, rgba, hsl, hsla, etc.) into `Argb` using `parse_css_color()`.
  This replaced the old `schemes_from_hex_map()` which only handled hex,
  meaning the two-step `extractColors` → `renderTemplate` path now works
  with any `color_format`.
- `register_default_filters()` duplicates `State::add_engine_filters`'s
  filter list because the FFI render path builds a bare `Engine` with no
  `State` at all. **If you add a filter to `State::add_engine_filters` in
  `lib.rs`, add it here too**, or it silently won't be available from
  `matugen_render_template` or `matugen_render_from_image`.

### `matugen-ffi/src/template.rs`
- `format_hook(...) -> Result<HookOutput, Report>`.
  `HookOutput { stdout, stderr, exit_code }` carries the command's output.
  Hook-compile failures return `Err(...)` instead of exiting the process.
  CLI call sites in `TemplateFile::generate` ignore the returned
  `HookOutput` and still bubble errors with `.wrap_err(...)`.
- `TemplateFile::export_template`'s write logic (prefix handling,
  `create_missing_folders`, read-only check, `OpenOptions` + `write_all`) is
  a standalone, pure function:
  `pub fn write_to_disk(content: &str, output_path_absolute: &Path, prefix:
  &Option<PathBuf>, create_missing_dirs: bool) -> Result<Option<PathBuf>,
  Report>`. `export_template` calls it, and so does `matugen_write_output`
  in `ffi.rs`.
- `export_template`'s render-error branch returns `Err(Report::msg(...))`
  instead of calling `std::process::exit(1)` (still respects
  `continue_on_error`).

### `matugen-ffi/src/parser/engine.rs`
- `Engine::add_template` (CLI-only) prints an ariadne report via
  `show_errors` and calls `std::process::exit(1)` on a parse error.
- `Engine::try_add_template(&mut self, name, source) -> Result<(), String>`:
  same parsing logic, but returns a joined error message instead of
  printing/exiting. **This is the variant `src/ffi.rs` uses** — never call
  `add_template` from FFI-reachable code.
- `Engine::compile` calls `try_add_template` (not `add_template`) and, on
  failure, stuffs the message into an `Error::TemplateNotFound` so the
  existing `Result<String, Vec<Error>>` signature (and all current callers,
  including `format_hook`) keeps working unchanged.

### `src/` (Bun/TypeScript bindings)
- `src/types.ts` — request/response types mirroring `matugen-ffi/src/ffi.rs`'s
  JSON contract.
- `src/matugen.ts` — the `Matugen` class: loads `libmatugen_ffi.{so,dylib,dll}`,
  encodes requests as NUL-terminated buffers (`FFIType.ptr`, **not**
  `FFIType.cstring`, which does not auto-convert JS strings on the way *in*
  — only `returns: FFIType.cstring` auto-converts on the way *out*, and this
  code uses `FFIType.ptr` + `CString` on the return side anyway so it can
  free the pointer explicitly), and always calls `matugen_free_string`.
- `src/api.ts` — module-level singleton API (primary entry point for most consumers).
- `src/ffi/index.ts` — low-level FFI symbol definitions, library resolution, `callFfi` helper.
- `matugen-ffi/cbindgen.toml` + `matugen-ffi/matugen.h` — the generated C
  header; regenerate with
  `cbindgen --config cbindgen.toml --crate matugen-ffi --output matugen.h`
  whenever `matugen-ffi/src/ffi.rs`'s public signatures change.

## Invariants to preserve

1. **No `std::process::exit()` or panic must be reachable from `matugen-ffi/src/ffi.rs`.**
   The only remaining `process::exit` in the crate is inside
   `Engine::add_template`, which is CLI-only. If you add a new FFI code path
   that touches template rendering, use `try_add_template`, not
   `add_template`. If you add a new call into `format_hook`, remember it
   returns `Result<HookOutput, Report>` — propagate the error, don't unwrap.
2. **The CLI (`cargo run --bin matugen`) should keep working as a normal
   CLI** (pretty ariadne error output, exit-on-error semantics) — it's kept
   around for convenience/testing, but the FFI path is what matters most and
   must never exit/panic.
3. **Keep `register_default_filters` (`ffi.rs`) and `add_engine_filters`
   (`lib.rs`) in sync.** There is currently no single source of truth for
   the filter list; this is a known duplication (see the PRD's "Required
   Code Changes" section — a future cleanup could hoist this into a shared
   function).
4. **JSON enum casing**: any new clap/serde enum exposed through
   `matugen-ffi/src/ffi.rs`'s request/response structs needs a kebab-case/lowercase
   `Ffi*` wrapper (see `matugen-ffi/src/ffi.rs` comments) unless the original type
   already derives a matching `Deserialize` (e.g. `SelectionPreference`
   already uses `#[serde(rename_all = "kebab-case")]`, so it's used directly
   with no wrapper).
5. **Regenerate `matugen.h`** after any change to the 6 `extern "C"`
   function signatures in `matugen-ffi/src/ffi.rs`:
   `cbindgen --config cbindgen.toml --crate matugen-ffi --output matugen.h`.

## Building & testing

```sh
# CLI binary:
cd matugen-ffi
cargo build --release
./target/release/matugen color hex '#4287f5'

# FFI library:
cd matugen-ffi
cargo build --release --features ffi
# -> target/release/libmatugen_ffi.{so,dylib,dll}

# Unit tests:
cd matugen-ffi
cargo test --lib --features ffi

# Bun/TypeScript bindings (requires the library built above):
bun install
bun run examples/example.ts
bun run examples/example-image.ts
bun run examples/load-template.ts
```

A clean `cargo build --release` (no `--features ffi`) and
`cargo build --release --features ffi` should both finish with **zero
warnings**. If you see a warning, it usually means an FFI-only helper (e.g.
`try_add_template`) drifted out of sync with the CLI-only path (e.g.
`add_template`) that used to share its logic — check whether both paths
still need the same helper (like `show_errors`) before deleting anything.
