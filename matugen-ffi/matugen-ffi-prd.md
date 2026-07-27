# PRD: Matugen FFI Bindings (`libmatugen`)

## 1. Background

`matugen` is currently a pure binary crate (CLI only). Its whole flow — reading the TOML config, generating color schemes, rendering templates, writing files, running hooks — is coupled into a single `run_in_term()` call that assumes full ownership of I/O (reads its own config file, writes its own output files, prints to stdout).

We want to reuse **only the core logic** (color extraction + template rendering) from another host application (e.g. a custom theme manager) via FFI — **without** dragging along matugen's own config-file/multi-template/wallpaper orchestration. Parsing `config.toml` (`[catalog.x]`, `[templates.x]`) stays the host application's responsibility; matugen is called per-primitive instead.

## 2. Goals

Provide `libmatugen.so` / `.dylib` / `.dll` with a minimal C-ABI surface made of **3 core functions**:

1. **Extract color** — from an image or an explicit color → JSON palette (Material You + base16).
2. **Render template** — from a single template file/string + the result of #1 → **a string with the rendered content, without writing anything to disk**.
3. **Write output** — take the rendered string + a destination path, write it to disk, and (optionally) run `pre_hook`/`post_hook` exactly like in the TOML config (`post_hook = "bash '{{ config_dir }}/apply.sh'"`).

Plus one required support function: memory cleanup (`matugen_free_string`).

## 3. Non-Goals (Out of Scope for v1)

- Does not parse matugen's own `config.toml` (`[catalog.*]`, `[templates.*]`) — that's the host app's job.
- Does not perform wallpaper-setting (`src/wallpaper/`).
- Does not manage the image cache (`src/cache.rs`) — can be a v2 item if repeated-run speedups are needed.
- Does not provide interactive prompting (choosing a source color out of several image candidates) — v1 goes straight from a default/index/preference sent by the caller, no TTY prompt.
- Does not expose all 40+ CLI flags; only the options relevant to the two core functions.

## 4. Boundary Design Principles

- **String-in / String-out (JSON)** on every function — the most portable choice for FFI (ctypes, Bun FFI, Dart FFI, JNI, etc.), and it avoids Rust struct-ABI churn across versions.
- **No hidden global state** — every call is self-contained and safe to call from multiple host threads in parallel.
- **No `process::exit()` / panics crossing the FFI boundary** — every error path is refactored into a `Result` that gets converted to a JSON error, wrapped in `catch_unwind`.
- **Clear ownership**: every `*mut c_char` returned by Rust **must** be released via `matugen_free_string`; documented explicitly so host bindings don't leak.

## 5. Function Specification

### 5.1 `matugen_extract_colors`

```c
char* matugen_extract_colors(const char* request_json);
```

**Request JSON:**

```jsonc
{
  "source": {
    "type": "image", // "image" | "color" | "json"
    "path": "/abs/path/wall.png",
    // or, when type == "color":
    // "format": "hex" | "rgb" | "hsl", "value": "#4287f5"
  },
  "scheme_type": "scheme-tonal-spot", // default: scheme-tonal-spot
  "contrast": 0.0, // optional, -1..1
  "lightness_dark": 0.0, // optional
  "lightness_light": 0.0, // optional
  "resize_filter": "lanczos3", // optional, used when source = image
  "source_color_index": null, // optional, 0-3, skips the candidate-color prompt
  "prefer": null, // optional: "darkness"|"lightness"|"saturation"|...
  "fallback_color": null, // optional hex, used when extraction fails
  "base16_backend": "wal", // optional
  "opacity": 1.0, // optional 0..1
}
```

**Response JSON (success):**

```jsonc
{
  "ok": true,
  "colors": {
    "dark": { "primary": "#...", "on_primary": "#...", "...": "..." },
    "light": { "primary": "#...", "...": "..." },
  },
  "base16": {
    "dark": { "base00": "#...", "...": "..." },
    "light": { "...": "..." },
  },
  "source_color": "#4287f5",
  "image": "/abs/path/wall.png", // null when source is not an image
}
```

**Response JSON (failure):**

```json
{ "ok": false, "error": "Could not read image at ..." }
```

**Reused logic:** `State::new()` (the schemes/theme-generation part, before the `init_engine`/template stage), `helpers::generate_schemes_and_theme`, `util::color::format_schemes` / `rgb_from_argb`. This function **stops** before reading `config_file.templates`, so it needs no `config.toml` at all — the arguments above are sufficient.

---

### 5.2 `matugen_render_template`

```c
char* matugen_render_template(const char* request_json);
```

Combines the result of §5.1 with one template and **renders it to a string only** (never touches disk). Equivalent to `engine.render(name)` in `src/template.rs`, without going on to `export_template`.

**Request JSON:**

```jsonc
{
  "colors": {
    /* ...result from matugen_extract_colors, OR resend "source" as in §5.1... */
  },
  "template": {
    "input_path": "/abs/path/bat.tmTheme", // or "input_string" with the raw template content
    "mode": "dark", // optional, overrides the default scheme
    "type": "scheme-tonal-spot", // optional, overrides the scheme for this template
    "block_prefix": null,
    "block_postfix": null, // optional, custom syntax override
    "expr_prefix": null,
    "expr_postfix": null,
  },
  "custom_keywords": { "any_key": "any_value" }, // optional, equivalent to --import-json-string
}
```

> Note: `colors` and its generation details are kept flexible — it may accept the object returned by §5.1 directly (saving one re-generation of the palette), or accept a raw `source` when the caller hasn't extracted colors yet.

**Response JSON:**

```jsonc
{ "ok": true, "rendered": "...the rendered file content as a string..." }
```

or

```jsonc
{ "ok": false, "error": "line 12: unexpected token ..." }
```

**Reused logic:** `Engine::new()`, `engine.set_syntax(...)`, `engine.add_context(json)`, `engine.add_template(name, data)`, `engine.render(name)` — without `export_template`. `pre_hook`/`post_hook` are **not** executed here (moved to §5.3, since hooks only make sense once a file has actually been written or deliberately skipped).

---

### 5.3 `matugen_write_output`

```c
char* matugen_write_output(const char* request_json);
```

**Request JSON:**

```jsonc
{
  "content": "...string from matugen_render_template...",
  "output_path": "/abs/path/or/$XDG_CONFIG_HOME/bat/themes/noctalia.tmTheme",
  "create_missing_dirs": true, // default true
  "pre_hook": null, // optional command string (may contain {{ }} template syntax)
  "post_hook": "bash '{{ config_dir }}/apply.sh'", // optional
  "hook_context": { "config_dir": "/home/user/.config/bat" }, // extra variables for hook rendering
}
```

**Response JSON:**

```jsonc
{
  "ok": true,
  "written_path": "/home/user/.config/bat/themes/noctalia.tmTheme",
  "pre_hook_output": { "stdout": "...", "stderr": "...", "exit_code": 0 },
  "post_hook_output": { "stdout": "...", "stderr": "...", "exit_code": 0 },
}
```

**Reused logic:** the second half of `TemplateFile::export_template` (create_missing_folders, read-only check, `OpenOptions`, `write_all`) + `format_hook` (for pre/post hooks, including `expand_tilde` and `$XDG_CONFIG_HOME`-style env-var expansion).

**"Optional output" behavior:** because §5.2 and §5.3 are now separate, the host app is free by design: call §5.2 only (preview / dry-run, nothing written), or call §5.2 then §5.3 (actually write the file). There's no need for an `output_path: null` flag inside one combined function — the split into two functions **is** the implementation of "optional output".

---

### 5.4 `matugen_free_string`

```c
void matugen_free_string(char* ptr);
```

Must be called by the host for every `char*` received from 5.1–5.3. No-op if `ptr == NULL`.

## 6. Non-Functional Requirements

| Aspect           | Requirement                                                                                                                                                                                                           |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Panic safety     | Every `extern "C"` function is wrapped in `catch_unwind`; panics are converted to `{"ok":false,"error":"internal panic: ..."}` and must never unwind across the FFI boundary.                                         |
| Thread-safety    | No mutable Rust global state; safe to call from many host threads in parallel. `color_eyre::install()` (if kept at all) is called at most once via `std::sync::Once`, or avoided entirely on the FFI path.            |
| Memory ownership | Explicitly documented: every returned `char*` **must** be released by the caller via `matugen_free_string`. Ownership never flows the other way (the host must never `free()` a Rust pointer with its own allocator). |
| Error surface    | No `std::process::exit()` anywhere reachable from FFI (currently present in `template.rs` / `format_hook` — must be refactored into `Result`).                                                                        |
| Encoding         | All strings UTF-8. Non-UTF8 paths (rare on Linux/macOS, possible on Windows) are returned as an error, not a crash.                                                                                                   |
| Platform         | Linux `.so`, macOS `.dylib`, Windows `.dll`, all built from the same crate (`crate-type = ["cdylib","rlib"]`); the old CLI binary can still be built alongside from the `rlib`.                                       |
| License          | matugen is **GPL-2.0-or-later** — verify before linking this `.so` into a closed-source application, since distribution may trigger copyleft obligations.                                                             |

## 7. Required Code Changes (summary)

1. `Cargo.toml`: add `[lib]` with `crate-type = ["cdylib", "rlib"]`.
2. Move `struct State` and the schemes-generation functions into `src/lib.rs`, make them `pub`; `main.rs` becomes a thin CLI wrapper (behavior unchanged).
3. Refactor `format_hook` in `src/template.rs`: remove `std::process::exit(1)`, return `Result<HookOutput, Report>` instead.
4. Refactor `TemplateFile::export_template` into two pure functions — `render_only(name) -> Result<String>` (already exists as `engine.render(name)`) and `write_to_disk(
})path, content) -> Result<()>` (takes a plain string, no longer reaches into `self.engine`).
5. Create `src/ffi.rs` with the 4 `extern "C"` functions from §5, plus request/response structs deriving `Deserialize`/`Serialize`, kept separate from `Cli` (which is designed for `clap`, not external JSON).
6. New feature flag in `Cargo.toml`, e.g. `ffi = []`, so a plain CLI build doesn't pull in the extra surface unless needed.

## 8. Open Questions

- Should §5.1 and §5.2 be combinable into a single call (`extract + render` in one shot) for the "1 template, 1 render" case, to cut JSON round-trip overhead? (Proposal: **keep them separate** in v1 — a caller that generates one palette and renders many templates from it is already more efficient by reusing the §5.1 result across multiple §5.2 calls.)
- How should `InputPathModes` (`input_path_modes.light` / `.dark`) be represented in the §5.2 request JSON — supported in v1, or does the caller just call twice with a different `mode`?
- `colors_to_compare` / `compare_to` (used by `format_hook` for the "closest color" feature) — v1 or v2?

## 9. Milestones

| #   | Deliverable                                                                                        |
| --- | -------------------------------------------------------------------------------------------------- |
| M1  | `Cargo.toml` + `lib.rs` refactor, `cargo build --lib` succeeds, old CLI behaves identically        |
| M2  | `matugen_extract_colors` + unit tests (image & color source)                                       |
| M3  | `matugen_render_template` + unit tests (template from file & from string)                          |
| M4  | `matugen_write_output` (including pre/post hooks) + `matugen_free_string`                          |
| M5  | Example bindings: **Bun/TypeScript** (`bun:ffi`) + generated C header (`matugen.h`) via `cbindgen` |
| M6  | GPL license review for `.so` distribution                                                          |
