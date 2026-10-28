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
        pub core: CoreResolution,
    }
}
