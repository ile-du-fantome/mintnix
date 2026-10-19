use patternsleuth::resolvers::futures::future::join_all;
use patternsleuth::resolvers::unreal::blueprint_library::UFunctionBind;
use patternsleuth::resolvers::unreal::fname::{FNameCtorWchar, FNameToString};
use patternsleuth::resolvers::unreal::game_loop::Main;
use patternsleuth::resolvers::unreal::gmalloc::GMalloc;
use patternsleuth::resolvers::unreal::kismet::{FFrameStep, FFrameStepExplicitProperty};
use patternsleuth::resolvers::unreal::*;
use patternsleuth::resolvers::*;
use patternsleuth::scanner::Pattern;

#[cfg(feature = "serde-resolvers")]
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "serde-resolvers", derive(Serialize, Deserialize))]
pub struct FOnlineSessionSettingsSetFString(pub u64);
impl_resolver_singleton!(collect, FOnlineSessionSettingsSetFString);
impl_resolver_singleton!(PEImage, FOnlineSessionSettingsSetFString, |ctx| async {
    let patterns = ["48 89 5C 24 ?? 48 89 54 24 ?? 55 56 57 48 83 EC 40 49 8B F8 48 8D 69"];
    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;
    Ok(Self(ensure_one(res.into_iter().flatten())?))
});

#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "serde-resolvers", derive(Serialize, Deserialize))]
pub struct USessionHandlingFSDFillSessionSetttingInner(pub u64);
impl_resolver_singleton!(collect, USessionHandlingFSDFillSessionSetttingInner);
impl_resolver_singleton!(
    PEImage,
    USessionHandlingFSDFillSessionSetttingInner,
    |ctx| async {
        let patterns = [
            "48 89 5C 24 ?? 48 89 4C 24 ?? 55 56 57 41 54 41 55 41 56 41 57 48 8B EC 48 81 EC 80 00 00 00 4C 8B FA",
            "48 89 5C 24 ?? 4C 89 4C 24 ?? 48 89 4C 24 ?? 55 56 57 41 54 41 55 41 56 41 57 48 8B EC 48 83 EC 70",
        ];
        let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;
        Ok(Self(ensure_one(res.into_iter().flatten())?))
    }
);

#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "serde-resolvers", derive(Serialize, Deserialize))]
pub struct Disable(pub u64);
impl_resolver_singleton!(collect, Disable);
impl_resolver_singleton!(PEImage, Disable, |ctx| async {
    let patterns = [
        "4C 8B B4 24 48 01 00 00 0F 84",
        "4C 8B B4 24 D8 01 00 00 0F 84",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(Self(ensure_one(res.into_iter().flatten())?))
});

impl_try_collector! {
    #[derive(Debug, PartialEq)]
    #[cfg_attr(feature = "serde-resolvers", derive(Serialize, Deserialize))]
    pub struct ServerModsResolution {
        pub set_fstring: FOnlineSessionSettingsSetFString,
        pub fill_session_setting: USessionHandlingFSDFillSessionSetttingInner,
    }
}

impl_try_collector! {
    #[derive(Debug, PartialEq)]
    #[cfg_attr(feature = "serde-resolvers", derive(Serialize, Deserialize))]
    pub struct CoreResolution {
        pub gmalloc: GMalloc,
        pub main: Main,
        pub fnametostring: FNameToString,
        pub fname_ctor_wchar: FNameCtorWchar,
        pub uobject_base_utility_get_path_name: UObjectBaseUtilityGetPathName,
        pub ufunction_bind: UFunctionBind,
        pub fframe_step: FFrameStep,
        pub fframe_step_explicit_property: FFrameStepExplicitProperty,
    }
}

impl_collector! {
    #[derive(Debug, PartialEq)]
    #[cfg_attr(feature = "serde-resolvers", derive(Serialize, Deserialize))]
    pub struct HookResolution {
        pub disable: Disable,
        pub server_mods: ServerModsResolution,
        pub core: CoreResolution,
    }
}
