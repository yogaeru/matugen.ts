/**
 * Request/response types mirroring the JSON contracts documented in
 * `matugen-ffi-prd.md` §5 and implemented in `../../src/ffi.rs`.
 */

export type ColorFormatKind = "hex" | "rgb" | "hsl";

export type SourceRequest =
  | { type: "image"; path: string }
  | { type: "imagebytes"; data_base64: string }
  | { type: "color"; format: ColorFormatKind; value: string }
  | { type: "json"; path: string };

export type SchemeType =
  | "scheme-content"
  | "scheme-expressive"
  | "scheme-fidelity"
  | "scheme-fruit-salad"
  | "scheme-monochrome"
  | "scheme-neutral"
  | "scheme-rainbow"
  | "scheme-tonal-spot"
  | "scheme-vibrant"
  | "scheme-smart";

export type SchemeMode = "light" | "dark" | "smart";

export type ResizeFilter =
  "nearest" | "triangle" | "catmull-rom" | "gaussian" | "lanczos3";

export type SelectionPreference =
  | "darkness"
  | "lightness"
  | "saturation"
  | "less-saturation"
  | "value"
  | "closest-to-fallback";

export type Base16Backend = "wal";

/** Output format for color values in the extractColors response. */
export type ColorOutputFormat = "hex" | "rgb" | "rgba" | "hsl" | "hsla";

export interface RenderFromImageRequest {
  /** Image source to extract colors from. */
  source: SourceRequest;
  scheme_type?: SchemeType;
  contrast?: number;
  lightness_dark?: number;
  lightness_light?: number;
  resize_filter?: ResizeFilter;
  source_color_index?: number;
  prefer?: SelectionPreference;
  fallback_color?: string;
  base16_backend?: Base16Backend;
  opacity?: number;
  /** Output format for color values in the `colors` response key. */
  color_format?: ColorOutputFormat;
  /** The template to render. */
  template: TemplateRequest;
  /** Optional extra template keywords. */
  custom_keywords?: Record<string, unknown>;
}

export interface RenderFromImageSuccess {
  ok: true;
  rendered: string;
  colors: SchemeColors | null;
  base16: SchemeColors | null;
  source_color: string | null;
}

export type RenderFromImageResponse = RenderFromImageSuccess | FfiError;

export interface ExtractColorsRequest {
  source: SourceRequest;
  scheme_type?: SchemeType;
  contrast?: number;
  lightness_dark?: number;
  lightness_light?: number;
  resize_filter?: ResizeFilter;
  source_color_index?: number;
  prefer?: SelectionPreference;
  fallback_color?: string;
  base16_backend?: Base16Backend;
  opacity?: number;
  /** Output format for color values (hex, rgb, rgba, hsl, hsla). Defaults to "hex". */
  color_format?: ColorOutputFormat;
}

export interface ColorMap {
  [name: string]: string;
}

export interface SchemeColors {
  dark: ColorMap;
  light: ColorMap;
}

export interface FfiError {
  ok: false;
  error: string;
}

export interface ExtractColorsSuccess {
  ok: true;
  colors: SchemeColors | null;
  base16: SchemeColors | null;
  source_color: string | null;
  image: string | null;
}

export type ExtractColorsResponse = ExtractColorsSuccess | FfiError;

export interface SourceColorsRequest {
  source: SourceRequest;
  resize_filter?: ResizeFilter;
  fallback_color?: string;
}

export interface SourceColorsSuccess {
  ok: true;
  source_colors: string[];
}

export type SourceColorsResponse = SourceColorsSuccess | FfiError;

export interface TemplateRequest {
  /** Absolute (or cwd-relative) path to the template file. Mutually
   * exclusive with `input_string`. */
  input_path?: string;
  /** Raw template source. Mutually exclusive with `input_path`. */
  input_string?: string;
  mode?: SchemeMode;
  block_prefix?: string;
  block_postfix?: string;
  expr_prefix?: string;
  expr_postfix?: string;
}

export interface RenderTemplateRequest {
  /**
   * Either the JSON object previously returned by `extractColors` (the whole
   * `{ ok, colors, base16, source_color, image }` object works, extra `ok`
   * field is ignored), or an `ExtractColorsRequest`-shaped object (i.e. one
   * that has a `source` key) to have matugen regenerate the palette from
   * scratch for this render call. The latter also exposes palette
   * (`{{ palettes.* }}`) keywords, which are not part of the simplified
   * `extractColors` JSON response.
   */
  colors: ExtractColorsSuccess | ExtractColorsRequest | Record<string, unknown>;
  template: TemplateRequest;
  custom_keywords?: Record<string, unknown>;
}

export interface RenderTemplateSuccess {
  ok: true;
  rendered: string;
}

export type RenderTemplateResponse = RenderTemplateSuccess | FfiError;

export interface HookOutput {
  stdout: string;
  stderr: string;
  exit_code: number | null;
}

export interface WriteOutputRequest {
  content: string;
  output_path: string;
  /** Defaults to `true` on the Rust side if omitted. */
  create_missing_dirs?: boolean;
  pre_hook?: string;
  post_hook?: string;
  hook_context?: Record<string, unknown>;
}

export interface WriteOutputSuccess {
  ok: true;
  written_path: string;
  pre_hook_output: HookOutput | null;
  post_hook_output: HookOutput | null;
}

export type WriteOutputResponse = WriteOutputSuccess | FfiError;
