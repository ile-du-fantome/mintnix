use std::collections::BTreeSet;
use std::ops::DerefMut;
use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use tokio::{
    sync::mpsc::{self, Sender},
    task::JoinHandle,
};
use tracing::*;

use super::{
    App, SpecFetchProgress, WindowProviderParameters,
    request_counter::{RequestCounter, RequestID},
};
use crate::gui::LastAction;
use crate::integrate::*;
use crate::mod_lints::{LintId, LintReport};
use crate::state::{ModData_v0_2_0 as ModData, ModOrGroup};
use crate::*;
use crate::{
    providers::{FetchProgress, ModInfo, ModStore},
    state::ModConfig,
};
use mint_lib::mod_info::MetaConfig;

#[derive(Debug)]
pub struct MessageHandle<S> {
    pub rid: RequestID,
    pub handle: JoinHandle<()>,
    pub state: S,
}

#[derive(Debug)]
pub enum Message {
    ResolveMods(ResolveMods),
    Integrate(Integrate),
    FetchModProgress(FetchModProgress),
    UpdateCache(UpdateCache),
    FetchModDetails(Box<FetchModDetails>),
    LintMods(Box<LintMods>),
}

impl Message {
    pub fn handle(self, app: &mut App) {
        match self {
            Self::ResolveMods(msg) => msg.receive(app),
            Self::Integrate(msg) => msg.receive(app),
            Self::FetchModProgress(msg) => msg.receive(app),
            Self::UpdateCache(msg) => msg.receive(app),
            Self::FetchModDetails(msg) => msg.receive(app),
            Self::LintMods(msg) => msg.receive(app),
        }
    }
}

#[derive(Debug)]
pub struct ResolveMods {
    rid: RequestID,
    specs: Vec<ModSpecification>,
    result: Result<HashMap<ModSpecification, ModInfo>, ProviderError>,
    is_dependency: bool,
}

impl ResolveMods {
    pub fn send(
        app: &mut App,
        ctx: &egui::Context,
        specs: Vec<ModSpecification>,
        is_dependency: bool,
    ) {
        let rid = app.request_counter.next();
        let store = app.state.store.clone();
        let ctx = ctx.clone();
        let tx = app.tx.clone();
        let handle = tokio::spawn(async move {
            let result = store.resolve_mods(&specs, false).await;
            tx.send(Message::ResolveMods(Self {
                rid,
                specs,
                result,
                is_dependency,
            }))
            .await
            .unwrap();
            ctx.request_repaint();
        });
        app.last_action = None;
        app.resolve_mod_rid = Some(MessageHandle {
            rid,
            handle,
            state: (),
        });
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.resolve_mod_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(resolved_mods) => {
                    let primary_mods = self
                        .specs
                        .into_iter()
                        .collect::<HashSet<ModSpecification>>();
                    for (resolved_spec, info) in resolved_mods {
                        let is_dep = self.is_dependency || !primary_mods.contains(&resolved_spec);
                        let add = if is_dep {
                            // if mod is a dependency then check if there is a disabled
                            // mod that satisfies the dependency and enable it. if it
                            // is not a dependency then assume the user explicitly
                            // wants to add a specific mod version.
                            let active_profile = app.state.mod_data.active_profile.clone();
                            !app.state.mod_data.any_mod_mut(
                                &active_profile,
                                |mc, mod_group_enabled| {
                                    if mc.spec.satisfies_dependency(&resolved_spec) {
                                        mc.enabled = true;
                                        if let Some(mod_group_enabled) = mod_group_enabled {
                                            *mod_group_enabled = true;
                                        }
                                        true
                                    } else {
                                        false
                                    }
                                },
                            )
                        } else {
                            true
                        };

                        if add {
                            let ModData {
                                active_profile,
                                profiles,
                                ..
                            } = app.state.mod_data.deref_mut().deref_mut();

                            profiles.get_mut(active_profile).unwrap().mods.insert(
                                0,
                                ModOrGroup::Individual(ModConfig {
                                    spec: info.spec.clone(),
                                    required: info.suggested_require,
                                    enabled: true,
                                    priority: 0,
                                }),
                            );
                        }
                    }
                    app.resolve_mod.clear();
                    app.state.mod_data.save().unwrap();
                    app.last_action = Some(LastAction::success(
                        "mods successfully resolved".to_string(),
                    ));
                }
                Err(ProviderError::NoProvider { url: _, factory }) => {
                    app.window_provider_parameters =
                        Some(WindowProviderParameters::new(factory, &app.state));
                    app.last_action = Some(LastAction::failure("no provider".to_string()));
                }
                Err(e) => {
                    error!("{}", e);
                    app.problematic_mod_id = e.opt_mod_id();
                    app.last_action = Some(LastAction::failure(e.to_string()));
                }
            }
            app.resolve_mod_rid = None;
        }
    }
}

#[derive(Debug)]
pub struct Integrate {
    rid: RequestID,
    result: Result<(), IntegrationError>,
}

impl Integrate {
    pub fn send(
        rc: &mut RequestCounter,
        store: Arc<ModStore>,
        mods: Vec<ModSpecification>,
        fsd_pak: PathBuf,
        config: MetaConfig,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> MessageHandle<HashMap<ModSpecification, SpecFetchProgress>> {
        let rid = rc.next();
        MessageHandle {
            rid,
            handle: tokio::task::spawn(async move {
                let res =
                    integrate_async(store, ctx.clone(), mods, fsd_pak, config, rid, tx.clone())
                        .await;
                tx.send(Message::Integrate(Integrate { rid, result: res }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }),
            state: Default::default(),
        }
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.integrate_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(()) => {
                    info!("integration complete");
                    app.last_action = Some(LastAction::success("integration complete".to_string()));
                }
                Err(ref e)
                    if let IntegrationError::ProviderError { source } = e
                        && let ProviderError::NoProvider { url: _, factory } = source =>
                {
                    app.window_provider_parameters =
                        Some(WindowProviderParameters::new(factory, &app.state));
                    app.last_action = Some(LastAction::failure("no provider".to_string()));
                }
                Err(e) => {
                    error!("{}", e);
                    app.problematic_mod_id = e.opt_mod_id();
                    app.last_action = Some(LastAction::failure(e.to_string()));
                }
            }
            app.integrate_rid = None;
        }
    }
}

#[derive(Debug)]
pub struct FetchModProgress {
    rid: RequestID,
    spec: ModSpecification,
    progress: SpecFetchProgress,
}

impl FetchModProgress {
    fn receive(self, app: &mut App) {
        if let Some(MessageHandle { rid, state, .. }) = &mut app.integrate_rid
            && *rid == self.rid
        {
            state.insert(self.spec, self.progress);
        }
    }
}

#[derive(Debug)]
pub struct UpdateCache {
    rid: RequestID,
    result: Result<(), ProviderError>,
}

impl UpdateCache {
    pub fn send(app: &mut App) {
        let rid = app.request_counter.next();
        let tx = app.tx.clone();
        let store = app.state.store.clone();
        let handle = tokio::spawn(async move {
            let res = store.update_cache().await;
            tx.send(Message::UpdateCache(UpdateCache { rid, result: res }))
                .await
                .unwrap();
        });
        app.last_action = None;
        app.update_rid = Some(MessageHandle {
            rid,
            handle,
            state: (),
        });
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.update_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(()) => {
                    info!("cache update complete");
                    app.last_action = Some(LastAction::success(
                        "successfully updated cache".to_string(),
                    ));
                }
                Err(ProviderError::NoProvider { url: _, factory }) => {
                    app.window_provider_parameters =
                        Some(WindowProviderParameters::new(factory, &app.state));
                    app.last_action = Some(LastAction::failure("no provider".to_string()));
                }
                Err(e) => {
                    error!("{}", e);
                    app.problematic_mod_id = e.opt_mod_id();
                    app.last_action = Some(LastAction::failure(e.to_string()));
                }
            }
            app.update_rid = None;
        }
    }
}

async fn integrate_async(
    store: Arc<ModStore>,
    ctx: egui::Context,
    mod_specs: Vec<ModSpecification>,
    fsd_pak: PathBuf,
    config: MetaConfig,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<(), IntegrationError> {
    let update = false;

    let mods = store.resolve_mods(&mod_specs, update).await?;

    let to_integrate = mod_specs
        .iter()
        .map(|u| mods[u].clone())
        .collect::<Vec<_>>();
    let res_map: HashMap<ModResolution, ModSpecification> = mods
        .iter()
        .map(|(spec, info)| (info.resolution.clone(), spec.clone()))
        .collect();
    let urls = to_integrate
        .iter()
        .map(|m| &m.resolution)
        .collect::<Vec<_>>();

    let (tx, mut rx) = mpsc::channel::<FetchProgress>(10);

    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let Some(spec) = res_map.get(progress.resolution()) {
                message_tx
                    .send(Message::FetchModProgress(FetchModProgress {
                        rid,
                        spec: spec.clone(),
                        progress: progress.into(),
                    }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }
        }
    });

    let paths = store.fetch_mods_ordered(&urls, update, Some(tx)).await?;

    tokio::task::spawn_blocking(|| {
        crate::integrate::integrate(
            fsd_pak,
            config,
            to_integrate.into_iter().zip(paths).collect(),
        )
    })
    .await??;

    Ok(())
}

#[derive(Debug)]
pub struct LintMods {
    rid: RequestID,
    result: Result<LintReport, IntegrationError>,
}

impl LintMods {
    pub fn send(
        rc: &mut RequestCounter,
        store: Arc<ModStore>,
        mods: Vec<ModSpecification>,
        enabled_lints: BTreeSet<LintId>,
        game_pak_path: Option<PathBuf>,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> MessageHandle<()> {
        let rid = rc.next();

        let handle = tokio::task::spawn(async move {
            let paths_res =
                resolve_async_ordered(store, ctx.clone(), mods.clone(), rid, tx.clone()).await;
            let mod_path_pairs_res =
                paths_res.map(|paths| mods.into_iter().zip(paths).collect::<Vec<_>>());

            let report_res = match mod_path_pairs_res {
                Ok(pairs) => tokio::task::spawn_blocking(move || {
                    crate::mod_lints::run_lints(
                        &enabled_lints,
                        pairs.into_iter().collect(),
                        game_pak_path,
                    )
                })
                .await
                .unwrap()
                .map_err(Into::into),
                Err(e) => Err(e),
            };

            tx.send(Message::LintMods(
                LintMods {
                    rid,
                    result: report_res,
                }
                .into(),
            ))
            .await
            .unwrap();
            ctx.request_repaint();
        });

        MessageHandle {
            rid,
            handle,
            state: Default::default(),
        }
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.lint_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(report) => {
                    info!("lint mod report complete");
                    app.lint_report = Some(report);
                    app.last_action =
                        Some(LastAction::success("lint mod report complete".to_string()));
                }
                Err(ref e)
                    if let IntegrationError::ProviderError { source } = e
                        && let ProviderError::NoProvider { url: _, factory } = source =>
                {
                    app.window_provider_parameters =
                        Some(WindowProviderParameters::new(factory, &app.state));
                    app.last_action = Some(LastAction::failure("no provider".to_string()));
                }
                Err(e) => {
                    error!("{}", e);
                    app.problematic_mod_id = e.opt_mod_id();
                    app.last_action = Some(LastAction::failure(e.to_string()));
                }
            }
            app.integrate_rid = None;
        }
    }
}

async fn resolve_async_ordered(
    store: Arc<ModStore>,
    ctx: egui::Context,
    mod_specs: Vec<ModSpecification>,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<Vec<PathBuf>, IntegrationError> {
    let update = false;

    let mods = store.resolve_mods(&mod_specs, update).await?;

    let to_integrate = mod_specs
        .iter()
        .map(|u| mods[u].clone())
        .collect::<Vec<_>>();
    let res_map: HashMap<ModResolution, ModSpecification> = mods
        .iter()
        .map(|(spec, info)| (info.resolution.clone(), spec.clone()))
        .collect();
    let urls = to_integrate
        .iter()
        .map(|m| &m.resolution)
        .collect::<Vec<&ModResolution>>();

    let (tx, mut rx) = mpsc::channel::<FetchProgress>(10);

    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let Some(spec) = res_map.get(progress.resolution()) {
                message_tx
                    .send(Message::FetchModProgress(FetchModProgress {
                        rid,
                        spec: spec.clone(),
                        progress: progress.into(),
                    }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }
        }
    });

    Ok(store.fetch_mods_ordered(&urls, update, Some(tx)).await?)
}

#[derive(Debug)]
pub struct FetchModDetails {
    rid: RequestID,
    modio_id: u32,
    result: Result<ModDetails>,
}

#[derive(Debug)]
pub struct ModDetails {
    pub r#mod: modio::mods::Mod,
    pub versions: Vec<modio::files::File>,
    pub thumbnail: Vec<u8>,
}

impl FetchModDetails {
    pub fn send(
        rc: &mut RequestCounter,
        ctx: &egui::Context,
        tx: Sender<Message>,
        oauth_token: &str,
        modio_id: u32,
    ) -> MessageHandle<()> {
        let rid = rc.next();
        let ctx = ctx.clone();
        let oauth_token = oauth_token.to_string();

        MessageHandle {
            rid,
            handle: tokio::task::spawn(async move {
                let result = fetch_modio_mod_details(oauth_token, modio_id).await;
                tx.send(Message::FetchModDetails(Box::new(FetchModDetails {
                    rid,
                    result,
                    modio_id,
                })))
                .await
                .unwrap();
                ctx.request_repaint();
            }),
            state: (),
        }
    }

    fn receive(self, app: &mut App) {
        let mut to_remove = None;

        if let Some(req) = app.fetch_mod_details_rid.get(&self.modio_id)
            && req.rid == self.rid
        {
            match self.result {
                Ok(mod_details) => {
                    info!("fetch mod details successful");
                    app.mod_details.insert(mod_details.r#mod.id, mod_details);
                    app.last_action = Some(LastAction::success("Fetched mod details".to_string()));
                }
                Err(e) => {
                    error!("fetch mod details failed");
                    error!("{:#?}", e);
                    to_remove = Some(self.modio_id);
                    app.last_action = Some(LastAction::failure(
                        "Failed to fetch mod details".to_string(),
                    ));
                }
            }
        }

        if let Some(id) = to_remove {
            app.fetch_mod_details_rid.remove(&id);
        }
    }
}

async fn fetch_modio_mod_details(oauth_token: String, modio_id: u32) -> Result<ModDetails> {
    use crate::providers::modio::{LoggingMiddleware, MODIO_DRG_ID};
    use modio::{Credentials, Modio, filter::prelude::*};

    let credentials = Credentials::with_token("", oauth_token);
    let client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
        .with::<LoggingMiddleware>(Default::default())
        .build();
    let modio = Modio::new(credentials, client.clone())?;

    let r#mod = modio
        .game(MODIO_DRG_ID)
        .mods()
        .search(Id::eq(modio_id))
        .first()
        .await?
        .ok_or_else(|| anyhow::anyhow!("mod {modio_id} not found"))?;

    let mod_ref = modio.mod_(MODIO_DRG_ID, modio_id);
    let filter = with_limit(10).order_by(modio::user::filters::files::Version::desc());
    let versions = mod_ref.clone().files().search(filter).first_page().await?;

    let thumbnail = client
        .get(r#mod.logo.thumb_320x180.clone())
        .send()
        .await?
        .bytes()
        .await?
        .to_vec();

    Ok(ModDetails {
        r#mod,
        versions,
        thumbnail,
    })
}
