//! C-ABI surface for `libmatugen`.
//!
//! This module exposes 5 `extern "C"` functions:
//!
//! - [`matugen_extract_colors`] — generate a Material You + base16 palette from
//!   an image (path or raw bytes) or an explicit color.
//! - [`matugen_get_source_colors`] — return the ranked list of candidate
//!   source colors extracted from an image (path or raw bytes), without
//!   picking one.
//! - [`matugen_render_template`] — render a single template to a string
//!   (never touches disk).
//! - [`matugen_write_output`] — write rendered content to disk, optionally
//!   running `pre_hook`/`post_hook`.
//! - [`matugen_free_string`] — release a `char*` returned by any of the above.
//!
//! Both [`matugen_extract_colors`] and [`matugen_get_source_colors`] accept a
//! `source` of `{"type": "image", "path": "..."}` or
//! `{"type": "imagebytes", "data_base64": "..."}` (base64-encoded raw image
//! bytes) so callers don't need to write an in-memory image to disk first.
//!
//! Every function is string-in / string-out JSON, never panics or calls
//! `std::process::exit` across the FFI boundary (panics are caught and
//! reported as `{"ok":false,"error":"internal panic: ..."}`), and touches no
//! global mutable state, so it is safe to call from multiple host threads in
//! parallel.
//!
//! ## Memory ownership
//!
//! Every `char*` returned by [`matugen_extract_colors`],
//! [`matugen_get_source_colors`], [`matugen_render_template`] and
//! [`matugen_write_output`] **must** be
//! released by calling [`matugen_free_string`] exactly once. The host must
//! never call its own `free()` on these pointers.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::str::FromStr;

use indexmap::IndexMap;
use material_colors::{color::Argb, theme::Theme};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::register_filters;

use crate::{
    color::{
        base16::Backend as CrateBackend,
        color::{
            decode_base64_image, get_filter, get_scored_colors_from_image,
            get_scored_colors_from_image_bytes, ColorFormat, Source,
        },
        format::{
            argb_from_rgb, format_hex_alpha, format_hsl, format_hsla, format_rgb, format_rgba,
            hsl_from_argb, rgb_from_argb,
        },
        parse::parse_css_color,
    },
    helpers::{
        apply_opacity_to_schemes, generate_schemes_and_theme, get_syntax, merge_json,
        merge_json_source, parse_fallback_color,
    },
    parser::Engine,
    scheme::{SchemeTypes, Schemes, SchemesEnum},
    template::{format_hook, write_to_disk, HookOutput},
    util::{
        arguments::{Cli, FilterType as CrateFilterType, SelectionPreference},
        config::{Config, ConfigFile},
    },
};

// ---------------------------------------------------------------------------
// Shared plumbing: catch_unwind + JSON (de)serialization + C string handling
// ---------------------------------------------------------------------------

fn c_str_to_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("request_json pointer was null".to_string());
    }
    // SAFETY: caller (the host application) guarantees `ptr` is either null
    // (checked above) or a valid pointer to a NUL-terminated UTF-8 C string
    // that lives for the duration of this call.
    let c_str = unsafe { CStr::from_ptr(ptr) };
    c_str
        .to_str()
        .map(|s| s.to_owned())
        .map_err(|e| format!("Request was not valid UTF-8: {e}"))
}

fn value_to_c_string(value: Value) -> *mut c_char {
    let s = serde_json::to_string(&value).unwrap_or_else(|e| {
        format!(
            r#"{{"ok":false,"error":"Failed to serialize response: {}"}}"#,
            e
        )
    });
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => {
            // Should never happen: JSON output never contains interior NUL bytes.
            CString::new(r#"{"ok":false,"error":"internal error: response contained a NUL byte"}"#)
                .unwrap()
                .into_raw()
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Runs `body`, catching panics, and always returns a well-formed
/// `{"ok": true, ...}` / `{"ok": false, "error": "..."}` JSON string as a
/// freshly-allocated, caller-owned `char*`.
fn run_ffi<F>(request_json: *const c_char, body: F) -> *mut c_char
where
    F: FnOnce(&str) -> Result<Value, String>,
{
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match c_str_to_string(request_json) {
            Ok(request) => match body(&request) {
                Ok(mut value) => {
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert("ok".to_string(), Value::Bool(true));
                    }
                    value
                }
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

/// Releases a `char*` previously returned by [`matugen_extract_colors`],
/// [`matugen_render_template`] or [`matugen_write_output`]. No-op if `ptr` is
/// null. Must be called exactly once per returned pointer.
///
/// # Safety
/// `ptr` must either be null, or a pointer previously returned by one of this
/// crate's FFI functions that has not already been freed.
#[no_mangle]
pub extern "C" fn matugen_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: `ptr` was allocated by `CString::into_raw` in `value_to_c_string`,
    // per this function's documented contract.
    unsafe {
        drop(CString::from_raw(ptr));
    }
}

// ---------------------------------------------------------------------------
// Request-only enums (need `Deserialize`, unlike the clap-oriented originals)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "kebab-case")]
enum FfiFilterType {
    Nearest,
    Triangle,
    CatmullRom,
    Gaussian,
    Lanczos3,
}

impl From<FfiFilterType> for CrateFilterType {
    fn from(value: FfiFilterType) -> Self {
        match value {
            FfiFilterType::Nearest => CrateFilterType::Nearest,
            FfiFilterType::Triangle => CrateFilterType::Triangle,
            FfiFilterType::CatmullRom => CrateFilterType::CatmullRom,
            FfiFilterType::Gaussian => CrateFilterType::Gaussian,
            FfiFilterType::Lanczos3 => CrateFilterType::Lanczos3,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "kebab-case")]
enum FfiBackend {
    Wal,
}

impl From<FfiBackend> for CrateBackend {
    fn from(value: FfiBackend) -> Self {
        match value {
            FfiBackend::Wal => CrateBackend::Wal,
        }
    }
}

/// Mirrors [`SchemeTypes`], but with a `kebab-case` `Deserialize` impl so JSON
/// requests can use the same spelling as the CLI's `--type` flag (e.g.
/// `"scheme-tonal-spot"`). `SchemeTypes` itself derives a plain (PascalCase)
/// `Deserialize` because it is primarily driven by `clap::ValueEnum`.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "kebab-case")]
enum FfiSchemeType {
    SchemeContent,
    SchemeExpressive,
    SchemeFidelity,
    SchemeFruitSalad,
    SchemeMonochrome,
    SchemeNeutral,
    SchemeRainbow,
    SchemeTonalSpot,
    SchemeVibrant,
    SchemeSmart,
}

impl From<FfiSchemeType> for SchemeTypes {
    fn from(value: FfiSchemeType) -> Self {
        match value {
            FfiSchemeType::SchemeContent => SchemeTypes::SchemeContent,
            FfiSchemeType::SchemeExpressive => SchemeTypes::SchemeExpressive,
            FfiSchemeType::SchemeFidelity => SchemeTypes::SchemeFidelity,
            FfiSchemeType::SchemeFruitSalad => SchemeTypes::SchemeFruitSalad,
            FfiSchemeType::SchemeMonochrome => SchemeTypes::SchemeMonochrome,
            FfiSchemeType::SchemeNeutral => SchemeTypes::SchemeNeutral,
            FfiSchemeType::SchemeRainbow => SchemeTypes::SchemeRainbow,
            FfiSchemeType::SchemeTonalSpot => SchemeTypes::SchemeTonalSpot,
            FfiSchemeType::SchemeVibrant => SchemeTypes::SchemeVibrant,
            FfiSchemeType::SchemeSmart => SchemeTypes::SchemeSmart,
        }
    }
}

/// Mirrors [`SchemesEnum`], but with a lowercase `Deserialize` impl so JSON
/// requests can use `"light"`/`"dark"`/`"smart"` like the CLI's `--mode` flag.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
enum FfiSchemesEnum {
    Light,
    Dark,
    Smart,
}

impl From<FfiSchemesEnum> for SchemesEnum {
    fn from(value: FfiSchemesEnum) -> Self {
        match value {
            FfiSchemesEnum::Light => SchemesEnum::Light,
            FfiSchemesEnum::Dark => SchemesEnum::Dark,
            FfiSchemesEnum::Smart => SchemesEnum::Smart,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
enum SourceRequest {
    Image {
        path: String,
    },
    /// Raw image bytes, base64-encoded, so callers don't need to write the
    /// image to disk first.
    ImageBytes {
        data_base64: String,
    },
    Color {
        format: ColorFormatKind,
        value: String,
    },
    Json {
        path: String,
    },
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
enum ColorFormatKind {
    Hex,
    Rgb,
    Hsl,
}

/// Output color format for the `extractColors` JSON response.
/// Controls how color values are serialized in `colors`, `base16`, and
/// `source_color` fields of the response.
#[derive(Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "lowercase")]
enum FfiOutputColorFormat {
    #[default]
    Hex,
    Rgb,
    Rgba,
    Hsl,
    Hsla,
}

impl SourceRequest {
    fn into_source(self) -> Source {
        match self {
            SourceRequest::Image { path } => Source::Image { path },
            SourceRequest::ImageBytes { data_base64 } => Source::ImageBytes { data_base64 },
            SourceRequest::Json { path } => Source::Json { path },
            SourceRequest::Color { format, value } => Source::Color(match format {
                ColorFormatKind::Hex => ColorFormat::Hex { string: value },
                ColorFormatKind::Rgb => ColorFormat::Rgb { string: value },
                ColorFormatKind::Hsl => ColorFormat::Hsl { string: value },
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// §5.1 matugen_extract_colors
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
struct ExtractRequest {
    source: SourceRequest,
    #[serde(default)]
    scheme_type: Option<FfiSchemeType>,
    #[serde(default)]
    contrast: Option<f64>,
    #[serde(default)]
    lightness_dark: Option<f64>,
    #[serde(default)]
    lightness_light: Option<f64>,
    #[serde(default)]
    resize_filter: Option<FfiFilterType>,
    #[serde(default)]
    source_color_index: Option<i64>,
    #[serde(default)]
    prefer: Option<SelectionPreference>,
    #[serde(default)]
    fallback_color: Option<String>,
    #[serde(default)]
    base16_backend: Option<FfiBackend>,
    #[serde(default)]
    opacity: Option<f64>,
    /// Output format for color values in the response.
    /// Accepts: "hex" (default), "rgb", "rgba", "hsl", "hsla".
    #[serde(default)]
    color_format: Option<FfiOutputColorFormat>,
}

/// Raw (non-JSON) result of running the color-extraction pipeline. Kept
/// separate from the public JSON response so that `matugen_render_template`
/// can reuse the full-fidelity `Theme` (for palette keywords) when the caller
/// resends a `source` instead of forwarding a previous `matugen_extract_colors`
/// result.
struct Extraction {
    schemes: Option<Schemes>,
    base16: Option<Schemes>,
    theme: Option<Theme>,
    source_color: Option<Argb>,
    image: Option<String>,
}

fn build_cli_and_config(
    req: &ExtractRequest,
    source: Source,
    scheme_type: SchemeTypes,
) -> (Cli, ConfigFile) {
    let cli = Cli {
        source,
        r#type: scheme_type,
        config: None,
        prefix: None,
        contrast: req.contrast,
        verbose: None,
        quiet: Some(true),
        debug: None,
        include_image_in_json: None,
        mode: None,
        dry_run: None,
        show_colors: None,
        json: None,
        import_json: None,
        import_json_string: None,
        resize_filter: req.resize_filter.map(Into::into),
        continue_on_error: None,
        fallback_color: req.fallback_color.clone(),
        prefer: req.prefer.clone(),
        old_json_output: None,
        base16_backend: req.base16_backend.map(Into::into),
        #[cfg(feature = "filter-docs")]
        filter_docs_html: None,
        lightness_dark: req.lightness_dark,
        lightness_light: req.lightness_light,
        source_color_index: req.source_color_index,
        show_source_colors: None,
        opacity: req.opacity,
    };

    let config_file = ConfigFile {
        config: Config {
            version_check: None,
            caching: None,
            wallpaper: None,
            prefix: None,
            custom_colors: None,
            expr_prefix: None,
            expr_postfix: None,
            block_prefix: None,
            block_postfix: None,
            import_json_files: None,
            fallback_color: req.fallback_color.clone(),
            prefer: req.prefer.clone(),
            contrast: req.contrast,
            source_color_index: req.source_color_index,
        },
        templates: HashMap::new(),
    };

    (cli, config_file)
}

fn run_extraction(req: ExtractRequest) -> Result<Extraction, String> {
    let scheme_type = req
        .scheme_type
        .map(Into::into)
        .unwrap_or(SchemeTypes::SchemeTonalSpot);
    let source = req.source.clone().into_source();

    let image = match &source {
        Source::Image { path } => Some(
            std::fs::canonicalize(path)
                .map_err(|e| format!("Could not read image at {}: {}", path, e))
                .map(|p| crate::normalize_path_to_forward_slash(p.to_str().unwrap_or_default()))?,
        ),
        _ => None,
    };

    let (cli, config_file) = build_cli_and_config(&req, source, scheme_type);

    let (mut schemes, source_color, theme, mut base16) =
        generate_schemes_and_theme(&cli, &config_file, scheme_type)
            .map_err(|e| format!("{:?}", e))?;

    apply_opacity_to_schemes(&mut base16, req.opacity);
    apply_opacity_to_schemes(&mut schemes, req.opacity);

    // Surfaced separately so a caller-provided fallback_color error is
    // reported the same way the CLI would report it.
    let _ = parse_fallback_color(&config_file).map_err(|e| format!("{:?}", e))?;

    Ok(Extraction {
        schemes,
        base16,
        theme,
        source_color,
        image,
    })
}

fn format_argb_color(color: Argb, format: FfiOutputColorFormat) -> String {
    match format {
        FfiOutputColorFormat::Hex => format_hex_alpha(&rgb_from_argb(color)),
        FfiOutputColorFormat::Rgb => format_rgb(&rgb_from_argb(color)),
        FfiOutputColorFormat::Rgba => format_rgba(&rgb_from_argb(color)),
        FfiOutputColorFormat::Hsl => format_hsl(&hsl_from_argb(color)),
        FfiOutputColorFormat::Hsla => format_hsla(&hsl_from_argb(color)),
    }
}

fn schemes_to_colored_json(schemes: &Schemes, format: FfiOutputColorFormat) -> Value {
    let mut dark = Map::new();
    let mut light = Map::new();

    for (name, color) in &schemes.dark {
        dark.insert(
            name.clone(),
            Value::String(format_argb_color(*color, format)),
        );
    }
    for (name, color) in &schemes.light {
        light.insert(
            name.clone(),
            Value::String(format_argb_color(*color, format)),
        );
    }

    json!({ "dark": dark, "light": light })
}

fn extract_colors_impl(body: &str) -> Result<Value, String> {
    let req: ExtractRequest =
        serde_json::from_str(body).map_err(|e| format!("Invalid request JSON: {e}"))?;

    let color_format = req.color_format.unwrap_or_default();
    let extraction = run_extraction(req)?;

    let colors = extraction
        .schemes
        .as_ref()
        .map(|s| schemes_to_colored_json(s, color_format));
    let base16 = extraction
        .base16
        .as_ref()
        .map(|s| schemes_to_colored_json(s, color_format));
    let source_color = extraction
        .source_color
        .map(|c| format_argb_color(c, color_format));

    Ok(json!({
        "colors": colors,
        "base16": base16,
        "source_color": source_color,
        "image": extraction.image,
    }))
}

/// Extracts a Material You + base16 palette from an image or an explicit
/// color. See the module docs for the request/response JSON shape and memory
/// ownership rules.
///
/// # Safety
/// `request_json` must be null or a valid pointer to a NUL-terminated UTF-8
/// C string, valid for the duration of the call.
#[no_mangle]
pub extern "C" fn matugen_extract_colors(request_json: *const c_char) -> *mut c_char {
    run_ffi(request_json, extract_colors_impl)
}

// ---------------------------------------------------------------------------
// §5.2 matugen_render_template
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug, Default)]
struct TemplateRequest {
    #[serde(default)]
    input_path: Option<PathBuf>,
    #[serde(default)]
    input_string: Option<String>,
    #[serde(default)]
    mode: Option<FfiSchemesEnum>,
    #[serde(default)]
    block_prefix: Option<String>,
    #[serde(default)]
    block_postfix: Option<String>,
    #[serde(default)]
    expr_prefix: Option<String>,
    #[serde(default)]
    expr_postfix: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
struct RenderRequest {
    colors: Value,
    template: TemplateRequest,
    #[serde(default)]
    custom_keywords: Option<Map<String, Value>>,
}

/// Parse a color value (hex, rgb, rgba, hsl, hsla, etc.) into an `Argb`.
/// This replaces the old `schemes_from_hex_map` which only handled `#hex`.
fn parse_color_value(value_str: &str) -> Result<Argb, String> {
    let rgb = parse_css_color(value_str)
        .map_err(|e| format!("Could not parse color '{}': {}", value_str, e))?;
    Ok(argb_from_rgb(&rgb))
}

fn schemes_from_color_map(map: &Map<String, Value>) -> Result<IndexMap<String, Argb>, String> {
    let mut out = IndexMap::new();
    for (name, value) in map {
        let color_str = value
            .as_str()
            .ok_or_else(|| format!("Color '{}' must be a string", name))?;
        let argb = parse_color_value(color_str)
            .map_err(|e| format!("Invalid color for '{}': {}", name, e))?;
        out.insert(name.clone(), argb);
    }
    Ok(out)
}

fn schemes_from_response(colors_obj: &Map<String, Value>) -> Result<Option<Schemes>, String> {
    let dark = colors_obj.get("dark").and_then(Value::as_object);
    let light = colors_obj.get("light").and_then(Value::as_object);

    match (dark, light) {
        (Some(dark), Some(light)) => Ok(Some(Schemes {
            dark: schemes_from_color_map(dark)?,
            light: schemes_from_color_map(light)?,
        })),
        _ => Ok(None),
    }
}

/// Builds the `{ "colors": ..., "base16": ..., "image": ..., "mode": ...,
/// "is_dark_mode": ... }` render context, either by:
/// - regenerating it from scratch when `colors` carries a `"source"` key
///   (same shape as a `matugen_extract_colors` request, giving full access to
///   palettes), or
/// - reconstructing it from a previous `matugen_extract_colors` response
///   (hex strings only, no palettes).
fn resolve_render_context(
    colors_value: &Value,
    default_scheme: SchemesEnum,
) -> Result<Value, String> {
    let is_dark_mode = match default_scheme {
        SchemesEnum::Dark => true,
        SchemesEnum::Light => false,
        SchemesEnum::Smart => {
            return Err(
                "template.mode must resolve to \"light\" or \"dark\", not \"smart\"".to_string(),
            )
        }
    };

    if colors_value.get("source").is_some() {
        let req: ExtractRequest = serde_json::from_value(colors_value.clone())
            .map_err(|e| format!("Invalid embedded source request in 'colors': {e}"))?;
        let extraction = run_extraction(req)?;

        let base = json!({
            "image": extraction.image,
            "mode": default_scheme.to_string(),
            "is_dark_mode": is_dark_mode,
        });

        merge_json_source(
            base,
            &extraction.schemes,
            &extraction.base16,
            &extraction.theme,
            default_scheme,
        )
        .map_err(|e| format!("{:?}", e))
    } else {
        let colors_obj = colors_value
            .as_object()
            .ok_or("'colors' must be a JSON object")?;

        let schemes = colors_obj
            .get("colors")
            .and_then(Value::as_object)
            .map(schemes_from_response)
            .transpose()?
            .flatten();

        let base16 = colors_obj
            .get("base16")
            .and_then(Value::as_object)
            .map(schemes_from_response)
            .transpose()?
            .flatten();

        let image = colors_obj.get("image").cloned().unwrap_or(Value::Null);

        let base = json!({
            "image": image,
            "mode": default_scheme.to_string(),
            "is_dark_mode": is_dark_mode,
        });

        merge_json_source(base, &schemes, &base16, &None, default_scheme)
            .map_err(|e| format!("{:?}", e))
    }
}

fn register_default_filters(engine: &mut Engine) {
    register_filters!((engine) {
        "Colors" => {
            "set_red" => crate::filters::set_red,
            "set_blue" => crate::filters::set_blue,
            "set_green" => crate::filters::set_green,
            "set_alpha" => crate::filters::set_alpha,
            "set_hue" => crate::filters::set_hue,
            "set_saturation" => crate::filters::set_saturation,
            "set_lightness" => crate::filters::set_lightness,
            "lighten" => crate::filters::lighten,
            "to_color" => crate::filters::to_color,
            "invert" => crate::filters::invert,
            "grayscale" => crate::filters::grayscale,
            "auto_lightness" => crate::filters::auto_lighten,
            "saturate" => crate::filters::saturate,
            "blend" => crate::filters::blend,
            "harmonize" => crate::filters::harmonize,
            "format" => crate::filters::format,
        },

        "String" => {
            "snake_case" => crate::filters::snake_case,
            "lower_case" => crate::filters::lower_case,
            "camel_case" => crate::filters::camel_case,
            "pascal_case" => crate::filters::pascal_case,
            "kebab_case" => crate::filters::kebab_case,
            "replace" => crate::filters::replace,
        },
    });
}

/// Helper shared by `render_template_impl` and `render_from_image_impl`.
/// Reads the template source, adds it to the engine, and renders it.
fn run_render(engine: &mut Engine, template: &TemplateRequest) -> Result<Value, String> {
    let data = match (&template.input_string, &template.input_path) {
        (Some(source), _) => source.clone(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| format!("Could not read the {} template: {}", path.display(), e))?,
        (None, None) => {
            return Err("template.input_path or template.input_string is required".to_string())
        }
    };

    let name = "ffi_template".to_string();
    engine.try_add_template(name.clone(), data)?;

    match engine.render(&name) {
        Ok(rendered) => Ok(json!({ "rendered": rendered })),
        Err(errors) => Err(errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ")),
    }
}

fn render_template_impl(body: &str) -> Result<Value, String> {
    let req: RenderRequest =
        serde_json::from_str(body).map_err(|e| format!("Invalid request JSON: {e}"))?;

    let mode = req
        .template
        .mode
        .map(Into::into)
        .unwrap_or(SchemesEnum::Dark);
    let context = resolve_render_context(&req.colors, mode)?;

    let mut engine = Engine::new();
    engine.set_syntax(get_syntax(
        req.template.block_prefix.as_ref(),
        req.template.block_postfix.as_ref(),
        req.template.expr_prefix.as_ref(),
        req.template.expr_postfix.as_ref(),
    ));
    register_default_filters(&mut engine);

    engine.add_context(context);

    if let Some(custom_keywords) = req.custom_keywords {
        let mut merged = Value::Object(Map::new());
        merge_json(&mut merged, Value::Object(custom_keywords));
        engine.add_context(merged);
    }

    run_render(&mut engine, &req.template)
}

/// Renders a single template to a string. Never touches disk. See the module
/// docs for the request/response JSON shape and memory ownership rules.
///
/// # Safety
/// `request_json` must be null or a valid pointer to a NUL-terminated UTF-8
/// C string, valid for the duration of the call.
#[no_mangle]
pub extern "C" fn matugen_render_template(request_json: *const c_char) -> *mut c_char {
    run_ffi(request_json, render_template_impl)
}

// ---------------------------------------------------------------------------
// §5.3 matugen_write_output
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
struct WriteRequest {
    content: String,
    output_path: PathBuf,
    #[serde(default = "default_true")]
    create_missing_dirs: bool,
    #[serde(default)]
    pre_hook: Option<String>,
    #[serde(default)]
    post_hook: Option<String>,
    #[serde(default)]
    hook_context: Option<Map<String, Value>>,
}

fn default_true() -> bool {
    true
}

fn hook_output_to_json(output: &HookOutput) -> Value {
    json!({
        "stdout": output.stdout,
        "stderr": output.stderr,
        "exit_code": output.exit_code,
    })
}

fn write_output_impl(body: &str) -> Result<Value, String> {
    let req: WriteRequest =
        serde_json::from_str(body).map_err(|e| format!("Invalid request JSON: {e}"))?;

    let mut engine = Engine::new();
    if let Some(hook_context) = &req.hook_context {
        engine.add_context(Value::Object(hook_context.clone()));
    }

    let mut pre_hook_output = None;
    if let Some(hook) = &req.pre_hook {
        pre_hook_output =
            Some(format_hook(&mut engine, hook, &None, &None).map_err(|e| format!("{:?}", e))?);
    }

    let written_path = write_to_disk(
        &req.content,
        &req.output_path,
        &None,
        req.create_missing_dirs,
    )
    .map_err(|e| format!("{:?}", e))?;

    let written_path = match written_path {
        Some(path) => path,
        None => {
            return Err(format!(
                "The {} file is read-only, not writing to it.",
                req.output_path.display()
            ))
        }
    };

    let mut post_hook_output = None;
    if let Some(hook) = &req.post_hook {
        post_hook_output =
            Some(format_hook(&mut engine, hook, &None, &None).map_err(|e| format!("{:?}", e))?);
    }

    Ok(json!({
        "written_path": written_path.to_string_lossy(),
        "pre_hook_output": pre_hook_output.as_ref().map(hook_output_to_json),
        "post_hook_output": post_hook_output.as_ref().map(hook_output_to_json),
    }))
}

/// Writes rendered content to disk and optionally runs `pre_hook`/`post_hook`.
/// See the module docs for the request/response JSON shape and memory
/// ownership rules.
///
/// # Safety
/// `request_json` must be null or a valid pointer to a NUL-terminated UTF-8
/// C string, valid for the duration of the call.
#[no_mangle]
pub extern "C" fn matugen_write_output(request_json: *const c_char) -> *mut c_char {
    run_ffi(request_json, write_output_impl)
}

// ---------------------------------------------------------------------------
// §5.5 matugen_render_from_image
// ---------------------------------------------------------------------------

/// Combined extraction + rendering in a single call.
/// Accepts an image source and a template, internally runs the full
/// extraction pipeline (producing raw `Argb` schemes), builds the template
/// context with full palette/keyword access, and renders the template.
/// The color format used in templates (e.g. `.red`, `.green`, `.blue`,
/// `.rgb`, `.hsl`) is always correctly inferred by the template engine
/// regardless of what `color_format` the caller might use for
/// `matugen_extract_colors`.
#[derive(Deserialize, Clone, Debug)]
struct RenderFromImageRequest {
    /// Same shape as `ExtractRequest.source`.
    source: SourceRequest,
    #[serde(default)]
    scheme_type: Option<FfiSchemeType>,
    #[serde(default)]
    contrast: Option<f64>,
    #[serde(default)]
    lightness_dark: Option<f64>,
    #[serde(default)]
    lightness_light: Option<f64>,
    #[serde(default)]
    resize_filter: Option<FfiFilterType>,
    #[serde(default)]
    source_color_index: Option<i64>,
    #[serde(default)]
    prefer: Option<SelectionPreference>,
    #[serde(default)]
    fallback_color: Option<String>,
    #[serde(default)]
    base16_backend: Option<FfiBackend>,
    #[serde(default)]
    opacity: Option<f64>,
    /// Output color format for the `colors` key in the response.
    #[serde(default)]
    color_format: Option<FfiOutputColorFormat>,
    /// The template to render.
    template: TemplateRequest,
    /// Optional extra template keywords.
    #[serde(default)]
    custom_keywords: Option<Map<String, Value>>,
}

fn render_from_image_impl(body: &str) -> Result<Value, String> {
    let req: RenderFromImageRequest =
        serde_json::from_str(body).map_err(|e| format!("Invalid request JSON: {e}"))?;

    let mode = req
        .template
        .mode
        .map(Into::into)
        .unwrap_or(SchemesEnum::Dark);
    let color_format = req.color_format.unwrap_or_default();

    // --- 1. Run extraction to get raw Argb schemes ---
    let extract_req = ExtractRequest {
        source: req.source,
        scheme_type: req.scheme_type,
        contrast: req.contrast,
        lightness_dark: req.lightness_dark,
        lightness_light: req.lightness_light,
        resize_filter: req.resize_filter,
        source_color_index: req.source_color_index,
        prefer: req.prefer,
        fallback_color: req.fallback_color,
        base16_backend: req.base16_backend,
        opacity: req.opacity,
        color_format: None, // always use raw Argb internally
    };
    let extraction = run_extraction(extract_req)?;

    // --- 2. Build template context via merge_json_source ---
    let is_dark_mode = match mode {
        SchemesEnum::Dark => true,
        SchemesEnum::Light => false,
        SchemesEnum::Smart => {
            return Err(
                "template.mode must resolve to \"light\" or \"dark\", not \"smart\"".to_string(),
            )
        }
    };

    let base = json!({
        "image": extraction.image,
        "mode": mode.to_string(),
        "is_dark_mode": is_dark_mode,
    });

    let context = merge_json_source(
        base,
        &extraction.schemes,
        &extraction.base16,
        &extraction.theme,
        mode,
    )
    .map_err(|e| format!("{:?}", e))?;

    // --- 3. Set up engine and render ---
    let mut engine = Engine::new();
    engine.set_syntax(get_syntax(
        req.template.block_prefix.as_ref(),
        req.template.block_postfix.as_ref(),
        req.template.expr_prefix.as_ref(),
        req.template.expr_postfix.as_ref(),
    ));
    register_default_filters(&mut engine);
    engine.add_context(context);

    if let Some(custom_keywords) = req.custom_keywords {
        let mut merged = Value::Object(Map::new());
        merge_json(&mut merged, Value::Object(custom_keywords));
        engine.add_context(merged);
    }

    let rendered = run_render(&mut engine, &req.template)?;

    // --- 4. Build the response with colors + rendered template ---
    let colors_json = extraction
        .schemes
        .as_ref()
        .map(|s| schemes_to_colored_json(s, color_format));
    let base16_json = extraction
        .base16
        .as_ref()
        .map(|s| schemes_to_colored_json(s, color_format));
    let source_color_json = extraction
        .source_color
        .map(|c| format_argb_color(c, color_format));

    Ok(json!({
        "rendered": rendered["rendered"],
        "colors": colors_json,
        "base16": base16_json,
        "source_color": source_color_json,
    }))
}

/// Extracts colors from an image and renders a template in a single call.
/// The template engine automatically infers the color format from how
/// the template accesses colors (`.red`, `.green`, `.blue`, `.rgb`, `.hsl`,
/// etc.), so callers don't need to worry about matching extraction
/// format to template needs.
///
/// Returns `{ "ok": true, "rendered": "...", "colors": ..., "base16": ..., "source_color": ... }`.
///
/// # Safety
/// `request_json` must be null or a valid pointer to a NUL-terminated UTF-8
/// C string, valid for the duration of the call.
#[no_mangle]
pub extern "C" fn matugen_render_from_image(request_json: *const c_char) -> *mut c_char {
    run_ffi(request_json, render_from_image_impl)
}

// ---------------------------------------------------------------------------
// matugen_get_source_colors
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
struct SourceColorsRequest {
    source: SourceRequest,
    #[serde(default)]
    resize_filter: Option<FfiFilterType>,
    #[serde(default)]
    fallback_color: Option<String>,
}

fn source_colors_impl(body: &str) -> Result<Value, String> {
    let req: SourceColorsRequest =
        serde_json::from_str(body).map_err(|e| format!("Invalid request JSON: {e}"))?;

    let filter = get_filter(&req.resize_filter.map(Into::into));
    let fallback_color = req
        .fallback_color
        .as_deref()
        .map(Argb::from_str)
        .transpose()
        .map_err(|_| "Invalid fallback_color: expected a hex color string".to_string())?;

    let source = req.source.into_source();
    let ranked = match &source {
        Source::Image { path } => get_scored_colors_from_image(path, filter, fallback_color)
            .map_err(|e| format!("{:?}", e))?,
        Source::ImageBytes { data_base64 } => {
            let bytes = decode_base64_image(data_base64).map_err(|e| format!("{:?}", e))?;
            get_scored_colors_from_image_bytes(&bytes, filter, fallback_color)
                .map_err(|e| format!("{:?}", e))?
        }
        _ => return Err("source must be an image or image-bytes source".to_string()),
    };

    let colors: Vec<String> = ranked
        .iter()
        .map(|c| format_hex_alpha(&rgb_from_argb(*c)))
        .collect();

    Ok(json!({ "source_colors": colors }))
}

/// Returns the ranked list of candidate source colors extracted from an image
/// (or image bytes), without picking one. This is the same ranking the CLI's
/// `--show-source-colors` flag prints. See the module docs for the
/// request/response JSON shape and memory ownership rules.
///
/// # Safety
/// `request_json` must be null or a valid pointer to a NUL-terminated UTF-8
/// C string, valid for the duration of the call.
#[no_mangle]
pub extern "C" fn matugen_get_source_colors(request_json: *const c_char) -> *mut c_char {
    run_ffi(request_json, source_colors_impl)
}
