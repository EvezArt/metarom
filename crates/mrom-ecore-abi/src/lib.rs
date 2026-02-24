//! mrom-ecore-abi — stable C-compatible ABI for MetaROM emulator cores
//!
//! An .mrom arcade cart exposes one EcoreVtable that the MetaROM host runtime
//! discovers via `mrom_ecore_init()`. All callbacks are extern "C" and safe to
//! call across shared-library boundaries.

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::os::raw::c_uchar;

// ── Version sentinel ─────────────────────────────────────────────────────────

pub const MROM_ABI_VERSION: u32 = 1;

// ── Core info block (returned by ecore_info) ──────────────────────────────────

#[repr(C)]
pub struct ECoreInfo {
    /// ABI version this core was compiled against
    pub abi_version: u32,
    /// Null-terminated UTF-8 core identifier (e.g. "gb_dmg")
    pub core_id: *const c_char,
    /// Human-readable name
    pub label: *const c_char,
    /// Supported ROM MIME types, null-terminated array of null-terminated strings
    pub mime_types: *const *const c_char,
    /// Supported save-state API version
    pub save_state_version: u32,
}

// ── Audio / video frame descriptors ──────────────────────────────────────────

#[repr(C)]
pub struct VideoFrame {
    pub data: *const c_uchar,
    pub width: c_uint,
    pub height: c_uint,
    /// Bytes per row
    pub pitch: c_uint,
    /// FOURCC pixel format tag (e.g. 0x32424752 = "RGB2")
    pub pixel_format: u32,
}

#[repr(C)]
pub struct AudioFrame {
    /// Interleaved stereo PCM-16
    pub samples: *const i16,
    pub sample_count: c_uint,
    pub sample_rate_hz: c_uint,
}

// ── Virtual table ─────────────────────────────────────────────────────────────

/// Function pointer table exposed by each emulator core.
/// The host runtime discovers this via `mrom_ecore_init()`.
#[repr(C)]
pub struct EcoreVtable {
    /// Return static metadata; called once at load time
    pub ecore_info: unsafe extern "C" fn() -> *const ECoreInfo,

    /// Load a ROM image. Returns 0 on success, non-zero on error.
    pub load_rom: unsafe extern "C" fn(data: *const c_uchar, len: c_uint) -> c_int,

    /// Unload current ROM and free all core-side resources
    pub unload_rom: unsafe extern "C" fn(),

    /// Advance emulation by one video frame
    pub run_frame: unsafe extern "C" fn(
        video_out: *mut VideoFrame,
        audio_out: *mut AudioFrame,
    ),

    /// Serialize full machine state into caller-allocated buffer.
    /// Pass null/0 to query required size.
    /// Returns number of bytes written, or required size when buf is null.
    pub save_state: unsafe extern "C" fn(buf: *mut c_uchar, buf_len: c_uint) -> c_uint,

    /// Deserialize machine state from buffer. Returns 0 on success.
    pub load_state: unsafe extern "C" fn(buf: *const c_uchar, buf_len: c_uint) -> c_int,

    /// Write an input word for the given player index (0-based).
    /// Bit layout is core-defined; host should query ecore_info for input schema.
    pub set_input: unsafe extern "C" fn(player: c_uint, input_word: u32),

    /// Optional: host calls this to forward a JSON config blob.
    /// Core may ignore. Returns 0 on success, non-zero if config rejected.
    pub configure: unsafe extern "C" fn(json_cfg: *const c_char) -> c_int,

    /// Optional: return a null-terminated JSON string describing current core state.
    /// Caller must NOT free; pointer valid until next call.
    pub diagnostics: unsafe extern "C" fn() -> *const c_char,
}

// ── Host-side entrypoint symbol ───────────────────────────────────────────────

/// Symbol the host runtime looks for in each .mrom shared object.
/// Implementations return a pointer to a static EcoreVtable.
#[no_mangle]
pub type MromEcoreInitFn = unsafe extern "C" fn() -> *const EcoreVtable;

// ── Rust helper: safe wrapper over an EcoreVtable pointer ────────────────────

pub struct EcoreHandle {
    vtable: *const EcoreVtable,
    // Opaque handle to keep the dlopen ref alive (host manages lifecycle)
    _lib: *mut c_void,
}

unsafe impl Send for EcoreHandle {}
unsafe impl Sync for EcoreHandle {}

impl EcoreHandle {
    /// # Safety
    /// `vtable` must point to a valid, stable EcoreVtable for the lifetime of this handle.
    pub unsafe fn new(vtable: *const EcoreVtable, lib: *mut c_void) -> Self {
        Self { vtable, _lib: lib }
    }

    pub fn info(&self) -> *const ECoreInfo {
        unsafe { ((*self.vtable).ecore_info)() }
    }

    pub fn load_rom(&self, data: &[u8]) -> c_int {
        unsafe { ((*self.vtable).load_rom)(data.as_ptr(), data.len() as c_uint) }
    }

    pub fn unload_rom(&self) {
        unsafe { ((*self.vtable).unload_rom)() }
    }

    pub fn run_frame(&self, video: &mut VideoFrame, audio: &mut AudioFrame) {
        unsafe { ((*self.vtable).run_frame)(video, audio) }
    }
}
