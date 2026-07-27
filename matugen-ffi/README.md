# matugen-ffi

`matugen-ffi` was originally forked from [`matugen`](https://github.com/InioX/matugen)
to add a minimal C-ABI surface (`libmatugen`) on top of its color-extraction
and template-rendering logic, so it can be reused from other host
applications (Bun/TypeScript, or any language with a C FFI) without needing
matugen's own `config.toml`. It's now developed and used as its own project.

This crate provides:

- `src/lib.rs` — the color-extraction/template-rendering engine (`State` and
  friends) as a library, with `src/main.rs` as a thin CLI wrapper around it.
- `src/ffi.rs` — the 5 `extern "C"` functions described below (only compiled
  when the `ffi` feature is enabled, which it is by default).
- No `std::process::exit()` reachable from the FFI boundary — every
  error path returns a `Result` that gets surfaced as JSON instead
  (`Engine::try_add_template`, `format_hook` returning
  `Result<HookOutput, Report>`, `template::write_to_disk` as a pure,
  reusable write function).

See [`matugen-ffi-prd.md`](./matugen-ffi-prd.md) for the full design
rationale (the PRD this crate implements), [`AGENTS.md`](./AGENTS.md) for
implementation notes and invariants to preserve when making changes, and
[`CHANGES_FROM_MATUGEN.md`](./CHANGES_FROM_MATUGEN.md) for a complete,
code-level record of every change made when this crate was forked from
`matugen`.

## Building

```sh
# CLI binary (`matugen`):
cargo build --release
./target/release/matugen color hex '#4287f5'

# Just the FFI library (also built by the command above):
cargo build --release --features ffi
# -> target/release/libmatugen_ffi.{so,dylib,dll}

# Regenerate the C header (requires `cargo install cbindgen`):
cbindgen --config cbindgen.toml --crate matugen-ffi --output matugen.h
```

## The FFI surface

5 functions, declared in [`matugen.h`](./matugen.h) and implemented in
[`src/ffi.rs`](./src/ffi.rs):

| Function | Purpose |
|---|---|
| `matugen_extract_colors` | image (path **or buffer**) / color → Material You + base16 palette (JSON) |
| `matugen_get_source_colors` | ranked candidate source colors from an image (path **or buffer**), without picking one |
| `matugen_render_template` | palette + template → rendered string (no disk I/O) |
| `matugen_write_output` | write a rendered string to disk, run `pre_hook`/`post_hook` |
| `matugen_free_string` | release a `char*` returned by any of the above |

### Image sources

Both `matugen_extract_colors` and `matugen_get_source_colors` accept a
`source` object. Two image variants are supported:

```jsonc
// From a file path:
{ "type": "image", "path": "/path/to/photo.jpg" }

// From an in-memory buffer (base64-encoded, no temp file needed):
{ "type": "imagebytes", "data_base64": "<base64-encoded bytes>" }
```

The `imagebytes` source avoids writing to disk entirely — useful when the
image was fetched over the network, generated in-process, or is already in
memory for any reason.

Every function is string-in / string-out JSON, never panics or calls
`std::process::exit` across the FFI boundary, and touches no global mutable
state (safe to call from multiple host threads in parallel). See the module
docs at the top of `src/ffi.rs` for the exact request/response JSON shapes,
or the Bun bindings below for a concrete example of each.

## Bun/TypeScript bindings

See [`bindings/bun-ts`](./bindings/bun-ts) for typed `bun:ffi` bindings,
split into separate files by category (`types.ts`, `error.ts`, `ffi.ts`,
`matugen.ts`, `api.ts`, `index.ts`) with a module-level API:

```ts
import { extractColors, renderTemplate, unwrap } from "@matugen/ffi";

const colors = unwrap(
  extractColors({ source: { type: "color", format: "hex", value: "#4287f5" } })
);
const rendered = unwrap(
  renderTemplate({ colors, template: { input_string: "{{ colors.primary.default.hex }}", mode: "dark" } })
);
```

Runnable examples:

| Example | What it demonstrates |
|---|---|
| `example.ts` | Explicit color → extract → render → write, with a `post_hook` |
| `example-image.ts` | File-path image → extract → render → write (dark + light) |
| `example-buffer.ts` | **In-memory buffer** → extract + getSourceColors → render → write |

```sh
cargo build --release --features ffi
cd bindings/bun-ts
bun install
bun run example.ts
bun run example:image
bun run example:buffer
```

## License

GPL-2.0-or-later, same as upstream `matugen`. Check the license implications
before linking `libmatugen_ffi.so`/`.dylib`/`.dll` into a closed-source
application.
