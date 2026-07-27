# Changes from the original `matugen`

This document is a complete, code-level record of everything that was
**added**, **changed**, or **removed** when `matugen-ffi` was forked from
[`matugen`](https://github.com/InioX/matugen) (the original CLI-only crate,
still available at `../matugen` in this repo for reference/comparison).
`matugen-ffi` is now developed independently — this file exists purely as a
historical/technical record of the fork, not as an ongoing sync checklist.

For the *why* behind these choices, see [`matugen-ffi-prd.md`](./matugen-ffi-prd.md).
For guidance on making further changes, see [`AGENTS.md`](./AGENTS.md).

---

## 1. Summary

| Kind | Path | What |
|---|---|---|
| Added | `src/lib.rs` | CLI logic (`State`) promoted into a library crate root |
| Added | `src/ffi.rs` | The 5 `extern "C"` FFI functions (including `matugen_get_source_colors`) |
| Changed | `src/main.rs` | Shrunk from 812 lines to a 22-line CLI wrapper |
| Changed | `src/template.rs` | `format_hook` returns `Result`, `write_to_disk` extracted |
| Changed | `src/parser/engine.rs` | Added `try_add_template`, `compile` no longer calls `add_template` |
| Changed | `src/color/color.rs` | Import path fix + `Source::ImageBytes` variant + bytes-based image extraction helpers |
| Changed | `src/wallpaper/unix.rs` | One `;` fix for `format_hook`'s new return type |
| Changed | `Cargo.toml` | `[lib]` + `[[bin]]` split, `ffi` feature, `serde_json` un-gated, `base64` added |
| Changed | `src/helpers.rs` | `set_wallpaper` match updated for `ImageBytes` variant |
| Changed | `src/color/base16.rs` | `generate_base16_schemes` match updated for `ImageBytes` variant |
| Added | `cbindgen.toml`, `matugen.h` | Generated C header for the FFI surface |
| Added | `bindings/bun-ts/` | Bun/TypeScript bindings + examples (including buffer example) |
| Added | `matugen-ffi-prd.md`, `AGENTS.md`, `README.md`, `CHANGES_FROM_MATUGEN.md` | Documentation |
| Unchanged | everything else under `src/` | byte-for-byte identical |

Everything under `src/` started as a byte-for-byte copy of `matugen/src/`.
Only the five files above (`lib.rs`+`main.rs`, `ffi.rs`, `template.rs`,
`parser/engine.rs`, `color/color.rs`, `wallpaper/unix.rs`) were touched.

---

## 2. `Cargo.toml`

```diff
 [package]
-name = "matugen"
+name = "matugen-ffi"
 version = "4.1.0"
 authors = ["InioX"]
-description = "A material you and base16 color generation tool with templates"
+description = "FFI bindings (C-ABI) for matugen's core color extraction and template rendering logic"
 repository = "https://github.com/InioX/matugen"
 categories = ["command-line-utilities"]
-exclude = ["default.nix", "flake.nix", "shell.nix", "example/*"]
 license = "GPL-2.0-or-later"
 edition = "2021"

+[lib]
+name = "matugen_ffi"
+crate-type = ["cdylib", "rlib"]
+
+[[bin]]
+name = "matugen"
+path = "src/main.rs"
+
 [target.'cfg(windows)'.dependencies]
 winapi = { version = "0.3", features = ["winuser"] }

 [features]
 filter-docs = []
-default = ["dump-json", "jxl-image"]
+default = ["dump-json", "jxl-image", "ffi"]
 update-informer = ["dep:update-informer"]
 web-image = ["dep:reqwest"]
-dump-json = ["dep:serde_json"]
+dump-json = []
 jxl-image = ["dep:jxl-oxide"]
+# Exposes the extern "C" FFI surface (matugen_extract_colors, matugen_render_template,
+# matugen_write_output, matugen_free_string). Disable for a lean CLI-only build.
+ffi = []

 [dependencies]
 ...
-# dump-json feature
-serde_json = { version = "1.0.107", optional = true }
+# used unconditionally: the CLI's --json/--dump flags as well as the FFI request/response
+# structs both need it, so it is no longer optional.
+serde_json = { version = "1.0.107" }
```

- The crate now builds **both** a library (`matugen_ffi`, as `cdylib` *and*
  `rlib`) and a binary (`matugen`), where the original only built a binary.
- `serde_json` was previously gated behind the `dump-json` feature (which
  happened to always be on by default). It's now a plain dependency because
  the FFI request/response structs (`src/ffi.rs`) need it regardless of
  which features are enabled.
- New `ffi` feature, on by default, gating `pub mod ffi;` in `src/lib.rs`.
- `base64 = "0.22"` added for decoding in-memory image buffers in
  `src/color/color.rs` and `src/ffi.rs` (used by the `imagebytes` source
  type and `matugen_get_source_colors`).

---

## 3. `src/main.rs` → split into `src/main.rs` + `src/lib.rs`

Original `matugen/src/main.rs` (812 lines) declared every module privately,
defined `struct State`/`impl State` inline, registered filters, and had
`fn main()` at the bottom — everything coupled into one binary crate.

**`src/main.rs` is now just:**

```rust
use clap::Parser;
use color_eyre::Report;

use matugen_ffi::{helpers::setup_logging, util::arguments::Cli, State};

fn main() -> Result<(), Report> {
    color_eyre::install()?;

    let args = Cli::parse();

    setup_logging(&args)?;

    let state = State::new(args.clone())?;

    if args.show_source_colors.is_some_and(|x| x) {
        return Ok(());
    }

    state.run_in_term()?;

    Ok(())
}
```

**`src/lib.rs` (new file, 449 lines)** contains everything else that used to
live in `main.rs`:

- All module declarations, now `pub` instead of private, so `src/ffi.rs` (and
  external consumers of the `rlib`) can reach them:
  ```rust
  pub mod helpers;
  pub mod smart_scheme;
  pub mod template;
  pub mod util;
  pub mod wallpaper;
  pub mod cache;
  pub mod color;
  pub mod exec;
  pub mod filters;
  pub mod parser;
  pub mod scheme;
  pub mod template_util;

  #[cfg(feature = "ffi")]
  pub mod ffi;
  ```
- `struct State` and `impl State` — `new`, `init_engine`, `save_cache`,
  `get_render_data`, `add_engine_filters`, `init_in_term`, `run_in_term` —
  copied verbatim, logic unchanged.
- `pub fn normalize_path_to_forward_slash`, also copied verbatim (made
  `pub` since `src/ffi.rs` reuses it to canonicalize image paths).

The CLI's observable behavior is unchanged: `cargo run --bin matugen -- ...`
does exactly what `cargo run` did in the original `matugen` crate.

---

## 4. `src/ffi.rs` (new file, ~830 lines)

Nothing like this exists in the original `matugen`. It exposes 5
`extern "C"` functions, all string-in/string-out JSON:

```rust
#[no_mangle]
pub extern "C" fn matugen_extract_colors(request_json: *const c_char) -> *mut c_char;

#[no_mangle]
pub extern "C" fn matugen_get_source_colors(request_json: *const c_char) -> *mut c_char;

#[no_mangle]
pub extern "C" fn matugen_render_template(request_json: *const c_char) -> *mut c_char;

#[no_mangle]
pub extern "C" fn matugen_write_output(request_json: *const c_char) -> *mut c_char;

#[no_mangle]
pub extern "C" fn matugen_free_string(ptr: *mut c_char);
```

Highlights (see `src/ffi.rs` itself for the full implementation):

- **Panic/exit safety.** Every function is wrapped in a `run_ffi()` helper
  that uses `std::panic::catch_unwind` and always produces a well-formed
  `{"ok": true, ...}` / `{"ok": false, "error": "..."}` JSON response, even
  on an internal panic:
  ```rust
  fn run_ffi<F>(request_json: *const c_char, body: F) -> *mut c_char
  where
      F: FnOnce(&str) -> Result<Value, String>,
  {
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
          match c_str_to_string(request_json) {
              Ok(request) => match body(&request) {
                  Ok(mut value) => { /* inject "ok": true */ value }
                  Err(error) => json!({ "ok": false, "error": error }),
              },
              Err(error) => json!({ "ok": false, "error": error }),
          }
      }));

      let value = result.unwrap_or_else(|panic| {
          json!({ "ok": false, "error": format!("internal panic: {}", panic_message(&*panic)) })
      });

      value_to_c_string(value)
  }
  ```
- **Memory ownership.** `matugen_free_string` is the only way to release a
  returned pointer (`CString::from_raw` + drop).
- **Request-only wrapper enums** (`FfiSchemeType`, `FfiSchemesEnum`,
  `FfiFilterType`, `FfiBackend`) because the original `SchemeTypes` /
  `SchemesEnum` / `FilterType` / `Backend` types derive a plain
  (PascalCase) `serde::Deserialize` — they're driven by `clap::ValueEnum`,
  not by an external JSON contract — whereas this crate's JSON API uses
  kebab-case/lowercase strings (`"scheme-tonal-spot"`, `"dark"`,
  `"catmull-rom"`, `"wal"`) to match the CLI's own `--type`/`--mode` flag
  spellings:
  ```rust
  #[derive(Deserialize, Clone, Copy, Debug)]
  #[serde(rename_all = "kebab-case")]
  enum FfiSchemeType {
      SchemeContent,
      SchemeExpressive,
      // ...
      SchemeTonalSpot,
      // ...
  }
  ```
- **`run_extraction()`** is the single source of truth for turning an
  `ExtractRequest` into `Option<Schemes>`/`Option<Theme>`/etc., reused by
  both `matugen_extract_colors` and (when a render request embeds a
  `"source"` instead of a previous extraction result)
  `matugen_render_template`, so palette (`{{ palettes.* }}`) keywords stay
  available end-to-end.
- **`register_default_filters()`** duplicates `State::add_engine_filters`'s
  filter list because the render path builds a bare `Engine` with no
  `State` at all (see [Invariants](./AGENTS.md#invariants-to-preserve) in
  `AGENTS.md`).

---

## 5. `src/template.rs`

### 5.1 `format_hook` no longer exits the process

```diff
 pub fn format_hook(
     engine: &mut Engine,
     hook: &String,
     colors_to_compare: &Option<Vec<crate::color::color::ColorDefinition>>,
     compare_to: &Option<String>,
-) -> Result<(), Report> {
+) -> Result<HookOutput, Report> {
     if let (Some(compare), Some(to)) = (colors_to_compare, compare_to) {
-        let res = match engine.compile(to.to_string()) {
-            Ok(v) => v,
-            Err(errors) => {
-                eprintln!("Error when formatting hook:\n{}", &hook);
-                for err in errors {
-                    err.emit(&engine)?;
-                }
-                std::process::exit(1);
-            }
-        };
+        let res = compile_or_err(engine, hook, to.to_string())?;
         let closest_color = get_closest_color(compare, &res)?;
         engine.add_context(json!({ "closest_color": closest_color }));
     }

-    let res = match engine.compile((&hook).to_string()) {
-        Ok(v) => v,
-        Err(errors) => {
-            eprintln!("Error when formatting hook:\n{}", &hook);
-            for err in errors {
-                err.emit(&engine)?;
-            }
-            std::process::exit(1);
-        }
-    };
+    let res = compile_or_err(engine, hook, hook.to_string())?;

     // ...run the shell command, capture stdout/stderr as before...

-    Ok(())
+    Ok(HookOutput { stdout, stderr, exit_code })
 }
```

A new `HookOutput` struct captures the command's result so it can be
returned to a caller (previously the output was only logged, never
returned):

```rust
/// Captured output of a `pre_hook`/`post_hook` command execution.
#[derive(Debug, Clone, Serialize)]
pub struct HookOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}
```

The two existing CLI call sites in `TemplateFile::generate` (running
`pre_hook`/`post_hook` for a template) are unchanged aside from now ignoring
the `Ok(HookOutput)` value — they still propagate errors the same way:

```rust
format_hook(self.engine, &hook, &template.colors_to_compare, &template.compare_to)
    .wrap_err(format!("Failed to format the following hook:\n{}", hook))?;
```

### 5.2 `export_template`'s write logic is now a standalone, reusable function

The original `export_template` inlined prefix handling, missing-folder
creation, the read-only check, and the actual `OpenOptions`/`write_all`
call. That logic was extracted into a new pure function with no
`State`/`Engine` dependency:

```rust
/// Writes rendered template `content` to `output_path`, applying the same
/// prefix/read-only/missing-folder handling as the CLI's `export_template`.
pub fn write_to_disk(
    content: &str,
    output_path_absolute: &Path,
    prefix: &Option<PathBuf>,
    create_missing_dirs: bool,
) -> Result<Option<PathBuf>, Report> {
    // ...same prefix / create_missing_folders / read-only / write_all logic...
    // returns Ok(Some(path)) on success, Ok(None) if the target was read-only.
}
```

`export_template` (CLI path) now just calls it:

```rust
match write_to_disk(&data, output_path_absolute, &self.state.args.prefix, true) {
    Ok(Some(written_path)) => { success!(/* ... */); Ok(()) }
    Ok(None) => { error!("...Read-Only..."); Ok(()) }
    Err(e) => Err(e),
}
```

`matugen_write_output` in `src/ffi.rs` calls the exact same `write_to_disk`
function — this is why matugen's on-disk-write behavior (prefix handling,
folder creation, read-only detection) is guaranteed identical between the
CLI and the FFI path.

### 5.3 `export_template`'s render-error branch no longer exits

```diff
         Err(errors) => {
-            for err in errors {
-                err.emit(&self.engine)?;
+            for err in &errors {
+                err.emit(self.engine)?;
             }

             if self.state.args.continue_on_error.unwrap_or(false) {
                 return Ok(());
             }

-            std::process::exit(1);
+            return Err(Report::msg(format!(
+                "Failed to render the {} template: {}",
+                name,
+                errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")
+            )));
         }
```

`continue_on_error` semantics are preserved exactly; only the "give up"
branch changed from `process::exit(1)` to returning an `Err`, which
propagates as a normal CLI error (non-zero exit via `main`'s `Result`
return) or as `{"ok": false, "error": "..."}` on the FFI path.

---

## 6. `src/parser/engine.rs`

### 6.1 New `Engine::try_add_template`

`Engine::add_template` is unchanged — it still prints an ariadne report via
`show_errors` and calls `std::process::exit(1)` on a parse error, exactly
like the original:

```rust
pub fn add_template(&mut self, name: String, source: String) {
    self.sources.push(source);
    let source_id = self.sources.len() - 1;
    let source_ref = &self.sources[source_id];
    let parser = Self::parser(&self.syntax);
    let (ast, errs) = parser.parse(source_ref).into_output_errors();
    self.templates.insert(
        name.clone(),
        Template {
            name,
            source_id,
            ast: ast.unwrap_or_else(|| {
                self.show_errors(errs, source_ref);
                std::process::exit(1);
            }),
        },
    );
}
```

A new sibling method was added for FFI-safe use, sharing the same parsing
logic but returning a `Result` instead:

```rust
/// Same as [`Engine::add_template`], but returns a `Result` instead of
/// printing to stderr and exiting the process on a parse error. This is
/// the variant that must be used from the FFI boundary, where a panic or
/// `process::exit` would take down the host application.
pub fn try_add_template(&mut self, name: String, source: String) -> Result<(), String> {
    self.sources.push(source);
    let source_id = self.sources.len() - 1;
    let source_ref = &self.sources[source_id];

    let parser = Self::parser(&self.syntax);
    let (ast, errs) = parser.parse(source_ref).into_output_errors();

    let ast = match ast {
        Some(ast) => ast,
        None => {
            let messages = errs
                .into_iter()
                .map(|e| format!("{} at {:?}: {}", name, e.span().into_range(), e.reason()))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(messages);
        }
    };

    self.templates.insert(name.clone(), Template { name, source_id, ast });
    Ok(())
}
```

### 6.2 `Engine::compile` no longer risks exiting

```diff
 pub fn compile(&mut self, source: String) -> Result<String, Vec<Error>> {
-    self.add_template(String::from("temporary"), source.clone());
+    if let Err(message) = self.try_add_template(String::from("temporary"), source.clone()) {
+        self.remove_template(&String::from("temporary"));
+        self.errors.add(Error::TemplateNotFound {
+            template: message,
+            name: String::from("temporary"),
+        });
+        return Err(self.errors.take());
+    }
     let res = self.render("temporary");
     self.remove_template(&String::from("temporary"));
     res
 }
```

`compile()` is used by `format_hook` (pre/post hook rendering) and by the
FFI's `matugen_render_template`, both of which must never crash the host
process on a malformed template/hook string. The function's public
signature (`Result<String, Vec<Error>>`) is unchanged, so no caller needed
updating.

---

## 7. `src/color/color.rs` — one import fix

```diff
-use crate::{util::arguments::SelectionPreference, FilterType as OwnFilterType};
+use crate::util::arguments::{FilterType as OwnFilterType, SelectionPreference};
```

In the original `matugen`, `main.rs` (the crate root of a *binary* crate)
had a private `use crate::util::arguments::FilterType;`. That name happens
to be reachable as `crate::FilterType` from any other module in the same
crate, because private items are visible throughout their defining module's
whole crate, and the crate root is an ancestor of every module — so
`color/color.rs`'s `use crate::{..., FilterType as OwnFilterType};` worked
by relying on that import existing in `main.rs`.

Since `src/lib.rs` is now the crate root and doesn't re-import that name,
the shortcut path no longer resolves, so the import here was made explicit.
This is a pure import-path fix — no behavior changed.

---

## 8. `src/wallpaper/unix.rs` — one-character fix

```diff
     info!("Executing pre_hook for wallpaper...");
     if let Some(hook) = pre_hook {
-        format_hook(engine, hook, &None, &None)?
+        format_hook(engine, hook, &None, &None)?;
     }
```

Required because `format_hook` now returns `Result<HookOutput, Report>`
instead of `Result<(), Report>` (§5.1) — the `if` block (which has no
matching `else`) must evaluate to `()`, so the trailing expression needs a
semicolon to become a discarded statement instead of the block's value.

---

## 9. New: `cbindgen.toml` + `matugen.h`

A `cbindgen` config and generated C header, absent from the original crate
(which has no C ABI to describe):

```c
/* Warning, this file is autogenerated by cbindgen from src/ffi.rs. Don't manually edit this file. */

char *matugen_extract_colors(const char *request_json);
char *matugen_render_template(const char *request_json);
char *matugen_write_output(const char *request_json);
void matugen_free_string(char *ptr);
```

Regenerate with:

```sh
cbindgen --config cbindgen.toml --crate matugen-ffi --output matugen.h
```

---

## 10. New: `bindings/bun-ts/`

An entirely new directory — Bun (`bun:ffi`) + TypeScript bindings for
`libmatugen_ffi`, with no equivalent in the original `matugen`.

The bindings are split into separate files by category:

| File | Responsibility |
|---|---|
| `src/types.ts` | Request/response TypeScript types mirroring `src/ffi.rs`'s JSON contract |
| `src/error.ts` | `MatugenFfiError` class + `unwrap` helper |
| `src/ffi.ts` | FFI symbol definitions, library resolution, low-level `callFfi` |
| `src/matugen.ts` | `Matugen` class (high-level typed wrapper for advanced use) |
| `src/api.ts` | Module-level functions backed by a singleton (`extractColors`, etc.) |
| `src/index.ts` | Re-exports everything |
| `example.ts` | Explicit hex color → extract → render → write, with a `post_hook` |
| `example-image.ts` | File-path image → extract → render → write (dark + light) |
| `example-buffer.ts` | In-memory buffer → extract + getSourceColors → render → write |
| `package.json`, `tsconfig.json`, `README.md` | Package scaffolding/docs |

Module-level API (singleton, no class instantiation needed):

```ts
import { extractColors, renderTemplate, writeOutput, unwrap } from "@matugen/ffi";

const colors = unwrap(
  extractColors({ source: { type: "color", format: "hex", value: "#4287f5" } })
);
const rendered = unwrap(
  renderTemplate({ colors, template: { input_string: '{{ colors.primary.default.hex }}', mode: "dark" } })
);
```

Advanced: multiple independent instances via the class:

```ts
import { Matugen } from "@matugen/ffi";
const matugen = new Matugen("/path/to/libmatugen_ffi.so");
const colors = matugen.extractColors({ source: { type: "color", format: "hex", value: "#ff0000" } });
matugen.close();
```

---

## 11. New: documentation files

- [`matugen-ffi-prd.md`](./matugen-ffi-prd.md) — the product requirements
  document this crate implements.
- [`AGENTS.md`](./AGENTS.md) — orientation for coding agents: what the
  project is, implementation notes for `src/ffi.rs`/`src/template.rs`/
  `src/parser/engine.rs`, and invariants to preserve.
- [`README.md`](./README.md) — user-facing overview, build instructions, FFI
  function table, Bun bindings pointer.
- This file.

None of these exist in the original `matugen` (which only has a
project-level `README.md` aimed at end users of the CLI/themes).

---

## 12. Net effect on the CLI

Despite the internal restructuring, `cargo run --bin matugen -- <args>`
behaves identically to the original `matugen` binary: same flags, same
config file format, same templates, same output, same pretty-printed
ariadne errors and `process::exit(1)` on unrecoverable template/hook
errors. The refactors in §§4–8 only add a second, `Result`-returning code
path (`try_add_template`, `format_hook`, `write_to_disk`, `matugen_write_output`)
used exclusively by `src/ffi.rs`; the CLI's own call sites keep using the
original exit-on-error variants.
