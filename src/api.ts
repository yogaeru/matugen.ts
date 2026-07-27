/**
 * Module-level API backed by a singleton `Matugen` instance.
 *
 * This is the primary entry point for most consumers — import functions
 * directly without instantiating a class.
 */
import type {
  ExtractColorsRequest,
  ExtractColorsResponse,
  SourceColorsRequest,
  SourceColorsResponse,
  RenderTemplateRequest,
  RenderTemplateResponse,
  RenderFromImageRequest,
  RenderFromImageResponse,
  WriteOutputRequest,
  WriteOutputResponse,
} from "./types";

import { Matugen } from "./matugen";

const matugen = new Matugen();

/**
 * Extract a Material You + base16 palette from an image (path or buffer)
 * or an explicit color.
 *
 * @example
 * ```ts
 * import { extractColors, unwrap } from "@matugen/ffi";
 *
 * const colors = unwrap(
 *   extractColors({ source: { type: "color", format: "hex", value: "#4287f5" } })
 * );
 * console.log(colors.colors?.dark.primary);
 * ```
 */
export function extractColors(
  request: ExtractColorsRequest,
): ExtractColorsResponse {
  return matugen.extractColors(request);
}

/**
 * Return the ranked list of candidate source colors extracted from an image
 * (path or buffer), without picking one.
 */
export function getSourceColors(
  request: SourceColorsRequest,
): SourceColorsResponse {
  return matugen.getSourceColors(request);
}

/**
 * Render a template to a string using a palette (no disk I/O).
 */
export function renderTemplate(
  request: RenderTemplateRequest,
): RenderTemplateResponse {
  return matugen.renderTemplate(request);
}

/**
 * Extract colors from an image and render a template in a single call.
 * The color format is automatically inferred by the template engine,
 * so callers don't need to worry about matching extraction format
 * to template needs.
 */
export function renderFromImage(
  request: RenderFromImageRequest,
): RenderFromImageResponse {
  return matugen.renderFromImage(request);
}

/**
 * Write rendered content to disk, optionally running `pre_hook`/`post_hook`.
 */
export function writeOutput(request: WriteOutputRequest): WriteOutputResponse {
  return matugen.writeOutput(request);
}

/**
 * Release the dynamic library handle. Safe to skip; not calling it just
 * keeps the library mapped for the lifetime of the process.
 */
export function close(): void {
  matugen.close();
}
