import type { FfiError } from "./types";

/** Error thrown when an FFI call fails or returns `{ ok: false }`. */
export class MatugenFfiError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MatugenFfiError";
  }
}

/** Narrows a response to its success variant, throwing `MatugenFfiError` on `{ ok: false }`. */
export function unwrap<T extends { ok: true }>(response: T | FfiError): T {
  if (!response.ok) {
    throw new MatugenFfiError(response.error);
  }
  return response;
}
