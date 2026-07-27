// Module-level API (primary entry point)
export {
  extractColors,
  getSourceColors,
  renderTemplate,
  renderFromImage,
  writeOutput,
  close,
} from "./api";

// Class for advanced use (multiple instances, custom library path)
export { Matugen } from "./matugen";

// FFI utilities
export { resolveLibPath } from "./ffi";

// Error handling
export { MatugenFfiError, unwrap } from "./error";

// Types
export type {
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
  FfiError,
  ColorFormatKind,
  SourceRequest,
  SchemeType,
  SchemeMode,
  ResizeFilter,
  SelectionPreference,
  Base16Backend,
  ColorMap,
  SchemeColors,
  ExtractColorsSuccess,
  SourceColorsSuccess,
  TemplateRequest,
  RenderTemplateSuccess,
  RenderFromImageSuccess,
  HookOutput,
  WriteOutputSuccess,
} from "./types";
