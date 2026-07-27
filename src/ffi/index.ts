/**
 * Low-level FFI bindings: symbol definitions, library resolution, and
 * the raw `callFfi` helper that encodes JSON over `char*` and frees
 * the returned pointer.
 */
import { dlopen, FFIType, suffix, CString, ptr, type Pointer } from "bun:ffi";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { MatugenFfiError } from "#/error";

// ---------------------------------------------------------------------------
// Symbol definitions
// ---------------------------------------------------------------------------
export const SYMBOLS = {
  matugen_extract_colors: {
    args: [FFIType.ptr],
    returns: FFIType.ptr,
  },
  matugen_get_source_colors: {
    args: [FFIType.ptr],
    returns: FFIType.ptr,
  },
  matugen_render_template: {
    args: [FFIType.ptr],
    returns: FFIType.ptr,
  },
  matugen_render_from_image: {
    args: [FFIType.ptr],
    returns: FFIType.ptr,
  },
  matugen_write_output: {
    args: [FFIType.ptr],
    returns: FFIType.ptr,
  },
  matugen_free_string: {
    args: [FFIType.ptr],
    returns: FFIType.void,
  },
} as const;

export type Lib = ReturnType<typeof dlopen<typeof SYMBOLS>>;

// ---------------------------------------------------------------------------
// Library resolution
// ---------------------------------------------------------------------------

/**
 * Locates `libmatugen_ffi.{so,dylib,dll}`.
 *
 * Resolution order:
 * 1. The explicit `libPath` argument, if provided.
 * 2. The `MATUGEN_FFI_LIB_PATH` environment variable.
 * 3. `../../target/release/` and `../../target/debug/` relative to this
 *    package (i.e. `matugen-ffi/target/{release,debug}`), which is where
 *    `cargo build [--release] --features ffi` puts the library by default.
 */
export function resolveLibPath(libPath?: string): string {
  if (libPath) return libPath;

  const envPath = process.env.MATUGEN_FFI_LIB_PATH;
  if (envPath) return envPath;

  const fileName = `libmatugen_ffi.${suffix}`;
  const candidates = [
    new URL(`./${fileName}`, import.meta.url),
    new URL(`../../matugen-ffi/target/release/${fileName}`, import.meta.url),
    new URL(`../../matugen-ffi/target/debug/${fileName}`, import.meta.url),
  ];

  for (const url of candidates) {
    const candidate = fileURLToPath(url);
    if (existsSync(candidate)) return candidate;
  }

  throw new Error(
    `Could not locate ${fileName}. Build it first with ` +
      "`cargo build --features ffi` (or `--release`) inside `matugen-ffi/`, " +
      "or point at it explicitly via the `MATUGEN_FFI_LIB_PATH` environment variable.",
  );
}

// ---------------------------------------------------------------------------
// Low-level call helper
// ---------------------------------------------------------------------------
type FfiFn = (requestPtr: Pointer | null) => Pointer | null;

/**
 * Encode a request as a NUL-terminated UTF-8 buffer, pass it to the native
 * function, parse the JSON response, and free the returned `char*`.
 *
 * `FFIType.ptr` args expect a native pointer, not a JS string, so we
 * encode the JSON request as a NUL-terminated UTF-8 buffer ourselves and
 * pass a pointer to it (`bun:ffi`'s `cstring` arg type is just an alias
 * for `ptr` and does *not* auto-convert JS strings on the way in, only
 * on the way out via `returns: FFIType.cstring`).
 */
export function callFfi<Req, Res>(lib: Lib, fn: FfiFn, request: Req): Res {
  const requestBuffer = Buffer.from(JSON.stringify(request) + "\0", "utf8");
  const resultPtr = fn(ptr(requestBuffer));

  if (resultPtr === null) {
    throw new MatugenFfiError(
      "matugen FFI call returned a null pointer (this should never happen; please report a bug)",
    );
  }

  try {
    const responseJson = new CString(resultPtr).toString();
    return JSON.parse(responseJson) as Res;
  } finally {
    lib.symbols.matugen_free_string(resultPtr);
  }
}
