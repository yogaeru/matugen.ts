# @matugen/ffi

Bun/TypeScript bindings for [matugen](https://github.com/InioX/matugen)'s
color-extraction and Material You template-rendering engine, exposed via a
C-ABI FFI surface (`libmatugen_ffi`).

## Features

- **Extract colors** from an image (path or bytes) or an explicit color into a
  Material You + base16 palette.
- **Render templates** using the extracted palette — supports the full matugen
  template engine (filters, palettes, base16, dark/light modes).
- **Single-call `renderFromImage`** — extract + render in one FFI call, with
  automatic color format inference (`.red`, `.green`, `.blue`, `.rgb`, `.hsl`,
  etc.) so callers never have to worry about matching formats.
- **Write output** to disk with optional pre/post hooks.

## Install

```bash
bun install
```

## Build the Rust library

```bash
cd matugen-ffi
cargo build --release --features ffi
# -> target/release/libmatugen_ffi.{so,dylib,dll}
```

## Usage

```ts
import {
  extractColors,
  renderTemplate,
  renderFromImage,
  writeOutput,
  unwrap,
} from "./src/index";

// --- Single-call extract + render (recommended) ---
const result = unwrap(
  renderFromImage({
    source: { type: "image", path: "./images/photo.jpg" },
    scheme_type: "scheme-tonal-spot",
    source_color_index: 0,
    template: { input_path: "./templates/inputs/steam/steam.css" },
  }),
);
console.log(result.rendered);

// --- Two-step: extract, then render ---
const colors = unwrap(
  extractColors({
    source: { type: "color", format: "hex", value: "#4287f5" },
  }),
);

const rendered = unwrap(
  renderTemplate({
    colors,
    template: { input_path: "./templates/inputs/steam/steam.css" },
  }),
);
console.log(rendered.rendered);
```

## Run the examples

```bash
bun run examples/example.ts          # explicit color → extract → render → write
bun run examples/example-image.ts    # image → extract → render (dark + light)
bun run examples/example-buffer.ts   # base64 image bytes pipeline
bun run examples/load-template.ts    # load templates from disk
```

## Project structure

| Path | Description |
|------|-------------|
| `src/` | Bun/TypeScript bindings (`bun:ffi` + TypeScript) |
| `matugen-ffi/` | Rust crate — the extraction and rendering engine |
| `examples/` | Runnable Bun examples |
| `templates/` | Sample template files for examples |
| `images/` | Sample images for examples |

## License

GPL-2.0-or-later
