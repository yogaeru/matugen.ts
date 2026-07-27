/**
 * High-level typed wrapper around `libmatugen`'s C-ABI surface.
 *
 * Every `char*` returned by the native library is read and freed
 * (`matugen_free_string`) before the corresponding method returns, so
 * callers never have to manage native memory themselves.
 */
import { dlopen } from "bun:ffi";

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

import { SYMBOLS, resolveLibPath, callFfi, type Lib } from "./ffi";

export class Matugen {
  #lib: Lib;

  constructor(libPath?: string) {
    this.#lib = dlopen(resolveLibPath(libPath), SYMBOLS);
  }

  extractColors(request: ExtractColorsRequest): ExtractColorsResponse {
    return callFfi(
      this.#lib,
      this.#lib.symbols.matugen_extract_colors,
      request,
    );
  }

  /** Returns the ranked list of candidate source colors extracted from an
   * image (or image bytes), without picking one. */
  getSourceColors(request: SourceColorsRequest): SourceColorsResponse {
    return callFfi(
      this.#lib,
      this.#lib.symbols.matugen_get_source_colors,
      request,
    );
  }

  renderTemplate(request: RenderTemplateRequest): RenderTemplateResponse {
    return callFfi(
      this.#lib,
      this.#lib.symbols.matugen_render_template,
      request,
    );
  }

  /**
   * Extract colors from an image and render a template in a single call.
   * The color format is automatically inferred by the template engine.
   */
  renderFromImage(request: RenderFromImageRequest): RenderFromImageResponse {
    return callFfi(
      this.#lib,
      this.#lib.symbols.matugen_render_from_image,
      request,
    );
  }

  writeOutput(request: WriteOutputRequest): WriteOutputResponse {
    return callFfi(this.#lib, this.#lib.symbols.matugen_write_output, request);
  }

  /** Releases the dynamic library handle. Safe to skip; not calling it just
   * keeps the library mapped for the lifetime of the process. */
  close(): void {
    this.#lib.close();
  }
}
