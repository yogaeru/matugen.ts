#![allow(clippy::too_many_arguments)]

extern crate pretty_env_logger;
#[macro_use]
extern crate paris_log;

use std::path::PathBuf;

use material_colors::theme::ThemeBuilder;
use serde_json::Value;

pub mod helpers;
pub mod smart_scheme;
pub mod template;
pub mod util;
pub mod wallpaper;

pub mod cache;
pub mod color;
pub mod exec;
pub mod filters;
pub mod parser;
pub mod scheme;
pub mod template_util;

#[cfg(feature = "ffi")]
pub mod ffi;

use crate::{
    cache::ImageCache,
    color::color::{get_filter, get_scored_colors_from_image, Source},
    helpers::{
        apply_opacity_to_schemes, generate_schemes_and_theme, get_syntax, json_from_file,
        merge_json, merge_json_source, parse_fallback_color,
    },
    scheme::{SchemeTypes, SchemesEnum},
    template::get_absolute_path,
};
use helpers::set_wallpaper;
use smart_scheme::SmartOpts;
use template::TemplateFile;

use crate::{
    template::Template,
    util::{arguments::Cli, color::show_color, config::ConfigFile},
};

use color_eyre::{eyre::Context, Report, Section};

use crate::{parser::Engine, scheme::Schemes};

use material_colors::{color::Argb, theme::Theme};

pub struct State {
    pub args: Cli,
    pub config_file: ConfigFile,
    pub config_path: Option<PathBuf>,
    pub source_color: Option<Argb>,
    pub theme: Option<Theme>,
    pub schemes: Option<Schemes>,
    pub default_scheme: SchemesEnum,
    pub resolved_type: SchemeTypes,
    pub smart_variant: SchemeTypes,
    pub image_hash: ImageCache,
    pub loaded_cache: bool,
    pub base16: Option<Schemes>,
}

impl State {
    pub fn new(args: Cli) -> Result<Self, Report> {
        #[cfg(feature = "jxl-image")]
        jxl_oxide::integration::register_image_decoding_hook();

        let (mut config_file, config_path) =
            ConfigFile::read(&args).wrap_err("Failed to read config file.")?;

        config_file.parse_cli_overrides(&args);

        let effective_mode = args.mode.unwrap_or(SchemesEnum::Dark);
        let effective_type = args.r#type;

        let mut loaded_cache = false;

        let caching_enabled = config_file.config.caching.unwrap_or(false) && args.source.is_image();

        let any_template_smart = config_file
            .templates
            .values()
            .any(|t| matches!(t.r#type, Some(SchemeTypes::SchemeSmart)));

        let smart_requested = matches!(effective_mode, SchemesEnum::Smart)
            || matches!(effective_type, SchemeTypes::SchemeSmart)
            || any_template_smart;

        let smart_opts: Option<SmartOpts> = if smart_requested {
            if !args.source.is_image() {
                warn!(
                    "Smart scheme needs an image source, got <yellow>{:?}</>. Falling back to defaults.",
                    args.source
                );
                None
            } else {
                let image_path = match &args.source {
                    Source::Image { path } => path,
                    _ => unreachable!(),
                };
                match smart_scheme::get_smart_opts(std::path::Path::new(image_path)) {
                    Ok(opts) => Some(opts),
                    Err(e) => {
                        warn!(
                            "Smart scheme detection failed: <yellow>{}</>. Falling back to defaults.",
                            e
                        );
                        None
                    }
                }
            }
        } else {
            None
        };

        let smart_variant = smart_opts
            .as_ref()
            .map(|o| o.variant)
            .unwrap_or(SchemeTypes::SchemeTonalSpot);

        let default_scheme = match effective_mode {
            SchemesEnum::Smart => smart_opts
                .as_ref()
                .map(|o| o.mode)
                .unwrap_or(SchemesEnum::Dark),
            other => other,
        };

        let resolved_type = match effective_type {
            SchemeTypes::SchemeSmart => smart_opts
                .as_ref()
                .map(|o| o.variant)
                .unwrap_or(SchemeTypes::SchemeTonalSpot),
            other => other,
        };

        let image_cache = ImageCache::new(
            &args.source,
            resolved_type,
            args.contrast.or(config_file.config.contrast),
            args.lightness_dark,
            args.lightness_light,
        );

        info!(
            "Scheme: mode=<b><cyan>{}</>, variant=<b><cyan>{:?}</>",
            default_scheme, resolved_type
        );

        if let Source::Image { path } = &args.source {
            if args.show_source_colors.is_some_and(|x| x) {
                let filter = get_filter(&args.resize_filter);
                let fallback_color = parse_fallback_color(&config_file)?;
                let ranked = get_scored_colors_from_image(&path, filter, fallback_color)?;

                for color in ranked {
                    println!("{}", color.to_hex_with_pound());
                }

                return Ok(Self {
                    args,
                    config_file,
                    config_path,
                    source_color: None,
                    theme: None,
                    schemes: None,
                    default_scheme,
                    resolved_type,
                    smart_variant,
                    image_hash: image_cache,
                    loaded_cache,
                    base16: None,
                });
            }
        }

        let (mut schemes, source_color, theme, mut base16) = if caching_enabled {
            match image_cache.load() {
                Ok((schemes, base16)) => {
                    // Source color will be the same in both light and dark mode
                    let source_color = *schemes.dark.clone().get("source_color").unwrap();

                    let theme = ThemeBuilder::with_source(source_color).build();

                    loaded_cache = true;

                    (Some(schemes), Some(source_color), Some(theme), Some(base16))
                }
                Err(e) => {
                    if !image_cache.exists() {
                        warn!(
                            "<d>The cache in <yellow><b>{}</><d> doesn't exist.</>",
                            image_cache.get_path().display()
                        );
                        generate_schemes_and_theme(&args, &config_file, resolved_type)?
                    } else {
                        return Err(e.wrap_err("Couldn't load the cache file").suggestion("You may need to regenerate your cache if coming from v3.1.0 and lower."));
                    }
                }
            }
        } else {
            generate_schemes_and_theme(&args, &config_file, resolved_type)?
        };

        apply_opacity_to_schemes(&mut base16, args.opacity);
        apply_opacity_to_schemes(&mut schemes, args.opacity);

        Ok(Self {
            args,
            config_file,
            config_path,
            source_color,
            theme,
            schemes,
            default_scheme,
            resolved_type,
            smart_variant,
            image_hash: image_cache,
            loaded_cache,
            base16,
        })
    }

    fn init_engine(&self) -> Result<(Engine, Value), Report> {
        let json = self
            .get_render_data()
            .wrap_err("Could not get render data")?;

        let mut engine = Engine::new();

        engine.set_syntax(get_syntax(
            self.config_file.config.block_prefix.as_ref(),
            self.config_file.config.block_postfix.as_ref(),
            self.config_file.config.expr_prefix.as_ref(),
            self.config_file.config.expr_postfix.as_ref(),
        ));

        self.add_engine_filters(&mut engine);

        let mut json = match &self.args.source {
            Source::Json { path } => json_from_file(&PathBuf::from(path)).unwrap(),
            _ => merge_json_source(
                json,
                &self.schemes,
                &self.base16,
                &self.theme,
                self.default_scheme,
            )?,
        };

        if let Some(paths) = &self.args.import_json {
            for path in paths {
                let json2 = json_from_file(&PathBuf::from(path)).unwrap();
                merge_json(&mut json, json2);
            }
        }

        if let Some(strings) = &self.args.import_json_string {
            for string in strings {
                let json2 =
                    serde_json::from_str(&string).expect("Failed to parse JSON from string.");
                merge_json(&mut json, json2);
            }
        }

        if let (Some(paths), Some(config_path)) = (
            &self.config_file.config.import_json_files,
            &self.config_path,
        ) {
            for path in paths {
                let absolute = get_absolute_path(config_path, path).unwrap();

                let json2 = json_from_file(&absolute).unwrap();

                merge_json(&mut json, json2);
            }
        }

        if self.config_file.config.caching.unwrap_or(false)
            && self.args.source.is_image()
            && !self.loaded_cache
        {
            self.save_cache(&mut json.clone())
                .expect("Failed saving cache");
        }

        engine.add_context(json.clone());

        Ok((engine, json))
    }

    fn save_cache(&self, _json: &Value) -> Result<(), Report> {
        let json_modified = serde_json::json!({
            "colors": {
                "dark": cache::convert_argb_scheme(&self.schemes.as_ref().unwrap().dark),
                "light": cache::convert_argb_scheme(&self.schemes.as_ref().unwrap().light),
            },
            "base16": {
                "dark": cache::convert_argb_scheme(&self.base16.as_ref().unwrap().dark),
                "light": cache::convert_argb_scheme(&self.base16.as_ref().unwrap().light),
            },
        });

        self.image_hash.save(&json_modified)
    }

    pub fn get_render_data(&self) -> Result<serde_json::Value, Report> {
        let image = match &self.args.source {
            Source::Image { path } => Some(normalize_path_to_forward_slash(
                std::fs::canonicalize(path)?
                    .to_str()
                    .ok_or_else(|| Report::msg("Could not canonicalize the image path"))?,
            )),
            Source::ImageBytes { .. } => None,
            #[cfg(feature = "web-image")]
            Source::WebImage { .. } => None,
            Source::Color { .. } => None,
            Source::Json { path: _ } => None,
        };

        let is_dark_mode = match self.default_scheme {
            SchemesEnum::Dark => true,
            SchemesEnum::Light => false,
            SchemesEnum::Smart => unreachable!("default_scheme is resolved before storage"),
        };

        Ok(serde_json::json!({
            "image": image, "mode": format!("{}", self.default_scheme), "is_dark_mode": is_dark_mode,
        }))
    }

    fn add_engine_filters(&self, engine: &mut Engine) {
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

    fn init_in_term(&self) -> Result<(), Report> {
        #[cfg(feature = "update-informer")]
        if self.config_file.config.version_check == Some(true) {
            use crate::helpers::check_version;
            check_version();
        }

        Ok(())
    }

    pub fn run_in_term(&self) -> Result<(), Report> {
        self.init_in_term()?;

        if self.args.show_colors == Some(true) && !self.args.source.is_json() {
            show_color(
                self.schemes.as_ref(),
                self.source_color.as_ref(),
                self.base16.as_ref(),
            );
        }

        let (mut engine, mut json_value) = self
            .init_engine()
            .wrap_err("Something went wrong while initializing the engine")?;
        let mut template = TemplateFile::new(self, &mut engine);

        #[cfg(feature = "filter-docs")]
        {
            if self.args.filter_docs_html == Some(true) {
                {
                    use crate::parser::helpers::filters_to_html;
                    println!("{}", filters_to_html());
                    return Ok(());
                }
            }
        }

        #[cfg(feature = "dump-json")]
        if let Some(ref format) = self.args.json {
            use crate::util::color::dump_json;
            if !self.args.include_image_in_json.unwrap_or(true) {
                if let Some(obj) = json_value.as_object_mut() {
                    obj.remove("image");
                };
            };
            dump_json(&mut json_value, format, self.args.old_json_output);
        }

        if self.args.dry_run == Some(true) {
            return Ok(());
        }

        template.generate()?;

        if let Some(_wallpaper_cfg) = &self.config_file.config.wallpaper {
            if _wallpaper_cfg.set.unwrap_or(true) {
                set_wallpaper(&self.args.source, _wallpaper_cfg, &mut engine)?;
            }
        }

        Ok(())
    }
}

pub fn normalize_path_to_forward_slash(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut prev_was_backslash = false;

    for c in path.chars() {
        if c == '\\' {
            if !prev_was_backslash {
                result.push(c);
                prev_was_backslash = true;
            }
        } else {
            result.push(c);
            prev_was_backslash = false;
        }
    }
    result.replace('\\', "/")
}
