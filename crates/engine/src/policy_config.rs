//! `IPolicyConfig`, declared by hand.
//!
//! Windows has no public API for changing the default audio device. Every
//! switcher tool uses this undocumented COM interface, which has stayed stable
//! across many Windows versions.
//!
//! Declared here against `windows` 0.62 because the `com-policy-config` crate,
//! whose source is the reference for the layout and GUIDs, depends on
//! `windows` 0.61, whose types do not mix with ours. The vtable is matched by
//! position, so the method order below must not change. Only
//! `SetDefaultEndpoint` is called; the others are declared to keep it in its
//! slot, with their pointer arguments left untyped.

#![allow(non_snake_case, reason = "COM method names as Windows declares them")]

use std::ffi::c_void;

use anyhow::{Context as _, Result};
use windows::Win32::Media::Audio::ERole;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows_core::{BOOL, GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

/// `CPolicyConfigClient`, the class that implements `IPolicyConfig`.
const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// The Windows 7 and later `IPolicyConfig`.
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
pub unsafe trait IPolicyConfig: IUnknown {
    fn GetMixFormat(&self, device_id: PCWSTR, format: *mut *mut c_void) -> HRESULT;
    fn GetDeviceFormat(
        &self,
        device_id: PCWSTR,
        default: BOOL,
        format: *mut *mut c_void,
    ) -> HRESULT;
    fn ResetDeviceFormat(&self, device_id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(
        &self,
        device_id: PCWSTR,
        endpoint_format: *mut c_void,
        mix_format: *mut c_void,
    ) -> HRESULT;
    fn GetProcessingPeriod(
        &self,
        device_id: PCWSTR,
        default: BOOL,
        default_period: *mut i64,
        minimum_period: *mut i64,
    ) -> HRESULT;
    fn SetProcessingPeriod(&self, device_id: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, device_id: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn SetShareMode(&self, device_id: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn GetPropertyValue(
        &self,
        device_id: PCWSTR,
        fx_store: BOOL,
        key: *const c_void,
        value: *mut c_void,
    ) -> HRESULT;
    fn SetPropertyValue(
        &self,
        device_id: PCWSTR,
        fx_store: BOOL,
        key: *const c_void,
        value: *mut c_void,
    ) -> HRESULT;
    fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self, device_id: PCWSTR, visible: BOOL) -> HRESULT;
}

/// Makes `device_id` the default endpoint for `role`.
///
/// COM must be initialised on the calling thread, in either apartment.
pub fn set_default_endpoint(device_id: &str, role: ERole) -> Result<()> {
    // SAFETY: plain COM activation; the class is registered on every Windows
    // since 7, and the interface layout above matches the one it implements.
    let policy: IPolicyConfig =
        unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
            .context("creating the PolicyConfig client")?;
    let id = HSTRING::from(device_id);
    // SAFETY: `id` outlives the call and is NUL-terminated.
    unsafe { policy.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role) }
        .ok()
        .with_context(|| format!("setting {device_id} as the default for {role:?}"))
}
