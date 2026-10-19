mod find_string;
mod message;
mod named_combobox;
mod request_counter;
mod toggle_switch;

//#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Deref, RangeInclusive};
use std::time::Instant;
use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
    path::PathBuf,
};

use eframe::egui::{Button, CollapsingHeader, Label, RichText};
use eframe::epaint::Vec2;
use eframe::{
    egui::{FontSelection, Layout, TextFormat, Ui},
    emath::{Align, Align2},
    epaint::{Color32, Stroke, text::LayoutJob},
};
use itertools::Itertools as _;
use mint_lib::error::ResultExt as _;
use mint_lib::mod_info::{ModioTags, RequiredStatus};
use strum::{EnumIter, IntoEnumIterator};
use tokio::{
    sync::mpsc::{self, Receiver, Sender},
    task::JoinHandle,
};
use tracing::{debug, trace};

use crate::Dirs;
use crate::gui::find_string::searchable_text;
use crate::mod_lints::{LintId, LintReport, SplitAssetPair};
use crate::providers::ProviderError;
use crate::state::SortingConfig;
use crate::{
    MintError,
    integrate::uninstall,
    is_drg_pak,
    providers::{
        ApprovalStatus, FetchProgress, ModInfo, ModSpecification, ModStore, ProviderFactory,
    },
    state::{
        ModConfig, ModData_v0_2_0 as ModData, ModNotes_v0_0_0 as ModNotes, ModOrGroup, ModProfile,
        State,
    },
};
use message::MessageHandle;
use request_counter::{RequestCounter, RequestID};

use self::message::ModDetails;
use self::toggle_switch::toggle_switch;

pub fn gui(dirs: Dirs, args: Option<Vec<String>>) -> Result<(), MintError> {
    let options = eframe::NativeOptions {
        centered: true,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: image::load_from_memory(include_bytes!("../../assets/icon-gui.ico"))
                    .unwrap()
                    .to_rgba8()
                    .to_vec(),
                width: 32,
                height: 32,
            })),
        ..Default::default()
    };
    eframe::run_native(
        &format!("mintnix {}", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc, dirs, args)?))),
    )
    .with_generic(|e| format!("{e}"))?;
    Ok(())
}

pub mod colors {
    use eframe::epaint::Color32;

    pub const DARK_RED: Color32 = Color32::DARK_RED;
    pub const DARKER_RED: Color32 = Color32::from_rgb(110, 0, 0);

    pub const DARK_GREEN: Color32 = Color32::DARK_GREEN;
    pub const DARKER_GREEN: Color32 = Color32::from_rgb(0, 80, 0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiTheme {
    Light,
    Dark,
}

impl GuiTheme {
    fn from_egui_theme(theme: egui::ThemePreference) -> Option<Self> {
        match theme {
            egui::ThemePreference::Dark => Some(GuiTheme::Dark),
            egui::ThemePreference::Light => Some(GuiTheme::Light),
            egui::ThemePreference::System => None,
        }
    }
    fn into_egui_theme(theme: Option<Self>) -> egui::ThemePreference {
        match theme {
            Some(GuiTheme::Dark) => egui::ThemePreference::Dark,
            Some(GuiTheme::Light) => egui::ThemePreference::Light,
            None => egui::ThemePreference::System,
        }
    }
}

#[derive(PartialEq, Debug, EnumIter, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum SortBy {
    Enabled,
    Name,
    Priority,
    Provider,
    RequiredStatus,
    ApprovalCategory,
}

impl SortBy {
    fn as_str(&self) -> &'static str {
        match self {
            SortBy::Enabled => "Enabled",
            SortBy::Name => "Name",
            SortBy::Priority => "Priority",
            SortBy::Provider => "Provider",
            SortBy::RequiredStatus => "Is Required",
            SortBy::ApprovalCategory => "Approval",
        }
    }
}

const FOLDER_LOGO_PNG: &[u8] = include_bytes!("../../assets/folder.png");
const HTTP_LOGO_PNG: &[u8] = include_bytes!("../../assets/globe.png");
const MODIO_LOGO_PNG: &[u8] = include_bytes!("../../assets/modio-cog-blue.png");
const HEADER_PNG: &[u8] = include_bytes!("../../assets/header.png");

pub struct App {
    args: Option<Vec<String>>,
    tx: Sender<message::Message>,
    rx: Receiver<message::Message>,
    state: State,
    resolve_mod: String,
    resolve_mod_rid: Option<MessageHandle<()>>,
    integrate_rid: Option<MessageHandle<HashMap<ModSpecification, SpecFetchProgress>>>,
    update_rid: Option<MessageHandle<()>>,
    has_run_init: bool,
    request_counter: RequestCounter,
    window_provider_parameters: Option<WindowProviderParameters>,
    search_string: String,
    scroll_to_match: bool,
    focus_search: bool,
    settings_window: Option<WindowSettings>,
    about_window: Option<WindowAbout>,
    notes_window: HashMap<String, WindowNotes>,
    folder_texture_handle: Option<egui::TextureHandle>,
    http_texture_handle: Option<egui::TextureHandle>,
    header_texture_handle: Option<egui::TextureHandle>,
    detailed_mod_info_windows: HashMap<u32, WindowDetailedModInfo>,
    mod_details: HashMap<u32, ModDetails>,
    fetch_mod_details_rid: HashMap<u32, MessageHandle<()>>,
    mod_details_thumbnail_texture_handle: HashMap<u32, egui::TextureHandle>,
    pending_delete: Option<usize>,
    modio_texture_handle: Option<egui::TextureHandle>,
    last_action: Option<LastAction>,
    open_profiles: HashSet<String>,
    lint_rid: Option<MessageHandle<()>>,
    lint_report_window: Option<WindowLintReport>,
    lint_report: Option<LintReport>,
    lints_toggle_window: Option<WindowLintsToggle>,
    lint_options: LintOptions,
    problematic_mod_id: Option<u32>,
}

#[derive(Default)]
struct LintOptions {
    archive_with_multiple_paks: bool,
    archive_with_only_non_pak_files: bool,
    asset_register_bin: bool,
    conflicting: bool,
    empty_archive: bool,
    outdated_pak_version: bool,
    shader_files: bool,
    non_asset_files: bool,
    split_asset_pairs: bool,
    unmodified_game_assets: bool,
}

struct LastAction {
    timestamp: Instant,
    status: LastActionStatus,
}
impl LastAction {
    fn success(msg: String) -> Self {
        Self {
            timestamp: Instant::now(),
            status: LastActionStatus::Success(msg),
        }
    }
    fn failure(msg: String) -> Self {
        Self {
            timestamp: Instant::now(),
            status: LastActionStatus::Failure(msg),
        }
    }
    fn timeago(&self) -> String {
        let duration = Instant::now().duration_since(self.timestamp);
        let seconds = duration.as_secs();
        if seconds < 60 {
            format!("{seconds}s ago")
        } else if seconds < 3600 {
            format!("{}m ago", seconds / 60)
        } else {
            ">1h ago".into()
        }
    }
}

enum LastActionStatus {
    Success(String),
    Failure(String),
}

impl App {
    fn new(
        cc: &eframe::CreationContext,
        dirs: Dirs,
        args: Option<Vec<String>>,
    ) -> Result<Self, MintError> {
        let (tx, rx) = mpsc::channel(10);
        let state = State::init(dirs)?;
        Self::set_custom_fonts(&cc.egui_ctx);

        Ok(Self {
            args,
            tx,
            rx,
            request_counter: Default::default(),
            state,
            resolve_mod: Default::default(),
            resolve_mod_rid: None,
            integrate_rid: None,
            update_rid: None,
            has_run_init: false,
            window_provider_parameters: None,
            search_string: Default::default(),
            scroll_to_match: false,
            focus_search: false,
            settings_window: None,
            about_window: None,
            notes_window: HashMap::default(),
            folder_texture_handle: None,
            http_texture_handle: None,
            header_texture_handle: None,
            detailed_mod_info_windows: HashMap::default(),
            mod_details: HashMap::default(),
            fetch_mod_details_rid: HashMap::default(),
            mod_details_thumbnail_texture_handle: HashMap::default(),
            pending_delete: None,
            modio_texture_handle: None,
            last_action: None,
            open_profiles: Default::default(),
            lint_rid: None,
            lint_report_window: None,
            lint_report: None,
            lints_toggle_window: None,
            lint_options: LintOptions::default(),
            problematic_mod_id: None,
        })
    }

    fn set_custom_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();

        fonts.font_data.insert(
            "my_font".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                "../../assets/NotoSansSC-Light.ttf"
            ))),
        );

        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "my_font".to_owned());

        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "my_font".to_owned());

        ctx.set_fonts(fonts);
    }

    fn ui_profile(&mut self, ui: &mut Ui, profile: &str) {
        let sorting_config = self.get_sorting_config();

        let ModData {
            profiles, groups, ..
        } = self.state.mod_data.deref_mut().deref_mut();

        struct Ctx {
            needs_save: bool,
            scroll_to_match: bool,
            btn_remove: Option<usize>,
            add_deps: Option<Vec<ModSpecification>>,
        }
        let mut ctx = Ctx {
            needs_save: false,
            scroll_to_match: self.scroll_to_match,
            btn_remove: None,
            add_deps: None,
        };

        let ui_profile = |ui: &mut Ui, profile: &mut ModProfile| {
            let enabled_specs = profile
                .mods
                .iter()
                .enumerate()
                .flat_map(|(i, m)| -> Box<dyn Iterator<Item = _>> {
                    match m {
                        ModOrGroup::Individual(mc) => {
                            Box::new(mc.enabled.then_some((Some(i), mc.spec.clone())).into_iter())
                        }
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                        } => Box::new(
                            enabled
                                .then(|| groups.get(group_name))
                                .flatten()
                                .into_iter()
                                .flat_map(|g| {
                                    g.mods
                                        .iter()
                                        .filter_map(|m| m.enabled.then_some((None, m.spec.clone())))
                                }),
                        ),
                    }
                })
                .collect::<Vec<_>>();

            let ui_mod_tags = |ctx: &mut Ctx, ui: &mut Ui, info: &ModInfo| {
                if let Some(ModioTags {
                    qol,
                    gameplay,
                    audio,
                    visual,
                    framework,
                    required_status,
                    approval_status,
                    versions: _,
                }) = info.modio_tags.as_ref()
                {
                    let mut mk_searchable_modio_tag =
                        |tag_str: &str,
                         ui: &mut Ui,
                         color: Option<egui::Color32>,
                         hover_str: Option<&str>| {
                            let search = searchable_text(tag_str, &self.search_string, {
                                TextFormat {
                                    color: if color.is_some() {
                                        Color32::BLACK
                                    } else {
                                        Color32::GRAY
                                    },

                                    ..Default::default()
                                }
                            });

                            let button = if let Some(color) = color {
                                egui::Button::new(search.job)
                                    .small()
                                    .fill(color)
                                    .stroke(egui::Stroke::NONE)
                            } else {
                                egui::Button::new(search.job)
                                    .small()
                                    .stroke(egui::Stroke::NONE)
                            };

                            let res = if let Some(hover_str) = hover_str {
                                ui.add_enabled(false, button)
                                    .on_disabled_hover_text(hover_str)
                            } else {
                                ui.add_enabled(false, button)
                            };

                            if search.is_match && self.scroll_to_match {
                                res.scroll_to_me(None);
                                ctx.scroll_to_match = false;
                            }
                        };

                    match approval_status {
                        ApprovalStatus::Verified => {
                            mk_searchable_modio_tag(
                                "Verified",
                                ui,
                                Some(egui::Color32::LIGHT_GREEN),
                                Some("Does not contain any gameplay affecting features or changes"),
                            );
                        }
                        ApprovalStatus::Approved => {
                            mk_searchable_modio_tag(
                                "Approved",
                                ui,
                                Some(egui::Color32::LIGHT_BLUE),
                                Some("Contains gameplay affecting features or changes"),
                            );
                        }
                        ApprovalStatus::Sandbox => {
                            mk_searchable_modio_tag(
                                "Sandbox",
                                ui,
                                Some(egui::Color32::LIGHT_YELLOW),
                                Some(
                                    "Contains significant, possibly progression breaking, changes to gameplay",
                                ),
                            );
                        }
                    }

                    match required_status {
                        RequiredStatus::RequiredByAll => {
                            mk_searchable_modio_tag(
                                "RequiredByAll",
                                ui,
                                Some(egui::Color32::LIGHT_RED),
                                Some(
                                    "All lobby members must use this mod for it to work correctly!",
                                ),
                            );
                        }
                        RequiredStatus::Optional => {
                            mk_searchable_modio_tag(
                                "Optional",
                                ui,
                                None,
                                Some("Clients are not required to install this mod to function"),
                            );
                        }
                    }

                    if *qol {
                        mk_searchable_modio_tag("QoL", ui, None, None);
                    }
                    if *gameplay {
                        mk_searchable_modio_tag("Gameplay", ui, None, None);
                    }
                    if *audio {
                        mk_searchable_modio_tag("Audio", ui, None, None);
                    }
                    if *visual {
                        mk_searchable_modio_tag("Visual", ui, None, None);
                    }
                    if *framework {
                        mk_searchable_modio_tag("Framework", ui, None, None);
                    }
                }
            };

            let mut ui_mod = |ctx: &mut Ctx,
                              ui: &mut Ui,
                              _group: Option<&str>,
                              row_index: usize,
                              mc: &mut ModConfig| {
                if !mc.enabled {
                    let vis = ui.visuals_mut();
                    vis.override_text_color = Some(vis.text_color());
                    vis.hyperlink_color = vis.text_color();
                }

                if ui
                    .add(toggle_switch(&mut mc.enabled))
                    .on_hover_text_at_pointer("Enabled?")
                    .changed()
                {
                    ctx.needs_save = true;
                }

                /*
                if ui
                    .add(egui::Checkbox::without_text(&mut mc.required))
                    .changed()
                {
                    needs_save = true;
                }
                */

                let info = self.state.store.get_mod_info(&mc.spec);

                if let Some(ref info) = info
                    && let Some(modio_id) = info.modio_id
                    && self.problematic_mod_id.is_some_and(|id| id == modio_id)
                {
                    let icon = egui::Button::new(RichText::new("❌").color(Color32::WHITE))
                        .fill(Color32::RED);
                    ui.add_enabled(false, icon);
                }

                if mc.enabled
                    && let Some(req) = &self.integrate_rid
                {
                    match req.state.get(&mc.spec) {
                        Some(SpecFetchProgress::Progress { progress, size }) => {
                            ui.add(
                                egui::ProgressBar::new(*progress as f32 / *size as f32)
                                    .show_percentage()
                                    .desired_width(100.0),
                            );
                        }
                        Some(SpecFetchProgress::Complete) => {
                            ui.add(egui::ProgressBar::new(1.0).desired_width(100.0));
                        }
                        None => {
                            ui.spinner();
                        }
                    }
                }

                if let Some(info) = &info {
                    egui::ComboBox::from_id_salt(row_index)
                        .selected_text(
                            self.state
                                .store
                                .get_version_name(&mc.spec)
                                .unwrap_or_default(),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut mc.spec.url,
                                info.spec.url.to_string(),
                                self.state
                                    .store
                                    .get_version_name(&info.spec)
                                    .unwrap_or_default(),
                            );
                            for version in info.versions.iter().rev() {
                                ui.selectable_value(
                                    &mut mc.spec.url,
                                    version.url.to_string(),
                                    self.state
                                        .store
                                        .get_version_name(version)
                                        .unwrap_or_default(),
                                );
                            }
                        });

                    ui.scope(|ui| {
                        ui.style_mut().spacing.interact_size.x = 30.;
                        let dark = ui.visuals().dark_mode;
                        match mc.priority.cmp(&0) {
                            std::cmp::Ordering::Less => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_RED
                                } else {
                                    Color32::DARK_RED
                                });
                            }
                            std::cmp::Ordering::Greater => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_GREEN
                                } else {
                                    Color32::DARK_GREEN
                                });
                            }
                            _ => {}
                        }
                        ui.add(
                            egui::DragValue::new(&mut mc.priority)
                                .custom_formatter(|n, _| {
                                    if n == 0. {
                                        "-".to_string()
                                    } else {
                                        format!("{n}")
                                    }
                                })
                                .speed(0.05)
                                .range(RangeInclusive::new(-999, 999)),
                        )
                        .on_hover_text_at_pointer(
                            "Load Priority\nIn case of asset conflict, mods with higher priority take precedent.\nCan have duplicate values.",
                        );
                    });

                    if ui
                        .button("📋")
                        .on_hover_text_at_pointer("copy URL")
                        .clicked()
                    {
                        ui.ctx().copy_text(mc.spec.url.to_string());
                    }

                    if let Some(modio_id) = info.modio_id
                        && let Some(modio_provider_params) =
                            self.state.config.provider_parameters.get("modio")
                        && let Some(oauth_token) = modio_provider_params.get("oauth")
                        && ui
                            .button("\u{2139}")
                            .on_hover_text_at_pointer("View details")
                            .clicked()
                    {
                        self.detailed_mod_info_windows
                            .insert(modio_id, WindowDetailedModInfo { info: info.clone() });
                        self.fetch_mod_details_rid.insert(
                            modio_id,
                            message::FetchModDetails::send(
                                &mut self.request_counter,
                                ui.ctx(),
                                self.tx.clone(),
                                oauth_token,
                                modio_id,
                            ),
                        );
                    }

                    if ui
                        .button("\u{1F4DD}")
                        .on_hover_text_at_pointer("Notes")
                        .clicked()
                    {
                        let note_key = ModNotes::get_note_key(info);
                        let current_note = self
                            .state
                            .mod_notes
                            .notes
                            .get(&note_key)
                            .cloned()
                            .unwrap_or_default();

                        self.notes_window.insert(
                            note_key.clone(),
                            WindowNotes {
                                mod_url: mc.spec.url.to_string(),
                                note_key,
                                note_text: current_note,
                            },
                        );
                    }

                    if mc.enabled {
                        let is_duplicate = enabled_specs.iter().any(|(i, spec)| {
                            Some(row_index) != *i && info.spec.satisfies_dependency(spec)
                        });
                        if is_duplicate
                            && ui
                                .button(
                                    egui::RichText::new("\u{26A0}")
                                        .color(ui.visuals().warn_fg_color),
                                )
                                .on_hover_text_at_pointer("remove duplicate")
                                .clicked()
                        {
                            ctx.btn_remove = Some(row_index);
                        }

                        let missing_deps = info
                            .suggested_dependencies
                            .iter()
                            .filter(|d| {
                                !enabled_specs.iter().any(|(_, s)| s.satisfies_dependency(d))
                            })
                            .collect::<Vec<_>>();

                        if !missing_deps.is_empty() {
                            let mut msg = "Add missing dependencies:".to_string();
                            for dep in &missing_deps {
                                msg.push('\n');
                                msg.push_str(&dep.url);
                            }
                            if ui
                                .button(
                                    egui::RichText::new("\u{26A0}")
                                        .color(ui.visuals().warn_fg_color),
                                )
                                .on_hover_text(msg)
                                .clicked()
                            {
                                ctx.add_deps = Some(missing_deps.into_iter().cloned().collect());
                            }
                        }
                    }

                    match info.provider {
                        "modio" => {
                            let texture: &egui::TextureHandle =
                                self.modio_texture_handle.get_or_insert_with(|| {
                                    let image = image::load_from_memory(MODIO_LOGO_PNG).unwrap();
                                    let size = [image.width() as _, image.height() as _];
                                    let image_buffer = image.to_rgba8();
                                    let pixels = image_buffer.as_flat_samples();
                                    let image = egui::ColorImage::from_rgba_unmultiplied(
                                        size,
                                        pixels.as_slice(),
                                    );

                                    ui.ctx()
                                        .load_texture("modio-logo", image, Default::default())
                                });
                            let mut img =
                                egui::Image::new(texture).fit_to_exact_size([16.0, 16.0].into());
                            if !mc.enabled {
                                img = img.tint(Color32::LIGHT_RED);
                            }
                            ui.add(img);
                        }
                        "http" => {
                            let texture: &egui::TextureHandle =
                                self.http_texture_handle.get_or_insert_with(|| {
                                    let image = image::load_from_memory(HTTP_LOGO_PNG).unwrap();
                                    let size = [image.width() as _, image.height() as _];
                                    let image_buffer = image.to_rgba8();
                                    let pixels = image_buffer.as_flat_samples();
                                    let image = egui::ColorImage::from_rgba_unmultiplied(
                                        size,
                                        pixels.as_slice(),
                                    );

                                    ui.ctx()
                                        .load_texture("http-logo", image, Default::default())
                                });
                            let mut img =
                                egui::Image::new(texture).fit_to_exact_size([16.0, 16.0].into());
                            if !mc.enabled {
                                img = img.tint(Color32::LIGHT_RED);
                            }
                            ui.add(img);
                        }
                        "file" => {
                            let texture: &egui::TextureHandle =
                                self.folder_texture_handle.get_or_insert_with(|| {
                                    let image = image::load_from_memory(FOLDER_LOGO_PNG).unwrap();
                                    let size = [image.width() as _, image.height() as _];
                                    let image_buffer = image.to_rgba8();
                                    let pixels = image_buffer.as_flat_samples();
                                    let image = egui::ColorImage::from_rgba_unmultiplied(
                                        size,
                                        pixels.as_slice(),
                                    );

                                    ui.ctx()
                                        .load_texture("folder-logo", image, Default::default())
                                });
                            let mut img =
                                egui::Image::new(texture).fit_to_exact_size([16.0, 16.0].into());
                            if !mc.enabled {
                                img = img.tint(Color32::LIGHT_RED);
                            }
                            ui.add(img);
                        }
                        _ => unimplemented!("unimplemented provider kind"),
                    }

                    let search = searchable_text(&info.name, &self.search_string, {
                        TextFormat {
                            color: ui.visuals().hyperlink_color,
                            ..Default::default()
                        }
                    });

                    let res = ui.hyperlink_to(search.job, &mc.spec.url);
                    if search.is_match && self.scroll_to_match {
                        res.scroll_to_me(None);
                        ctx.scroll_to_match = false;
                    }

                    let note_key = ModNotes::get_note_key(info);
                    if let Some(note) = self.state.mod_notes.notes.get(&note_key) {
                        let max_chars = ((ui.available_width()
                            / (ui.text_style_height(&egui::TextStyle::Body) * 0.6))
                            as usize)
                            .saturating_sub(3)
                            .max(5);
                        let note_sum = note.chars().take(max_chars).collect::<String>();
                        let note_str = if note.chars().count() > max_chars {
                            format!("{note_sum}...")
                        } else {
                            note_sum
                        };

                        ui.label(egui::RichText::new(note_str).color(Color32::GRAY))
                            .on_hover_text(note);
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui_mod_tags(ctx, ui, info);
                    });
                } else {
                    if ui
                        .button("📋")
                        .on_hover_text_at_pointer("Copy URL")
                        .clicked()
                    {
                        ui.ctx().copy_text(mc.spec.url.to_string());
                    }

                    let search = searchable_text(&mc.spec.url, &self.search_string, {
                        TextFormat {
                            color: ui.visuals().hyperlink_color,
                            ..Default::default()
                        }
                    });

                    let res = ui.hyperlink_to(search.job, &mc.spec.url);
                    if search.is_match && self.scroll_to_match {
                        res.scroll_to_me(None);
                        ctx.scroll_to_match = false;
                    }
                }
            };

            let mut ui_item =
                |ctx: &mut Ctx, ui: &mut Ui, mc: &mut ModOrGroup, row_index: usize| {
                    ui.scope(|ui| {
                        ui.visuals_mut().widgets.hovered.weak_bg_fill = colors::DARK_RED;
                        ui.visuals_mut().widgets.active.weak_bg_fill = colors::DARKER_RED;
                        if ui
                            .add(Button::new(" 🗑 "))
                            .on_hover_text_at_pointer("Delete mod")
                            .clicked()
                        {
                            ctx.btn_remove = Some(row_index);
                        };
                    });

                    match mc {
                        ModOrGroup::Individual(mc) => {
                            ui_mod(ctx, ui, None, row_index, mc);
                        }
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                        } => {
                            if ui
                                .add(toggle_switch(enabled))
                                .on_hover_text_at_pointer("Enabled?")
                                .changed()
                            {
                                ctx.needs_save = true;
                            }
                            ui.collapsing(group_name.as_str(), |ui| {
                                for (index, m) in groups
                                    .get_mut(group_name)
                                    .unwrap()
                                    .mods
                                    .iter_mut()
                                    .enumerate()
                                {
                                    ui.horizontal(|ui| ui_mod(ctx, ui, Some(group_name), index, m));
                                }
                            });
                        }
                    }
                };

            if let Some(sorting_config) = sorting_config {
                let comp = sort_mods(sorting_config);
                profile
                    .mods
                    .iter_mut()
                    .map(|m| {
                        // fetch ModInfo up front because doing it in the comparator is slow
                        let ModOrGroup::Individual(mc) = m else {
                            unimplemented!("Item is not Individual \n{:?}", m);
                        };
                        let info = self.state.store.get_mod_info(&mc.spec);
                        (m, info)
                    })
                    .enumerate()
                    .sorted_by(|a, b| comp((a.1.0, a.1.1.as_ref()), (b.1.0, b.1.1.as_ref())))
                    .enumerate()
                    .for_each(|(visual_index, (store_index, item))| {
                        let mut frame = egui::Frame::NONE;
                        if visual_index % 2 == 1 {
                            frame.fill = ui.visuals().faint_bg_color
                        }
                        frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui_item(&mut ctx, ui, item.0, store_index);
                            });
                        });
                    });
            } else {
                let res = egui_dnd::dnd(ui, ui.scope_id())
                    .with_mouse_config(egui_dnd::DragDropConfig::mouse())
                    .show(
                        profile.mods.iter_mut().enumerate(),
                        |ui, (_index, item), handle, state| {
                            let mut frame = egui::Frame::NONE;
                            if state.dragged {
                                frame.fill = ui.visuals().extreme_bg_color
                            } else if state.index % 2 == 1 {
                                frame.fill = ui.visuals().faint_bg_color
                            }
                            frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    handle.ui(ui, |ui| {
                                        ui.label("   ☰  ");
                                    });

                                    ui_item(&mut ctx, ui, item, state.index);
                                });
                            });
                        },
                    );

                if res.final_update().is_some() {
                    res.update_vec(&mut profile.mods);
                    ctx.needs_save = true;
                }
            }
            if let Some(remove) = ctx.btn_remove {
                profile.mods.remove(remove);
                ctx.needs_save = true;
            }
            if let Some(index) = self.pending_delete {
                if self.state.config.confirm_deletion {
                    egui::Window::new("Delete Mod")
                        .collapsible(false)
                        .resizable(false)
                        .show(ui.ctx(), |ui| {
                            ui.add_space(10.);
                            ui.label("Are you sure you want to delete this mod from the current profile? This cannot be undone.");
                            ui.add_space(10.);

                            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Delete").color(Color32::WHITE),
                                        )
                                        .fill(Color32::DARK_RED),
                                    )
                                    .clicked()
                                {
                                    profile.mods.remove(index);
                                    ctx.needs_save = true;
                                    self.pending_delete = None;
                                }

                                if ui.button("Cancel").clicked() {
                                    self.pending_delete = None;
                                }
                            });
                        });
                } else {
                    profile.mods.remove(index);
                    ctx.needs_save = true;
                    self.pending_delete = None;
                }
            }
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(profile) = profiles.get_mut(profile) {
                ui_profile(ui, profile);
            } else {
                ui.label("no such profile");
            }
        });

        if let Some(index) = ctx.btn_remove {
            self.pending_delete = Some(index);
        }

        if let Some(add_deps) = ctx.add_deps {
            message::ResolveMods::send(self, ui.ctx(), add_deps, true);
            self.problematic_mod_id = None;
        }

        self.scroll_to_match = ctx.scroll_to_match;

        if ctx.needs_save {
            self.state.mod_data.save().unwrap();
        }
    }

    fn parse_mods(&self) -> Vec<ModSpecification> {
        self.resolve_mod
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| ModSpecification::new(l.to_string()))
            .collect()
    }

    fn build_mod_string(mods: &Vec<ModConfig>) -> String {
        let mut string = String::new();
        for m in mods {
            if m.enabled {
                string.push_str(&m.spec.url);
                string.push('\n');
            }
        }
        string
    }

    fn show_provider_parameters(&mut self, ctx: &egui::Context) {
        let Some(window) = &mut self.window_provider_parameters else {
            return;
        };

        while let Ok((rid, res)) = window.rx.try_recv() {
            if window.check_rid.as_ref().is_some_and(|r| rid == r.0) {
                match res {
                    Ok(()) => {
                        let window = self.window_provider_parameters.take().unwrap();
                        self.state
                            .config
                            .provider_parameters
                            .insert(window.factory.id.to_string(), window.parameters);
                        self.state.config.save().unwrap();
                        return;
                    }
                    Err(e) => {
                        window.check_error = Some(e.to_string());
                    }
                }
                window.check_rid = None;
            }
        }

        let mut open = true;
        let mut check = false;
        egui::Window::new(format!("Configure {} provider", window.factory.id))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_enabled_ui(window.check_rid.is_none(), |ui| {
                    egui::Grid::new("grid").num_columns(2).show(ui, |ui| {
                        for p in window.factory.parameters {
                            if let Some(link) = p.link {
                                ui.hyperlink_to(p.name, link).on_hover_text(p.description);
                            } else {
                                ui.label(p.name).on_hover_text(p.description);
                            }
                            let res = ui.add(
                                egui::TextEdit::singleline(
                                    window.parameters.entry(p.id.to_string()).or_default(),
                                )
                                .password(true)
                                .desired_width(200.0),
                            );
                            if is_committed(&res) {
                                check = true;
                            }
                            ui.end_row();
                        }
                    });

                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        if ui.button("Save").clicked() {
                            check = true;
                        }
                        if window.check_rid.is_some() {
                            ui.spinner();
                        }
                        if let Some(error) = &window.check_error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });
                });
            });
        if !open {
            self.window_provider_parameters = None;
        } else if check {
            window.check_error = None;
            let tx = window.tx.clone();
            let ctx = ctx.clone();
            let rid = self.request_counter.next();
            let store = self.state.store.clone();
            let params = window.parameters.clone();
            let factory = window.factory;
            let handle = tokio::task::spawn(async move {
                let res = store.add_provider_checked(factory, &params).await;
                tx.send((rid, res)).await.unwrap();
                ctx.request_repaint();
            });
            window.check_rid = Some((rid, handle));
        }
    }

    fn show_profile_windows(&mut self, ctx: &egui::Context) {
        let mut to_remove = vec![];
        for profile in &self.open_profiles.clone() {
            let mut open = true;
            egui::Window::new(format!("Profile \"{profile}\""))
                .open(&mut open)
                .show(ctx, |ui| {
                    self.ui_profile(ui, profile);
                });
            if !open {
                to_remove.push(profile.clone());
            }
        }
        for r in to_remove {
            self.open_profiles.remove(&r);
        }
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        if let Some(window) = &mut self.settings_window {
            let mut open = true;
            let mut try_save = false;
            egui::Window::new("Settings")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::Grid::new("grid").num_columns(2).striped(true).show(ui, |ui| {
                        let mut job = LayoutJob::default();
                        job.append(
                            "DRG pak",
                            0.0,
                            TextFormat {
                                color: ui.visuals().text_color(),
                                underline: Stroke::new(1.0_f32, ui.visuals().text_color()),
                                ..Default::default()
                            },
                        );
                        ui.label(job).on_hover_cursor(egui::CursorIcon::Help).on_hover_text("Path to FSD-WindowsNoEditor.pak\nLocated inside the \"Deep Rock Galactic\" installation directory under FSD/Content/Paks.");
                        ui.horizontal(|ui| {
                            let res = ui.add(
                                egui::TextEdit::singleline(
                                    &mut window.drg_pak_path
                                )
                                .desired_width(200.0),
                            );
                            if res.changed() {
                                window.drg_pak_path_err = None;
                            }
                            if is_committed(&res) {
                                try_save = true;
                            }
                            if ui.button("browse").clicked()
                                && let Some(fsd_pak) = rfd::FileDialog::new()
                                    .add_filter("DRG Pak", &["pak"])
                                    .pick_file()
                                {
                                    window.drg_pak_path = fsd_pak.to_string_lossy().to_string();
                                    window.drg_pak_path_err = None;
                                }
                        });
                        ui.end_row();

                        let config_dir = &self.state.dirs.config_dir;
                        ui.label("Config directory:");
                        if ui.link(config_dir.display().to_string()).clicked() {
                            opener::open(config_dir).ok();
                        }
                        ui.end_row();

                        let cache_dir = &self.state.dirs.cache_dir;
                        ui.label("Cache directory:");
                        if ui.link(cache_dir.display().to_string()).clicked() {
                            opener::open(cache_dir).ok();
                        }
                        ui.end_row();

                        let data_dir = &self.state.dirs.data_dir;
                        ui.label("Data directory:");
                        if ui.link(data_dir.display().to_string()).clicked() {
                            opener::open(data_dir).ok();
                        }
                        ui.end_row();

                        ui.label("GUI theme:");
                        ui.horizontal(|ui| {
                            ui.horizontal(|ui| {
                                let config = &mut self.state.config;

                                let old_theme = GuiTheme::into_egui_theme(config.gui_theme);
                                let mut theme = old_theme;
                                theme.buttons(ui);
                                if theme != old_theme {
                                    ui.memory_mut(|m| m.options.theme_preference = theme);
                                    config.gui_theme = GuiTheme::from_egui_theme(theme);
                                    config.save().unwrap();
                                }
                            });
                        });
                        ui.end_row();

                        ui.label("Confirm delete:");
                        ui.horizontal(|ui| {
                            let config = &mut self.state.config;
                            if ui
                                .add(toggle_switch(&mut config.confirm_deletion))
                                .changed()
                            {
                                config.save().unwrap();
                            }
                        });
                        ui.end_row();

                        ui.label("Mod providers:");
                        ui.end_row();

                        for provider_factory in ModStore::get_provider_factories() {
                            ui.label(provider_factory.id);
                            if ui.add_enabled(!provider_factory.parameters.is_empty(), egui::Button::new("⚙"))
                                    .on_hover_text(format!("Open \"{}\" settings", provider_factory.id))
                                    .clicked() {
                                self.window_provider_parameters = Some(
                                    WindowProviderParameters::new(provider_factory, &self.state),
                                );
                            }
                            ui.end_row();
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                        if ui.add_enabled(window.drg_pak_path_err.is_none(), egui::Button::new("save")).clicked() {
                            try_save = true;
                        }
                        if let Some(error) = &window.drg_pak_path_err {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });

                });
            if try_save {
                if let Err(e) = is_drg_pak(&window.drg_pak_path) {
                    window.drg_pak_path_err = Some(e.to_string());
                } else {
                    self.state.config.drg_pak_path = Some(PathBuf::from(
                        self.settings_window.take().unwrap().drg_pak_path,
                    ));
                    self.state.config.save().unwrap();
                }
            } else if !open {
                self.settings_window = None;
            }
        }
    }

    fn show_lints_toggle(&mut self, ctx: &egui::Context) {
        if let Some(_lints_toggle) = &self.lints_toggle_window {
            let mut open = true;

            egui::Window::new("Toggle lints")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::Grid::new("lints-toggle-grid").show(ui, |ui| {
                            ui.heading("Lint");
                            ui.heading("Enabled?");
                            ui.end_row();

                            ui.label("Archive with multiple paks");
                            ui.add(toggle_switch(
                                &mut self.lint_options.archive_with_multiple_paks,
                            ));
                            ui.end_row();

                            ui.label("Archive with only non-pak files");
                            ui.add(toggle_switch(
                                &mut self.lint_options.archive_with_only_non_pak_files,
                            ));
                            ui.end_row();

                            ui.label("Mods containing AssetRegister.bin");
                            ui.add(toggle_switch(&mut self.lint_options.asset_register_bin));
                            ui.end_row();

                            ui.label("Mods containing conflicting files");
                            ui.add(toggle_switch(&mut self.lint_options.conflicting));
                            ui.end_row();

                            ui.label("Mods containing empty archives");
                            ui.add(toggle_switch(&mut self.lint_options.empty_archive));
                            ui.end_row();

                            ui.label("Mods containing oudated pak version");
                            ui.add(toggle_switch(&mut self.lint_options.outdated_pak_version));
                            ui.end_row();

                            ui.label("Mods containing shader files");
                            ui.add(toggle_switch(&mut self.lint_options.shader_files));
                            ui.end_row();

                            ui.label("Mods containing non-asset files");
                            ui.add(toggle_switch(&mut self.lint_options.non_asset_files));
                            ui.end_row();

                            ui.label("Mods containing split {uexp, uasset} pairs");
                            ui.add(toggle_switch(&mut self.lint_options.split_asset_pairs));
                            ui.end_row();

                            ui.label("Mods containing unmodified game assets");
                            ui.add_enabled(
                                self.state.config.drg_pak_path.is_some(),
                                toggle_switch(&mut self.lint_options.unmodified_game_assets),
                            )
                            .on_disabled_hover_text(
                                "This lint requires DRG pak path to be specified",
                            );
                            ui.end_row();
                        });
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.lints_toggle_window = None;
                        }

                        if ui
                            .add_enabled(
                                self.integrate_rid.is_none() && self.lint_rid.is_none(),
                                egui::Button::new("Generate report"),
                            )
                            .clicked()
                        {
                            let lint_options = BTreeMap::from([
                                (
                                    LintId::ARCHIVE_WITH_MULTIPLE_PAKS,
                                    self.lint_options.archive_with_multiple_paks,
                                ),
                                (
                                    LintId::ARCHIVE_WITH_ONLY_NON_PAK_FILES,
                                    self.lint_options.archive_with_only_non_pak_files,
                                ),
                                (
                                    LintId::ASSET_REGISTRY_BIN,
                                    self.lint_options.asset_register_bin,
                                ),
                                (LintId::CONFLICTING, self.lint_options.conflicting),
                                (LintId::EMPTY_ARCHIVE, self.lint_options.empty_archive),
                                (
                                    LintId::OUTDATED_PAK_VERSION,
                                    self.lint_options.outdated_pak_version,
                                ),
                                (LintId::SHADER_FILES, self.lint_options.shader_files),
                                (LintId::NON_ASSET_FILES, self.lint_options.non_asset_files),
                                (
                                    LintId::SPLIT_ASSET_PAIRS,
                                    self.lint_options.split_asset_pairs,
                                ),
                                (
                                    LintId::UNMODIFIED_GAME_ASSETS,
                                    self.lint_options.unmodified_game_assets,
                                ),
                            ]);

                            trace!(?lint_options);

                            let mut mods = Vec::new();
                            self.state.mod_data.for_each_enabled_mod(
                                &self.state.mod_data.active_profile,
                                |mc| {
                                    mods.push(mc.spec.clone());
                                },
                            );

                            self.lint_report = None;
                            self.lint_rid = Some(message::LintMods::send(
                                &mut self.request_counter,
                                self.state.store.clone(),
                                mods,
                                BTreeSet::from_iter(
                                    lint_options
                                        .into_iter()
                                        .filter_map(|(lint, enabled)| enabled.then_some(lint)),
                                ),
                                self.state.config.drg_pak_path.clone(),
                                self.tx.clone(),
                                ctx.clone(),
                            ));
                            self.problematic_mod_id = None;
                            self.lint_report_window = Some(WindowLintReport);
                        }
                    });
                });

            if !open {
                self.lints_toggle_window = None;
            }
        }
    }

    fn show_lint_report(&mut self, ctx: &egui::Context) {
        if self.lint_report_window.is_some() {
            let mut open = true;

            egui::Window::new("Lint results")
                .open(&mut open)
                .resizable(true)
                .show(ctx, |ui| {
                    if let Some(report) = &self.lint_report {
                        let scroll_height =
                            (ui.available_height() - 30.0).clamp(0.0, f32::INFINITY);
                        egui::ScrollArea::vertical()
                            .max_height(scroll_height)
                            .show(ui, |ui| {
                                const AMBER: Color32 = Color32::from_rgb(255, 191, 0);

                                if let Some(conflicting_mods) = &report.conflicting_mods
                                    && !conflicting_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new("⚠ Mods(s) with conflicting asset modifications detected")
                                                .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            conflicting_mods.iter().for_each(|(path, mods)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ Conflicting modification of asset `{path}`"
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(
                                                    ui,
                                                    |ui| {
                                                        mods.iter().for_each(|mod_spec| {
                                                            ui.label(&mod_spec.url);
                                                        });
                                                    },
                                                );
                                            });
                                        });
                                    }

                                if let Some(asset_register_bin_mods) = &report.asset_register_bin_mods
                                    && !asset_register_bin_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new("ℹ Mod(s) with `AssetRegistry.bin` included detected")
                                                .color(Color32::LIGHT_BLUE),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            asset_register_bin_mods.iter().for_each(
                                                |(r#mod, paths)| {
                                                    CollapsingHeader::new(
                                                        RichText::new(format!(
                                                        "ℹ {} includes one or more `AssetRegistry.bin`",
                                                        r#mod.url
                                                    ))
                                                        .color(Color32::LIGHT_BLUE),
                                                    )
                                                    .show(ui, |ui| {
                                                        paths.iter().for_each(|path| {
                                                            ui.label(path);
                                                        });
                                                    });
                                                },
                                            );
                                        });
                                    }

                                if let Some(shader_file_mods) = &report.shader_file_mods
                                    && !shader_file_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mods(s) with shader files included detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            shader_file_mods.iter().for_each(
                                                |(r#mod, shader_files)| {
                                                    CollapsingHeader::new(
                                                        RichText::new(format!(
                                                            "⚠ {} includes one or more shader files",
                                                            r#mod.url
                                                        ))
                                                        .color(AMBER),
                                                    )
                                                    .show(ui, |ui| {
                                                        shader_files.iter().for_each(|shader_file| {
                                                            ui.label(shader_file);
                                                        });
                                                    });
                                                },
                                            );
                                        });
                                    }

                                if let Some(outdated_pak_version_mods) = &report.outdated_pak_version_mods
                                    && !outdated_pak_version_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with outdated pak version detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            outdated_pak_version_mods.iter().for_each(
                                                |(r#mod, version)| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "⚠ {} includes outdated pak version {}",
                                                            r#mod.url, version
                                                        ))
                                                        .color(AMBER),
                                                    );
                                                },
                                            );
                                        });
                                    }

                                if let Some(empty_archive_mods) = &report.empty_archive_mods
                                    && !empty_archive_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with empty archives detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            empty_archive_mods.iter().for_each(|r#mod| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "⚠ {} contains an empty archive",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                );
                                            });
                                        });
                                    }

                                if let Some(archive_with_only_non_pak_files_mods) = &report.archive_with_only_non_pak_files_mods
                                    && !archive_with_only_non_pak_files_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with only non-`.pak` files detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            archive_with_only_non_pak_files_mods.iter().for_each(|r#mod| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "⚠ {} contains only non-`.pak` files, perhaps the author forgot to pack it?",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                );
                                            });
                                        });
                                    }

                                if let Some(archive_with_multiple_paks_mods) = &report.archive_with_multiple_paks_mods
                                    && !archive_with_multiple_paks_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with multiple `.pak`s detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            archive_with_multiple_paks_mods.iter().for_each(|r#mod| {
                                                ui.label(RichText::new(format!(
                                                    "⚠ {} contains multiple `.pak`s, only the first encountered `.pak` will be loaded",
                                                    r#mod.url
                                                ))
                                                .color(AMBER));
                                            });
                                        });
                                    }

                                if let Some(non_asset_file_mods) = &report.non_asset_file_mods
                                    && !non_asset_file_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with non-asset files detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            non_asset_file_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes non-asset files",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|file| {
                                                        ui.label(file);
                                                    });
                                                });
                                            });
                                        });
                                    }

                                if let Some(split_asset_pairs_mods) = &report.split_asset_pairs_mods
                                    && !split_asset_pairs_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with split {uexp, uasset} pairs detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            split_asset_pairs_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes split {{uexp, uasset}} pairs",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|(file, kind)| {
                                                        match kind {
                                                            SplitAssetPair::MissingUasset => {
                                                                ui.label(format!("`{file}` missing matching .uasset file"));
                                                            },
                                                            SplitAssetPair::MissingUexp => {
                                                                ui.label(format!("`{file}` missing matching .uexp file"));
                                                            }
                                                        }
                                                    });
                                                });
                                            });
                                        });
                                    }

                                if let Some(unmodified_game_assets_mods) = &report.unmodified_game_assets_mods
                                    && !unmodified_game_assets_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with unmodified game assets detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            unmodified_game_assets_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes unmodified game assets",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|file| {
                                                        ui.label(file);
                                                    });
                                                });
                                            });
                                        });
                                    }
                            });
                    } else {
                        ui.spinner();
                        ui.label("Lint report generating...");
                    }
                });

            if !open {
                self.lint_report_window = None;
                self.lint_rid = None;
            }
        }
    }

    fn show_about(&mut self, ctx: &egui::Context) {
        if self.about_window.is_some() {
            let mut open = true;

            egui::Window::new("About")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.add_space(10.);
                    ui.vertical_centered(|ui| {
                        let texture: &egui::TextureHandle =
                            self.header_texture_handle.get_or_insert_with(|| {
                                let image = image::load_from_memory(HEADER_PNG).unwrap();
                                let size = [image.width() as _, image.height() as _];
                                let image_buffer = image.to_rgba8();
                                let pixels = image_buffer.as_flat_samples();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    size,
                                    pixels.as_slice(),
                                );

                                ui.ctx()
                                    .load_texture("header", color_image, Default::default())
                            });

                        ui.add(egui::Image::new(texture));
                    });
                    ui.add_space(10.);

                    ui.label("Third-party mod integration tool for Deep Rock Galactic.");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label("For more information about this fork, see the ");
                        ui.hyperlink_to(
                            "README",
                            "https://github.com/ile-du-fantome/mintnix/blob/master/README.md",
                        );
                        ui.label(".");
                    });
                    ui.add_space(10.);

                    ui.hyperlink_to(
                        "Changelog",
                        "https://github.com/ile-du-fantome/mintnix/blob/master/CHANGELOG.md",
                    );
                    ui.hyperlink_to(
                        "License",
                        "https://github.com/ile-du-fantome/mintnix/blob/master/LICENSE",
                    );
                    ui.hyperlink_to("GitHub", "https://github.com/ile-du-fantome/mintnix");
                    ui.add_space(10.);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label("App icon \"Rock\" from ");
                        ui.hyperlink_to("Icons8", "https://icons8.com/icon/6zZTdWRZoWil/rock");
                        ui.label(".");
                    });
                    ui.add_space(10.);

                    ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                        if ui.button("Close").clicked() {
                            self.about_window = None;
                        }
                    });
                });
            if !open {
                self.about_window = None;
            }
        }
    }

    fn show_notes(&mut self, ctx: &egui::Context) {
        let mut to_remove = Vec::new();

        for (note_key, editor) in &mut self.notes_window {
            let mut open = true;
            let mod_spec = ModSpecification::new(editor.mod_url.clone());
            let mod_name = self
                .state
                .store
                .get_mod_info(&mod_spec)
                .map(|info| info.name.clone())
                .unwrap_or_else(|| editor.mod_url.clone());

            egui::Window::new(format!("Notes: {mod_name}"))
                .open(&mut open)
                .collapsible(true)
                .default_height(300.)
                .resizable(false)
                .show(ctx, |ui| {
                    let char_count = editor.note_text.chars().count();
                    let char_limit = 128;

                    ui.add_space(6.);
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.note_text)
                            .cursor_at_end(false)
                            .desired_width(f32::INFINITY)
                            .char_limit(char_limit),
                    );
                    ui.add_space(6.);

                    ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                        if ui.button("Save").clicked() {
                            if editor.note_text.is_empty() {
                                self.state.mod_notes.notes.remove(&editor.note_key);
                            } else {
                                self.state
                                    .mod_notes
                                    .notes
                                    .insert(editor.note_key.clone(), editor.note_text.clone());
                            }
                            self.state.mod_notes.save().unwrap();
                            to_remove.push(note_key.clone());
                        }
                        ui.with_layout(egui::Layout::left_to_right(Align::TOP), |ui| {
                            ui.label(format!("{char_count}/{char_limit} characters"));
                        });
                    });
                });

            if !open {
                to_remove.push(note_key.clone());
            }
        }

        for key in to_remove {
            self.notes_window.remove(&key);
        }
    }

    fn show_detailed_mod_info(&mut self, ctx: &egui::Context, modio_id: u32) {
        let mut to_remove = Vec::new();

        if let Some(WindowDetailedModInfo { info }) = self.detailed_mod_info_windows.get(&modio_id)
        {
            let mut open = true;

            egui::Window::new(&info.name)
                .open(&mut open)
                .collapsible(false)
                .movable(true)
                .resizable(false)
                .show(ctx, |ui| self.show_detailed_mod_info_inner(ui, modio_id));

            if !open {
                to_remove.push(modio_id);
            }
        }

        for id in to_remove {
            self.detailed_mod_info_windows.remove(&id);
            self.mod_details.remove(&id);
            self.fetch_mod_details_rid.remove(&id);
            self.mod_details_thumbnail_texture_handle.remove(&id);
        }
    }

    fn show_detailed_mod_info_inner(&mut self, ui: &mut egui::Ui, modio_id: u32) {
        if let Some(mod_details) = &self.mod_details.get(&modio_id) {
            let scroll_area_height = (ui.available_height() - 60.0).clamp(0.0, f32::INFINITY);

            egui::ScrollArea::vertical()
                .max_height(scroll_area_height)
                .max_width(f32::INFINITY)
                .auto_shrink([false, false])
                .stick_to_right(true)
                .show(ui, |ui| {
                    let texture: &egui::TextureHandle = self
                        .mod_details_thumbnail_texture_handle
                        .entry(modio_id)
                        .or_insert_with(|| {
                            ui.ctx().load_texture(
                                format!("{} image", mod_details.r#mod.name),
                                {
                                    let image =
                                        image::load_from_memory(&mod_details.thumbnail).unwrap();
                                    let size = [image.width() as _, image.height() as _];
                                    let image_buffer = image.to_rgb8();
                                    let pixels = image_buffer.as_flat_samples();
                                    egui::ColorImage::from_rgb(size, pixels.as_slice())
                                },
                                Default::default(),
                            )
                        });
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Image::new(texture));
                    });

                    ui.heading("Uploader");
                    ui.label(&mod_details.r#mod.submitted_by.username);
                    ui.add_space(10.0);

                    ui.heading("Description");
                    if let Some(desc) = &mod_details.r#mod.description_plaintext {
                        ui.label(desc);
                    } else {
                        ui.label("No description provided.");
                    }
                    ui.add_space(10.0);

                    ui.heading("Versions and Changelog");
                    ui.label(
                        RichText::new("Only the 10 most recent versions are shown.")
                            .color(Color32::GRAY),
                    );
                    egui::Grid::new("mod-details-available-versions")
                        .spacing(Vec2::new(3.0, 10.0))
                        .striped(true)
                        .num_columns(2)
                        .show(ui, |ui| {
                            mod_details.versions.iter().for_each(|file| {
                                if let Some(version) = &file.version {
                                    ui.label(version);
                                } else {
                                    ui.label("Unknown version");
                                }
                                if let Some(changelog) = &file.changelog {
                                    ui.add(Label::new(changelog).wrap());
                                } else {
                                    ui.label("N/A");
                                }
                                ui.end_row();
                            });
                        });
                    ui.add_space(10.0);

                    ui.heading("Files");
                    if let Some(file) = &mod_details.r#mod.modfile {
                        ui.horizontal(|ui| {
                            if let Some(version) = &file.version {
                                ui.label(version);
                            } else {
                                ui.label("Unknown version");
                            }
                            ui.hyperlink(&file.download.binary_url);
                        });
                    } else {
                        ui.label("No files provided.");
                    }
                });
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Fetching mod details from mod.io...");
            });
        }
    }

    fn get_sorting_config(&self) -> Option<SortingConfig> {
        self.state.config.sorting_config.clone()
    }

    fn update_sorting_config(&mut self, sort_category: Option<SortBy>, is_ascending: bool) {
        self.state.config.sorting_config = sort_category.map(|sort_category| SortingConfig {
            sort_category,
            is_ascending,
        });
        self.state.config.save().unwrap();
    }
}

type ModListEntry<'a> = (&'a ModOrGroup, Option<&'a ModInfo>);
fn sort_mods(config: SortingConfig) -> impl Fn(ModListEntry, ModListEntry) -> Ordering {
    move |(a, info_a), (b, info_b)| {
        if matches!(a, ModOrGroup::Group { .. }) || matches!(b, ModOrGroup::Group { .. }) {
            unimplemented!("Groups in sorting not implemented");
        }

        let ModOrGroup::Individual(mc_a) = a else {
            debug!("Item is not Individual \n{:?}", a);
            return Ordering::Equal;
        };
        let ModOrGroup::Individual(mc_b) = b else {
            debug!("Item is not Individual \n{:?}", b);
            return Ordering::Equal;
        };

        fn map_cmp<V, M, F>(a: &V, b: &V, map: F) -> Ordering
        where
            M: Ord,
            F: Fn(&V) -> M,
        {
            map(a).cmp(&map(b))
        }

        let name_order = map_cmp(&(mc_a, info_a), &(mc_b, info_b), |(mc, info)| {
            (info.map(|i| i.name.to_lowercase()), &mc.spec.url)
        });
        let provider_order = map_cmp(&info_a, &info_b, |info| info.map(|i| i.provider));
        let approval_order = map_cmp(&info_a, &info_b, |info| {
            info.and_then(|i| i.modio_tags.as_ref())
                .map(|t| t.approval_status)
        });
        let required_order = map_cmp(&info_a, &info_b, |info| {
            info.and_then(|i| i.modio_tags.as_ref())
                .map(|t| std::cmp::Reverse(t.required_status))
        });
        let mut order = match config.sort_category {
            SortBy::Enabled => mc_b.enabled.cmp(&mc_a.enabled),
            SortBy::Name => name_order,
            SortBy::Priority => mc_a.priority.cmp(&mc_b.priority),
            SortBy::Provider => provider_order,
            SortBy::RequiredStatus => required_order,
            SortBy::ApprovalCategory => approval_order,
        };

        if config.is_ascending {
            order = order.reverse();
        }
        if config.sort_category != SortBy::Name {
            order = order.then(name_order);
        }
        order
    }
}

struct WindowProviderParameters {
    tx: Sender<(RequestID, Result<(), ProviderError>)>,
    rx: Receiver<(RequestID, Result<(), ProviderError>)>,
    check_rid: Option<(RequestID, JoinHandle<()>)>,
    check_error: Option<String>,
    factory: &'static ProviderFactory,
    parameters: HashMap<String, String>,
}

impl WindowProviderParameters {
    fn new(factory: &'static ProviderFactory, state: &State) -> Self {
        let (tx, rx) = mpsc::channel(10);
        Self {
            tx,
            rx,
            check_rid: None,
            check_error: None,
            parameters: state
                .config
                .provider_parameters
                .get(factory.id)
                .cloned()
                .unwrap_or_default(),
            factory,
        }
    }
}

struct WindowSettings {
    drg_pak_path: String,
    drg_pak_path_err: Option<String>,
}

impl WindowSettings {
    fn new(state: &State) -> Self {
        let path = state
            .config
            .drg_pak_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        Self {
            drg_pak_path: path,
            drg_pak_path_err: None,
        }
    }
}

struct WindowLintReport;

struct WindowLintsToggle;

struct WindowAbout;

struct WindowNotes {
    mod_url: String,
    note_key: String,
    note_text: String,
}

struct WindowDetailedModInfo {
    info: ModInfo,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // do some init things that depend on ctx so cannot be done earlier
        if !self.has_run_init {
            self.has_run_init = true;

            let theme = GuiTheme::into_egui_theme(self.state.config.gui_theme);
            ctx.memory_mut(|m| m.options.theme_preference = theme);
        }

        // message handling
        while let Ok(msg) = self.rx.try_recv() {
            msg.handle(self);
        }

        // begin draw

        self.show_provider_parameters(&ctx);
        self.show_profile_windows(&ctx);
        self.show_about(&ctx);
        self.show_settings(&ctx);
        self.show_notes(&ctx);
        self.show_lints_toggle(&ctx);
        self.show_lint_report(&ctx);

        let modio_ids = self
            .detailed_mod_info_windows
            .keys()
            .copied()
            .collect::<Vec<_>>();

        for modio_id in modio_ids {
            self.show_detailed_mod_info(&ctx, modio_id);
        }

        egui::Panel::bottom("bottom_panel").show(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                ui.add_enabled_ui(
                    self.integrate_rid.is_none()
                        && self.update_rid.is_none()
                        && self.lint_rid.is_none()
                        && self.state.config.drg_pak_path.is_some(),
                    |ui| {
                        if let Some(args) = &self.args
                            && ui
                                .button("Launch game")
                                .on_hover_ui(|ui| {
                                    for arg in args {
                                        ui.label(arg);
                                    }
                                })
                                .clicked()
                        {
                            let args = args.clone();
                            std::thread::spawn(move || {
                                let mut iter = args.iter();
                                std::process::Command::new(iter.next().unwrap())
                                    .args(iter)
                                    .spawn()
                                    .unwrap()
                                    .wait()
                                    .unwrap();
                            });
                        }

                        ui.add_enabled_ui(self.state.config.drg_pak_path.is_some(), |ui| {
                            let mut button = ui.button("Install mods");
                            if self.state.config.drg_pak_path.is_none() {
                                button = button.on_disabled_hover_text(
                                    "DRG install not found. Configure it in the settings menu.",
                                );
                            }

                            if button.clicked() {
                                let mut mod_configs = Vec::new();
                                let mut mods = Vec::new();
                                let active_profile = self.state.mod_data.active_profile.clone();
                                self.state
                                    .mod_data
                                    .for_each_enabled_mod(&active_profile, |mc| {
                                        mod_configs.push(mc.clone());
                                    });

                                mod_configs.sort_by_key(|k| -k.priority);

                                for config in mod_configs {
                                    mods.push(config.spec.clone());
                                }

                                self.last_action = None;
                                self.integrate_rid = Some(message::Integrate::send(
                                    &mut self.request_counter,
                                    self.state.store.clone(),
                                    mods,
                                    self.state.config.drg_pak_path.as_ref().unwrap().clone(),
                                    self.state.config.deref().into(),
                                    self.tx.clone(),
                                    ctx.clone(),
                                ));
                                self.problematic_mod_id = None;
                            }
                        });

                        ui.add_enabled_ui(self.state.config.drg_pak_path.is_some(), |ui| {
                            let mut button = ui.button("Uninstall mods");
                            if self.state.config.drg_pak_path.is_none() {
                                button = button.on_disabled_hover_text(
                                    "DRG install not found. Configure it in the settings menu.",
                                );
                            }
                            if button.clicked() {
                                self.last_action = None;
                                if let Some(pak_path) = &self.state.config.drg_pak_path {
                                    let mut mods = HashSet::default();
                                    let active_profile = self.state.mod_data.active_profile.clone();
                                    self.state.mod_data.for_each_enabled_mod(
                                        &active_profile,
                                        |mc| {
                                            if let Some(modio_id) = self
                                                .state
                                                .store
                                                .get_mod_info(&mc.spec)
                                                .and_then(|i| i.modio_id)
                                            {
                                                mods.insert(modio_id);
                                            }
                                        },
                                    );

                                    debug!("uninstalling mods: pak_path = {}", pak_path.display());
                                    self.last_action = Some(match uninstall(pak_path, mods) {
                                        Ok(()) => LastAction::success(
                                            "Successfully uninstalled mods".to_string(),
                                        ),
                                        Err(e) => LastAction::failure(format!(
                                            "Failed to uninstall mods: {e}"
                                        )),
                                    })
                                }
                            }
                        });

                        if ui
                            .button("Update cache")
                            .on_hover_text(
                                "Checks for updates for all mods and updates local cache",
                            )
                            .clicked()
                        {
                            message::UpdateCache::send(self);
                            self.problematic_mod_id = None;
                        }
                    },
                );
                if self.integrate_rid.is_some() {
                    if ui.button("Cancel").clicked() {
                        self.integrate_rid.take().unwrap().handle.abort();
                    }
                    ui.spinner();
                }
                if self.update_rid.is_some() {
                    if ui.button("Cancel").clicked() {
                        self.update_rid.take().unwrap().handle.abort();
                    }
                    ui.spinner();
                }
                if ui
                    .button("Lint mods")
                    .on_hover_text("Lint mods in the current profile")
                    .clicked()
                {
                    self.lints_toggle_window = Some(WindowLintsToggle);
                }
                if ui.button("⚙").on_hover_text("Open settings").clicked() {
                    self.settings_window = Some(WindowSettings::new(&self.state));
                }
                if ui
                    .button("\u{2139}")
                    .on_hover_text("About the app")
                    .clicked()
                {
                    self.about_window = Some(WindowAbout);
                }
                ui.with_layout(egui::Layout::left_to_right(Align::TOP), |ui| {
                    if let Some(last_action) = &self.last_action {
                        let msg = match &last_action.status {
                            LastActionStatus::Success(msg) => {
                                ui.label(
                                    egui::RichText::new("STATUS")
                                        .color(Color32::BLACK)
                                        .background_color(Color32::LIGHT_GREEN),
                                );
                                msg
                            }
                            LastActionStatus::Failure(msg) => {
                                ui.label(
                                    egui::RichText::new("STATUS")
                                        .color(Color32::BLACK)
                                        .background_color(Color32::LIGHT_RED),
                                );
                                msg
                            }
                        };
                        ui.ctx().request_repaint(); // for continuously updating time
                        ui.label(format!("({}): {}", last_action.timeago(), msg));
                    }
                });
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if self.integrate_rid.is_some() || self.update_rid.is_some() || self.lint_rid.is_some()
            {
                ui.disable();
            }
            // profile selection

            let buttons = |ui: &mut Ui, mod_data: &mut ModData| {
                if ui
                    .button("📋")
                    .on_hover_text_at_pointer("Copy profile mods")
                    .clicked()
                {
                    let mut mods = Vec::new();
                    let active_profile = mod_data.active_profile.clone();
                    mod_data.for_each_enabled_mod(&active_profile, |mc| {
                        mods.push(mc.clone());
                    });
                    let mods = Self::build_mod_string(&mods);
                    ui.ctx().copy_text(mods);
                }

                // TODO find better icon, flesh out multiple-view usage, fix GUI locking
                /*
                if ui
                    .button("pop out")
                    .on_hover_text_at_pointer("pop out")
                    .clicked()
                {
                    self.open_profiles.insert(mod_data.active_profile.clone());
                }
                */
            };

            if named_combobox::ui(
                ui,
                "profile",
                self.state.mod_data.deref_mut().deref_mut(),
                Some(buttons),
                self.state.config.confirm_deletion,
            ) {
                self.state.mod_data.save().unwrap();
            }

            ui.separator();

            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                if self.resolve_mod_rid.is_some() {
                    ui.spinner();
                }
                ui.with_layout(ui.layout().with_main_justify(true), |ui| {
                    // define multiline layouter to be able to show multiple lines in a single line widget
                    let font_id = FontSelection::default().resolve(ui.style());
                    let text_color = ui.visuals().widgets.inactive.text_color();
                    let mut multiline_layouter =
                        move |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                            let layout_job = LayoutJob::simple(
                                text.as_str().to_string(),
                                font_id.clone(),
                                text_color,
                                wrap_width,
                            );
                            ui.painter().layout_job(layout_job)
                        };

                    let resolve = ui.add_enabled(
                        self.resolve_mod_rid.is_none(),
                        egui::TextEdit::singleline(&mut self.resolve_mod)
                            .layouter(&mut multiline_layouter)
                            .hint_text("Add mod..."),
                    );
                    if is_committed(&resolve) {
                        message::ResolveMods::send(self, &ctx, self.parse_mods(), false);
                        self.problematic_mod_id = None;
                    }
                });
            });

            let profile = self.state.mod_data.active_profile.clone();

            ui.horizontal(|ui| {
                ui.label("Sort by: ");

                let (mut sort_category, mut is_ascending) = self
                    .get_sorting_config()
                    .map(|c| (Some(c.sort_category), c.is_ascending))
                    .unwrap_or_default();

                let mut clicked = ui.radio_value(&mut sort_category, None, "Manual").clicked();
                for category in SortBy::iter() {
                    let mut radio_label = category.as_str().to_owned();
                    if sort_category == Some(category) {
                        radio_label.push_str(if is_ascending { " ⏶" } else { " ⏷" });
                    }
                    let resp = ui.radio_value(&mut sort_category, Some(category), radio_label);
                    if resp.clicked() {
                        clicked = true;
                        if resp.changed() {
                            is_ascending = true;
                        } else {
                            is_ascending = !is_ascending;
                        }
                    };
                }
                if clicked {
                    self.update_sorting_config(sort_category, is_ascending);
                }

                ui.add_space(16.);
                // TODO: actually implement mod groups.
                let search_string = &mut self.search_string;
                let lower = search_string.to_lowercase();
                let any_matches = self.state.mod_data.any_mod(&profile, |mc, _| {
                    self.state
                        .store
                        .get_mod_info(&mc.spec)
                        .map(|i| i.name.to_lowercase().contains(&lower))
                        .unwrap_or(false)
                });

                let mut text_edit = egui::TextEdit::singleline(search_string).hint_text("Search");
                if !any_matches {
                    text_edit = text_edit.text_color(ui.visuals().error_fg_color);
                }
                let res = ui
                    .scope_builder(
                        egui::UiBuilder::new().layout(egui::Layout::bottom_up(Align::RIGHT)),
                        |ui| ui.add(text_edit),
                    )
                    .inner;
                if res.changed() {
                    self.scroll_to_match = true;
                }
                if res.lost_focus()
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)
                    })
                {
                    *search_string = String::new();
                    self.scroll_to_match = false;
                } else if self.focus_search {
                    res.request_focus();
                    self.focus_search = false;
                }
            });
            ui.add_space(4.);

            self.ui_profile(ui, &profile);

            // must access memory outside of input lock to prevent deadlock
            let is_anything_focused = ctx.memory(|m| m.focused().is_some());
            ctx.input(|i| {
                if !i.raw.dropped_files.is_empty()
                    && self.integrate_rid.is_none()
                    && self.update_rid.is_none()
                {
                    let mut mods = String::new();
                    for f in i.raw.dropped_files.iter() {
                        mods.push_str(&f.path().to_string_lossy());
                        mods.push('\n');
                    }

                    self.resolve_mod = mods.trim().to_string();
                    message::ResolveMods::send(self, &ctx, self.parse_mods(), false);
                    self.problematic_mod_id = None;
                }
                for e in &i.events {
                    match e {
                        egui::Event::Paste(s) => {
                            if self.integrate_rid.is_none()
                                && self.update_rid.is_none()
                                && self.lint_rid.is_none()
                                && !is_anything_focused
                            {
                                self.resolve_mod = s.trim().to_string();
                                message::ResolveMods::send(self, &ctx, self.parse_mods(), false);
                            }
                        }
                        egui::Event::Text(text) if !is_anything_focused => {
                            self.search_string = text.to_string();
                            self.scroll_to_match = true;
                            self.focus_search = true;
                        }
                        _ => {}
                    }
                }
            });
        });
    }
}

fn is_committed(res: &egui::Response) -> bool {
    res.lost_focus() && res.ctx.input(|i| i.key_pressed(egui::Key::Enter))
}

/// A custom popup which does not automatically close when clicked.
fn custom_popup_below_widget<R>(
    ui: &Ui,
    popup_id: egui::Id,
    widget_response: &egui::Response,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    if egui::Popup::is_id_open(ui.ctx(), popup_id) {
        let inner = egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .constrain(true)
            .fixed_pos(widget_response.rect.left_bottom())
            .pivot(Align2::LEFT_TOP)
            .show(ui.ctx(), |ui| {
                // Note: we use a separate clip-rect for this area, so the popup can be outside the parent.
                // See https://github.com/emilk/egui/issues/825
                let frame = egui::Frame::popup(ui.style());
                let frame_margin = frame.total_margin();
                frame
                    .show(ui, |ui| {
                        ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                            ui.set_width(widget_response.rect.width() - frame_margin.sum().x);
                            add_contents(ui)
                        })
                        .inner
                    })
                    .inner
            })
            .inner;

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            egui::Popup::close_id(ui.ctx(), popup_id);
        }
        Some(inner)
    } else {
        None
    }
}

#[derive(Debug)]
pub enum SpecFetchProgress {
    Progress { progress: u64, size: u64 },
    Complete,
}

impl From<FetchProgress> for SpecFetchProgress {
    fn from(value: FetchProgress) -> Self {
        match value {
            FetchProgress::Progress { progress, size, .. } => Self::Progress { progress, size },
            FetchProgress::Complete { .. } => Self::Complete,
        }
    }
}
