#![allow(clippy::missing_transmute_annotations)]

mod server_list;

use std::{collections::HashMap, ffi::c_void, ptr::NonNull, sync::OnceLock};

use anyhow::{Result, anyhow};
use retour::GenericDetour;
use windows::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};

use crate::{
    LOG_GUARD, globals,
    ue::{self, FLinearColor, UObject},
};

type FnHookUFunctionBind = unsafe extern "system" fn(*mut ue::UFunction);
type FnWinMain = unsafe extern "system" fn(*mut (), *mut (), *mut (), i32, *const ()) -> i32;

static HOOK_UFUNCTION_BIND: OnceLock<GenericDetour<FnHookUFunctionBind>> = OnceLock::new();
static WIN_MAIN: OnceLock<GenericDetour<FnWinMain>> = OnceLock::new();
static KISMET_HOOKS: OnceLock<HashMap<String, ExecFn>> = OnceLock::new();

pub(super) unsafe fn install<T: retour::Function>(
    slot: &'static OnceLock<GenericDetour<T>>,
    target: T,
    detour: T,
) -> Result<()> {
    let hook = unsafe { GenericDetour::new(target, detour)? };
    slot.set(hook)
        .map_err(|_| anyhow!("detour already initialized"))?;
    unsafe { slot.get().unwrap().enable()? };
    Ok(())
}

type ExecFn = unsafe extern "system" fn(*mut ue::UObject, *mut ue::kismet::FFrame, *mut c_void);

pub unsafe fn initialize() -> Result<()> {
    let hooks = [
        (
            "/Game/_mint/BPL_MINT.BPL_MINT_C:Get Mod JSON",
            exec_get_mod_json as ExecFn,
        ),
        (
            "/Game/_mint/BPL_MINT.BPL_MINT_C:GetUpdateAvailable",
            exec_get_update_available as ExecFn,
        ),
        (
            "/Script/Engine.KismetSystemLibrary:PrintString",
            exec_print_string as ExecFn,
        ),
    ]
    .iter()
    .chain(server_list::kismet_hooks().iter())
    .cloned()
    .collect::<std::collections::HashMap<_, ExecFn>>();

    unsafe {
        install(
            &WIN_MAIN,
            std::mem::transmute(globals().resolution.core.as_ref().unwrap().main.0),
            detour_main as FnWinMain,
        )?;

        install(
            &HOOK_UFUNCTION_BIND,
            std::mem::transmute(globals().resolution.core.as_ref().unwrap().ufunction_bind.0),
            detour_ufunction_bind as FnHookUFunctionBind,
        )?;

        KISMET_HOOKS
            .set(
                hooks
                    .into_iter()
                    .map(|(name, hook)| (name.to_owned(), hook))
                    .collect(),
            )
            .map_err(|_| anyhow!("kismet hooks already initialized"))?;

        if let Ok(address) = &globals().resolution.disable {
            patch_mem(
                (address.0 as *mut u8).add(29),
                [0xB8, 0x01, 0x00, 0x00, 0x00],
            )?;
        }
    }
    Ok(())
}

unsafe extern "system" fn detour_ufunction_bind(function: *mut ue::UFunction) {
    unsafe {
        HOOK_UFUNCTION_BIND.get().unwrap().call(function);

        if let Some(function) = function.as_mut() {
            let path = function
                .ustruct
                .ufield
                .uobject
                .uobject_base_utility
                .uobject_base
                .get_path_name(None);
            if let Some(hook) = KISMET_HOOKS
                .get()
                .and_then(|hooks| hooks.get(path.as_str()))
            {
                function
                    .function_flags
                    .insert(ue::EFunctionFlags::FUNC_Native);
                function.func = *hook;
            }
        }
    }
}

unsafe fn patch_mem(address: *mut u8, patch: impl AsRef<[u8]>) -> Result<()> {
    unsafe {
        let patch = patch.as_ref();
        let patch_mem = std::slice::from_raw_parts_mut(address, patch.len());

        let mut old = Default::default();
        VirtualProtect(
            patch_mem.as_ptr() as *const c_void,
            patch_mem.len(),
            PAGE_EXECUTE_READWRITE,
            &mut old,
        )?;

        patch_mem.copy_from_slice(patch);

        VirtualProtect(
            patch_mem.as_ptr() as *const c_void,
            patch_mem.len(),
            old,
            &mut old,
        )?;
    }

    Ok(())
}

unsafe extern "system" fn detour_main(
    h_instance: *mut (),
    h_prev_instance: *mut (),
    lp_cmd_line: *mut (),
    n_cmd_show: i32,
    cmd_line: *const (),
) -> i32 {
    let ret = unsafe {
        WIN_MAIN.get().unwrap().call(
            h_instance,
            h_prev_instance,
            lp_cmd_line,
            n_cmd_show,
            cmd_line,
        )
    };

    // about to exit, drop log guard
    drop(LOG_GUARD.with_borrow_mut(|g| g.take()).unwrap());

    ret
}

unsafe extern "system" fn exec_get_mod_json(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    unsafe {
        let stack = stack.as_mut().unwrap();

        let _ctx: Option<&ue::UObject> = stack.arg();

        stack.most_recent_property_address = std::ptr::null();
        let ret: Option<ue::FString> = stack.arg();
        let ret_address = (stack.most_recent_property_address as *mut ue::FString)
            .as_mut()
            .unwrap();

        let json = serde_json::to_string(&globals().meta).unwrap();

        ret_address.clear();
        ret_address.extend_from_slice(&json.encode_utf16().chain([0]).collect::<Vec<_>>());

        std::mem::forget(ret);

        if !stack.code.is_null() {
            stack.code = stack.code.add(1);
        }
    }
}

unsafe extern "system" fn exec_get_update_available(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    unsafe {
        let stack = stack.as_mut().unwrap();

        let _ctx: Option<&ue::UObject> = stack.arg();

        stack.arg::<bool>();
        let done_checking = &mut *(stack.most_recent_property_address as *mut bool);
        *done_checking = true;

        stack.arg::<bool>();
        let available = &mut *(stack.most_recent_property_address as *mut bool);
        *available = false;

        std::mem::forget(stack.arg::<ue::FString>());
        let version = &mut *(stack.most_recent_property_address as *mut ue::FString);
        *version = ue::FString::new();

        if !stack.code.is_null() {
            stack.code = stack.code.add(1);
        }
    }
}

unsafe extern "system" fn exec_print_string(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    unsafe {
        let stack = stack.as_mut().unwrap();

        let _ctx: Option<NonNull<UObject>> = stack.arg();
        let string: ue::FString = stack.arg();
        let _print_to_screen: bool = stack.arg();
        let _print_to_log: bool = stack.arg();
        let _color: FLinearColor = stack.arg();
        let _duration: f32 = stack.arg();

        println!("PrintString({string})");

        if !stack.code.is_null() {
            stack.code = stack.code.add(1);
        }
    }
}
