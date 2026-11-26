//! Windows `WM_POINTER*` hook extracting high-frequency pen pressure, tilt, rotation,
//! barrel button, and inverted eraser state alongside winit.

#![allow(unsafe_code, clippy::upper_case_acronyms)]

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::Sample;
use crate::win_ink::{
    PenState, RawPenPacket, WM_POINTERCAPTURECHANGED, WM_POINTERDOWN, WM_POINTERENTER,
    WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE, dib_to_bmp_bytes,
    is_pointer_promoted_mouse_extra_info, is_supported_image_extension,
    should_clear_stale_pen_on_mouse, trim_png_trailing_bytes,
};

const MAX_HISTORY: usize = 64;
const MAX_QUEUED_SAMPLES: usize = 2048;
const NO_PEN: u32 = u32::MAX;
const PM_REMOVE: usize = 1;
const WM_KILLFOCUS: u32 = 0x0008;
const WM_KEYDOWN: u32 = 0x0100;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_POINTERDEVICECHANGE: u32 = 0x0238;
const WM_POINTERDEVICEINRANGE: u32 = 0x0239;

const POINTER_DEVICE_PRODUCT_STRING_MAX: usize = 520;
const POINTER_DEVICE_TYPE_INTEGRATED_PEN: u32 = 0x0000_0001;
const POINTER_DEVICE_TYPE_EXTERNAL_PEN: u32 = 0x0000_0002;
const SM_DIGITIZER: i32 = 94;
const NID_INTEGRATED_PEN: i32 = 0x0000_0004;
const NID_EXTERNAL_PEN: i32 = 0x0000_0008;
const NID_READY: i32 = 0x0000_0080;
const MAX_POINTER_DEVICES: usize = 64;

const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_INSERT: usize = 0x2D;
const VK_V: usize = 0x56;

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIBV5: u32 = 17;
const GMEM_MOVEABLE: u32 = 0x0002;
const MAX_CLIPBOARD_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 2 * 1024 * 1024;

static BARREL_DOWN: AtomicBool = AtomicBool::new(false);
static SECONDARY_BARREL_DOWN: AtomicBool = AtomicBool::new(false);
static BARREL_PRESS_COUNT: AtomicU32 = AtomicU32::new(0);
static SECONDARY_BARREL_PRESS_COUNT: AtomicU32 = AtomicU32::new(0);
static DEVICES_DIRTY: AtomicBool = AtomicBool::new(true);
static PASTE_REQUESTED: AtomicBool = AtomicBool::new(false);
static PEN_ID: AtomicU32 = AtomicU32::new(NO_PEN);
static PEN_END: AtomicU8 = AtomicU8::new(0);
static CONTACT_END: AtomicU8 = AtomicU8::new(0);
static LAST_PEN_TIME_MS: AtomicU32 = AtomicU32::new(0);
static HAS_PEN_TIME: AtomicBool = AtomicBool::new(false);

static SAMPLE_STATE: Mutex<SharedSamples> = Mutex::new(SharedSamples {
    latest: None,
    queue: VecDeque::new(),
});

struct SharedSamples {
    latest: Option<Sample>,
    queue: VecDeque<Sample>,
}

fn encode_end(end: Option<bool>) -> u8 {
    match end {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

fn decode_end(v: u8) -> Option<bool> {
    match v {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

#[repr(C)]
struct CWPSTRUCT {
    l_param: isize,
    w_param: usize,
    message: u32,
    hwnd: *mut std::ffi::c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: *mut std::ffi::c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct POINTER_INFO {
    pointer_type: u32,
    pointer_id: u32,
    frame_id: u32,
    pointer_flags: u32,
    source_device: *mut std::ffi::c_void,
    hwnd_target: *mut std::ffi::c_void,
    pt_pixel_location: POINT,
    pt_himetric_location: POINT,
    pt_pixel_location_raw: POINT,
    pt_himetric_location_raw: POINT,
    dw_time: u32,
    history_count: u32,
    input_data: i32,
    dw_key_states: u32,
    performance_count: u64,
    button_change_type: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct POINTER_PEN_INFO {
    pointer_info: POINTER_INFO,
    pen_flags: u32,
    pen_mask: u32,
    pressure: u32,
    rotation: u32,
    tilt_x: i32,
    tilt_y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct POINTER_DEVICE_INFO {
    display_orientation: u32,
    device: *mut std::ffi::c_void,
    pointer_device_type: u32,
    monitor: *mut std::ffi::c_void,
    starting_cursor_id: u32,
    max_active_contacts: u16,
    product_string: [u16; POINTER_DEVICE_PRODUCT_STRING_MAX],
}

#[repr(C)]
struct OPENFILENAMEW {
    l_struct_size: u32,
    hwnd_owner: *mut std::ffi::c_void,
    h_instance: *mut std::ffi::c_void,
    lpstr_filter: *const u16,
    lpstr_custom_filter: *mut u16,
    n_max_cust_filter: u32,
    n_filter_index: u32,
    lpstr_file: *mut u16,
    n_max_file: u32,
    lpstr_file_title: *mut u16,
    n_max_file_title: u32,
    lpstr_initial_dir: *const u16,
    lpstr_title: *const u16,
    flags: u32,
    n_file_offset: u16,
    n_file_extension: u16,
    lpstr_def_ext: *const u16,
    l_cust_data: isize,
    lpfn_hook: *const std::ffi::c_void,
    lp_template_name: *const u16,
    pv_reserved: *mut std::ffi::c_void,
    dw_reserved: u32,
    flags_ex: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowsHookExW(
        id_hook: i32,
        lpfn: unsafe extern "system" fn(i32, usize, isize) -> isize,
        h_mod: *mut std::ffi::c_void,
        dw_thread_id: u32,
    ) -> *mut std::ffi::c_void;
    fn UnhookWindowsHookEx(hhk: *mut std::ffi::c_void) -> i32;
    fn CallNextHookEx(
        hhk: *mut std::ffi::c_void,
        n_code: i32,
        w_param: usize,
        l_param: isize,
    ) -> isize;
    fn GetCurrentThreadId() -> u32;
    fn GetActiveWindow() -> *mut std::ffi::c_void;
    fn GetMessageExtraInfo() -> isize;
    fn GetMessageTime() -> i32;
    fn GetKeyState(n_virt_key: i32) -> i16;
    fn OpenClipboard(h_wnd_new_owner: *mut std::ffi::c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(u_format: u32) -> *mut std::ffi::c_void;
    fn SetClipboardData(u_format: u32, h_mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn RegisterClipboardFormatW(lpsz_format: *const u16) -> u32;
    fn GetPointerPenInfo(pointer_id: u32, pen_info: *mut POINTER_PEN_INFO) -> i32;
    fn GetPointerPenInfoHistory(
        pointer_id: u32,
        entries_count: *mut u32,
        pen_info: *mut POINTER_PEN_INFO,
    ) -> i32;
    fn GetPointerDevices(device_count: *mut u32, pointer_devices: *mut POINTER_DEVICE_INFO) -> i32;
    fn GetSystemMetrics(n_index: i32) -> i32;
    fn IsWindow(h_wnd: *mut std::ffi::c_void) -> i32;
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: *mut std::ffi::c_void,
        dw_attribute: u32,
        pv_attribute: *const std::ffi::c_void,
        cb_attribute: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(u_flags: u32, dw_bytes: usize) -> *mut std::ffi::c_void;
    fn GlobalFree(h_mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn GlobalLock(h_mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn GlobalUnlock(h_mem: *mut std::ffi::c_void) -> i32;
    fn GlobalSize(h_mem: *mut std::ffi::c_void) -> usize;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn DragQueryFileW(
        h_drop: *mut std::ffi::c_void,
        i_file: u32,
        lpsz_file: *mut u16,
        cch: u32,
    ) -> u32;
    fn SHCreateItemFromParsingName(
        psz_path: *const u16,
        pbc: *mut std::ffi::c_void,
        riid: *const ComGuid,
        ppv: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn SHGetKnownFolderPath(
        rfid: *const ComGuid,
        dw_flags: u32,
        h_token: *mut std::ffi::c_void,
        ppsz_path: *mut *mut u16,
    ) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(pv_reserved: *const std::ffi::c_void, dw_co_init: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const ComGuid,
        p_unk_outer: *mut std::ffi::c_void,
        dw_cls_context: u32,
        riid: *const ComGuid,
        ppv: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn CoTaskMemFree(pv: *const std::ffi::c_void);
}

#[link(name = "advapi32")]
unsafe extern "system" {
    #[allow(clippy::too_many_arguments)]
    fn RegCreateKeyExW(
        h_key: *mut std::ffi::c_void,
        lp_sub_key: *const u16,
        reserved: u32,
        lp_class: *mut u16,
        dw_options: u32,
        sam_desired: u32,
        lp_security_attributes: *const std::ffi::c_void,
        phk_result: *mut *mut std::ffi::c_void,
        lpdw_disposition: *mut u32,
    ) -> i32;
    fn RegOpenKeyExW(
        h_key: *mut std::ffi::c_void,
        lp_sub_key: *const u16,
        ul_options: u32,
        sam_desired: u32,
        phk_result: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn RegSetValueExW(
        h_key: *mut std::ffi::c_void,
        lp_value_name: *const u16,
        reserved: u32,
        dw_type: u32,
        lp_data: *const u8,
        cb_data: u32,
    ) -> i32;
    fn RegQueryValueExW(
        h_key: *mut std::ffi::c_void,
        lp_value_name: *const u16,
        lp_reserved: *mut u32,
        lp_type: *mut u32,
        lp_data: *mut u8,
        lpcb_data: *mut u32,
    ) -> i32;
    fn RegDeleteValueW(h_key: *mut std::ffi::c_void, lp_value_name: *const u16) -> i32;
    fn RegCloseKey(h_key: *mut std::ffi::c_void) -> i32;
    #[cfg(test)]
    fn RegDeleteKeyW(h_key: *mut std::ffi::c_void, lp_sub_key: *const u16) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ComGuid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl ComGuid {
    const fn from_u128(uuid: u128) -> Self {
        Self {
            data1: (uuid >> 96) as u32,
            data2: ((uuid >> 80) & 0xffff) as u16,
            data3: ((uuid >> 64) & 0xffff) as u16,
            data4: (uuid as u64).to_be_bytes(),
        }
    }
}

const CLSID_FILE_OPEN_DIALOG: ComGuid = ComGuid::from_u128(0xdc1c5a9c_e88a_4dde_a5a1_60f82a20aef7);
const IID_I_FILE_DIALOG: ComGuid = ComGuid::from_u128(0x42f85136_db7e_439c_85f1_e4075d135fc8);
const IID_I_SHELL_ITEM: ComGuid = ComGuid::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
const FOLDERID_DESKTOP: ComGuid = ComGuid::from_u128(0xb4bfcc3a_db2c_424c_b029_7fe99a87c641);
const CLSID_SHELL_LINK: ComGuid = ComGuid::from_u128(0x00021401_0000_0000_c000_000000000046);
const IID_I_SHELL_LINK_W: ComGuid = ComGuid::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_I_PERSIST_FILE: ComGuid = ComGuid::from_u128(0x0000010b_0000_0000_c000_000000000046);

const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const RPC_E_CHANGED_MODE: i32 = -2_147_417_850; // 0x80010106
const COINIT_APARTMENTTHREADED: u32 = 0x2;
const COINIT_DISABLE_OLE1DDE: u32 = 0x4;
const CLSCTX_INPROC_SERVER: u32 = 0x1;
const FOS_NOCHANGEDIR: u32 = 0x0000_0008;
const FOS_PICKFOLDERS: u32 = 0x0000_0020;
const FOS_FORCEFILESYSTEM: u32 = 0x0000_0040;
const SIGDN_FILESYSPATH: i32 = -2_147_123_200; // 0x80058000

const HKEY_CURRENT_USER: *mut std::ffi::c_void =
    std::ptr::without_provenance_mut((0x8000_0001_u32 as i32 as isize) as usize);
const ERROR_SUCCESS: i32 = 0;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const REG_OPTION_NON_VOLATILE: u32 = 0;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const MAX_REG_STRING_CHARS: usize = 4096;
const MAX_REG_STRING_BYTES: usize = 64 * 1024;

#[cfg(test)]
const IUNKNOWN_VTBL_LEN: usize = 3;
#[cfg(test)]
const IFILE_DIALOG_VTBL_LEN: usize = 27;
#[cfg(test)]
const ISHELL_ITEM_VTBL_LEN: usize = 8;
#[cfg(test)]
const ISHELL_LINK_VTBL_LEN: usize = 21;
#[cfg(test)]
const IPERSIST_FILE_VTBL_LEN: usize = 9;
const VTBL_IUNKNOWN_QUERY_INTERFACE: usize = 0;
const VTBL_IUNKNOWN_RELEASE: usize = 2;
const VTBL_IMODAL_WINDOW_SHOW: usize = 3;
const VTBL_IFILE_DIALOG_SET_OPTIONS: usize = 9;
const VTBL_IFILE_DIALOG_GET_OPTIONS: usize = 10;
const VTBL_IFILE_DIALOG_SET_FOLDER: usize = 12;
const VTBL_IFILE_DIALOG_SET_TITLE: usize = 17;
const VTBL_IFILE_DIALOG_GET_RESULT: usize = 20;
const VTBL_ISHELL_ITEM_GET_DISPLAY_NAME: usize = 5;
const VTBL_ISHELL_LINK_SET_DESCRIPTION: usize = 7;
const VTBL_ISHELL_LINK_SET_WORKING_DIR: usize = 9;
const VTBL_ISHELL_LINK_SET_ICON_LOCATION: usize = 17;
const VTBL_ISHELL_LINK_SET_PATH: usize = 20;
const VTBL_IPERSIST_FILE_SAVE: usize = 6;

#[link(name = "comdlg32")]
unsafe extern "system" {
    fn GetOpenFileNameW(lpofn: *mut OPENFILENAMEW) -> i32;
    fn GetSaveFileNameW(lpofn: *mut OPENFILENAMEW) -> i32;
}

const OFN_OVERWRITEPROMPT: u32 = 0x0000_0002;
const OFN_HIDEREADONLY: u32 = 0x0000_0004;
const OFN_NOCHANGEDIR: u32 = 0x0000_0008;
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_EXPLORER: u32 = 0x0008_0000;

fn str_to_wide_null(s: &str) -> Vec<u16> {
    let mut wide: Vec<u16> = s.encode_utf16().filter(|&c| c != 0).collect();
    wide.push(0);
    wide
}

fn os_str_to_wide_null(s: &OsStr) -> Vec<u16> {
    let mut wide: Vec<u16> = s.encode_wide().filter(|&c| c != 0).collect();
    wide.push(0);
    wide
}

fn build_multi_filter_wide(filter_label: &str, patterns: &[&str]) -> Vec<u16> {
    let joined = patterns.join(";");
    let desc = format!("{filter_label} ({joined})");
    let mut wide = Vec::new();
    wide.extend(desc.encode_utf16().filter(|&c| c != 0));
    wide.push(0);
    wide.extend(joined.encode_utf16().filter(|&c| c != 0));
    wide.push(0);
    wide.extend("All Files (*.*)".encode_utf16());
    wide.push(0);
    wide.extend("*.*".encode_utf16());
    wide.push(0);
    wide.push(0);
    wide
}

fn build_filter_wide(filter_label: &str, ext: &str) -> Vec<u16> {
    let clean_ext = ext.trim_start_matches('.');
    let pattern = format!("*.{clean_ext}");
    build_multi_filter_wide(filter_label, &[&pattern])
}

struct FileDialogSpec<'a> {
    save_mode: bool,
    title: &'a str,
    filter_wide: Vec<u16>,
    def_ext: &'a str,
    default_name: Option<&'a str>,
    initial_dir: Option<&'a Path>,
}

fn run_file_dialog(spec: FileDialogSpec<'_>) -> Option<PathBuf> {
    let mut file_buf = vec![0_u16; 4096];
    let max_name = file_buf.len().saturating_sub(1);
    if let Some(name) = spec.default_name {
        for (dst, src) in file_buf
            .iter_mut()
            .take(max_name)
            .zip(name.encode_utf16().filter(|&c| c != 0))
        {
            *dst = src;
        }
    }

    let filter_wide = spec.filter_wide;
    let title_wide = str_to_wide_null(spec.title);
    let clean_ext = spec.def_ext.trim_start_matches('.');
    let def_ext_wide = str_to_wide_null(clean_ext);
    let init_dir_wide = spec
        .initial_dir
        .filter(|p| p.exists())
        .map(|p| os_str_to_wide_null(p.as_os_str()));

    let flags = if spec.save_mode {
        OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY
    } else {
        OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY
    };

    // SAFETY: `GetActiveWindow` takes no pointers and returns the caller thread's active HWND or null.
    let owner = unsafe { GetActiveWindow() };
    let mut ofn = OPENFILENAMEW {
        l_struct_size: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwnd_owner: owner,
        h_instance: std::ptr::null_mut(),
        lpstr_filter: filter_wide.as_ptr(),
        lpstr_custom_filter: std::ptr::null_mut(),
        n_max_cust_filter: 0,
        n_filter_index: 1,
        lpstr_file: file_buf.as_mut_ptr(),
        n_max_file: file_buf.len() as u32,
        lpstr_file_title: std::ptr::null_mut(),
        n_max_file_title: 0,
        lpstr_initial_dir: init_dir_wide
            .as_ref()
            .map_or(std::ptr::null(), |v| v.as_ptr()),
        lpstr_title: title_wide.as_ptr(),
        flags,
        n_file_offset: 0,
        n_file_extension: 0,
        lpstr_def_ext: def_ext_wide.as_ptr(),
        l_cust_data: 0,
        lpfn_hook: std::ptr::null(),
        lp_template_name: std::ptr::null(),
        pv_reserved: std::ptr::null_mut(),
        dw_reserved: 0,
        flags_ex: 0,
    };

    // SAFETY: All pointers stored in `ofn` reference stack/heap buffers owned by this frame
    // (`file_buf`, `filter_wide`, `title_wide`, `def_ext_wide`, `init_dir_wide`) that remain alive
    // and unmoved for the entire duration of the modal `GetSaveFileNameW`/`GetOpenFileNameW` call.
    let ok = unsafe {
        if spec.save_mode {
            GetSaveFileNameW(&mut ofn)
        } else {
            GetOpenFileNameW(&mut ofn)
        }
    };
    if ok == 0 {
        return None;
    }

    let end = file_buf
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(file_buf.len());
    let chosen = String::from_utf16_lossy(&file_buf[..end]);
    (!chosen.trim().is_empty()).then(|| PathBuf::from(chosen))
}

/// Opens the native Windows Explorer "Save As" dialog and returns the selected path, if confirmed.
pub fn pick_save_file(
    title: &str,
    filter_label: &str,
    ext: &str,
    default_name: &str,
    initial_dir: Option<&Path>,
) -> Option<PathBuf> {
    run_file_dialog(FileDialogSpec {
        save_mode: true,
        title,
        filter_wide: build_filter_wide(filter_label, ext),
        def_ext: ext,
        default_name: Some(default_name),
        initial_dir,
    })
}

/// Opens the native Windows Explorer "Open File" dialog and returns the selected path, if confirmed.
pub fn pick_open_file(
    title: &str,
    filter_label: &str,
    ext: &str,
    initial_dir: Option<&Path>,
) -> Option<PathBuf> {
    pick_open_file_exts(title, filter_label, &[ext], initial_dir)
}

/// Opens the native Windows Explorer "Open File" dialog matching any extension in `exts`.
pub fn pick_open_file_exts(
    title: &str,
    filter_label: &str,
    exts: &[&str],
    initial_dir: Option<&Path>,
) -> Option<PathBuf> {
    let patterns: Vec<String> = exts
        .iter()
        .map(|ext| format!("*.{}", ext.trim_start_matches('.')))
        .collect();
    let pattern_refs: Vec<&str> = patterns.iter().map(String::as_str).collect();
    let filter_wide = if pattern_refs.is_empty() {
        build_filter_wide(filter_label, "*")
    } else {
        build_multi_filter_wide(filter_label, &pattern_refs)
    };
    let def_ext = exts.first().copied().unwrap_or("");
    run_file_dialog(FileDialogSpec {
        save_mode: false,
        title,
        filter_wide,
        def_ext,
        default_name: None,
        initial_dir,
    })
}

/// Opens the native Windows Explorer dialog filtered for supported raster image formats.
pub fn pick_open_image_file(title: &str, initial_dir: Option<&Path>) -> Option<PathBuf> {
    let filter_wide = build_multi_filter_wide(
        "Image Files",
        &[
            "*.png", "*.jpg", "*.jpeg", "*.jfif", "*.webp", "*.gif", "*.bmp", "*.ico", "*.tif",
            "*.tiff", "*.tga", "*.qoi",
        ],
    );
    run_file_dialog(FileDialogSpec {
        save_mode: false,
        title,
        filter_wide,
        def_ext: "png",
        default_name: None,
        initial_dir,
    })
}

struct ComGuard {
    needs_uninit: bool,
}

impl ComGuard {
    fn init() -> Option<Self> {
        // SAFETY: `CoInitializeEx` initializes COM for the calling thread with no borrowed pointers.
        let hr = unsafe {
            CoInitializeEx(
                std::ptr::null(),
                COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE,
            )
        };
        if hr == S_OK || hr == S_FALSE {
            Some(Self { needs_uninit: true })
        } else if hr == RPC_E_CHANGED_MODE {
            Some(Self {
                needs_uninit: false,
            })
        } else {
            None
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.needs_uninit {
            // SAFETY: Matches a successful `CoInitializeEx` call on the same thread.
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct ComObject {
    ptr: *mut std::ffi::c_void,
}

impl ComObject {
    /// # Safety
    /// `ptr` must be a non-null, owning COM interface pointer with at least `vtbl_slot` slots.
    unsafe fn vtable_slot(&self, slot: usize) -> usize {
        // SAFETY: Every COM object starts with a pointer to its interface vtable.
        unsafe {
            let vtbl = *(self.ptr as *const *const usize);
            *vtbl.add(slot)
        }
    }
}

impl Drop for ComObject {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: `self.ptr` is an owned COM pointer whose vtable begins with `IUnknown` (`Release` at slot 2).
            unsafe {
                let release_addr = self.vtable_slot(VTBL_IUNKNOWN_RELEASE);
                let release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(release_addr);
                let _ = release(self.ptr);
            }
        }
    }
}

struct CoTaskMemWideGuard(*mut u16);

impl Drop for CoTaskMemWideGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` was allocated by COM via `CoTaskMemAlloc` (`IShellItem::GetDisplayName`).
            unsafe {
                CoTaskMemFree(self.0.cast());
            }
        }
    }
}

fn create_file_open_dialog() -> Option<ComObject> {
    let mut raw: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `CLSID_FILE_OPEN_DIALOG` and `IID_I_FILE_DIALOG` are valid GUIDs and `raw` is a valid out-pointer.
    let hr = unsafe {
        CoCreateInstance(
            &CLSID_FILE_OPEN_DIALOG,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_I_FILE_DIALOG,
            &mut raw,
        )
    };
    (hr >= 0 && !raw.is_null()).then_some(ComObject { ptr: raw })
}

fn configure_folder_dialog(dialog: &ComObject, title: &str, initial_dir: Option<&Path>) -> bool {
    let mut opts: u32 = 0;
    // SAFETY: `dialog` is an initialized `IFileDialog`; slots 10 (`GetOptions`), 9 (`SetOptions`),
    // and 17 (`SetTitle`) match the `IFileDialog_Vtbl` ABI.
    unsafe {
        let get_options: unsafe extern "system" fn(*mut std::ffi::c_void, *mut u32) -> i32 =
            std::mem::transmute(dialog.vtable_slot(VTBL_IFILE_DIALOG_GET_OPTIONS));
        if get_options(dialog.ptr, &mut opts) < 0 {
            return false;
        }
        let set_options: unsafe extern "system" fn(*mut std::ffi::c_void, u32) -> i32 =
            std::mem::transmute(dialog.vtable_slot(VTBL_IFILE_DIALOG_SET_OPTIONS));
        if set_options(
            dialog.ptr,
            opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR,
        ) < 0
        {
            return false;
        }
        let title_wide = str_to_wide_null(title);
        let set_title: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16) -> i32 =
            std::mem::transmute(dialog.vtable_slot(VTBL_IFILE_DIALOG_SET_TITLE));
        let _ = set_title(dialog.ptr, title_wide.as_ptr());
    }

    if let Some(dir) = initial_dir.filter(|p| p.exists()) {
        let dir_wide = os_str_to_wide_null(dir.as_os_str());
        let mut item_raw: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `dir_wide` is NUL-terminated UTF-16 and `item_raw` receives an owned `IShellItem`.
        let hr = unsafe {
            SHCreateItemFromParsingName(
                dir_wide.as_ptr(),
                std::ptr::null_mut(),
                &IID_I_SHELL_ITEM,
                &mut item_raw,
            )
        };
        if hr >= 0 && !item_raw.is_null() {
            let folder_item = ComObject { ptr: item_raw };
            // SAFETY: `SetFolder` is at slot 12 on `IFileDialog` and borrows `folder_item.ptr` for this call.
            unsafe {
                let set_folder: unsafe extern "system" fn(
                    *mut std::ffi::c_void,
                    *mut std::ffi::c_void,
                ) -> i32 = std::mem::transmute(dialog.vtable_slot(VTBL_IFILE_DIALOG_SET_FOLDER));
                let _ = set_folder(dialog.ptr, folder_item.ptr);
            }
        }
    }
    true
}

fn read_co_task_mem_path(raw_str: *mut u16) -> Option<PathBuf> {
    let _free_guard = CoTaskMemWideGuard(raw_str);
    if raw_str.is_null() {
        return None;
    }

    let mut len = 0usize;
    while len < 32_768 {
        // SAFETY: `raw_str` is a valid NUL-terminated UTF-16 string allocated via `CoTaskMemAlloc`.
        let unit = unsafe { *raw_str.add(len) };
        if unit == 0 {
            break;
        }
        len += 1;
    }
    if len == 0 {
        return None;
    }
    // SAFETY: `len` code units preceding the NUL terminator belong to the COM-allocated string buffer.
    let slice = unsafe { std::slice::from_raw_parts(raw_str, len) };
    let chosen = String::from_utf16_lossy(slice);
    (!chosen.trim().is_empty()).then(|| PathBuf::from(chosen))
}

fn read_shell_item_path(item: &ComObject) -> Option<PathBuf> {
    let mut raw_str: *mut u16 = std::ptr::null_mut();
    // SAFETY: `item` is an `IShellItem`; `GetDisplayName` is at slot 5 and writes a `CoTaskMemAlloc` UTF-16 string.
    let hr = unsafe {
        let get_display_name: unsafe extern "system" fn(
            *mut std::ffi::c_void,
            i32,
            *mut *mut u16,
        ) -> i32 = std::mem::transmute(item.vtable_slot(VTBL_ISHELL_ITEM_GET_DISPLAY_NAME));
        get_display_name(item.ptr, SIGDN_FILESYSPATH, &mut raw_str)
    };
    if hr < 0 {
        let _free_guard = CoTaskMemWideGuard(raw_str);
        return None;
    }
    read_co_task_mem_path(raw_str)
}

/// Parameters for creating or refreshing a Windows `.lnk` desktop shortcut.
#[derive(Clone, Copy, Debug)]
pub struct WindowsShortcutSpec<'a> {
    pub shortcut_path: &'a Path,
    pub target_path: &'a Path,
    pub working_dir: Option<&'a Path>,
    pub description: &'a str,
    pub icon_path: Option<&'a Path>,
}

/// Resolves the user's current Windows Desktop directory (`FOLDERID_Desktop`), with `%USERPROFILE%\Desktop` fallback.
#[must_use]
pub fn desktop_dir() -> Option<PathBuf> {
    let _com = ComGuard::init();
    let mut raw_str: *mut u16 = std::ptr::null_mut();
    // SAFETY: `FOLDERID_DESKTOP` is a valid `KNOWNFOLDERID` and `raw_str` receives a `CoTaskMemAlloc` path.
    let hr =
        unsafe { SHGetKnownFolderPath(&FOLDERID_DESKTOP, 0, std::ptr::null_mut(), &mut raw_str) };
    if hr >= 0
        && let Some(path) = read_co_task_mem_path(raw_str)
    {
        return Some(path);
    } else {
        let _free_guard = CoTaskMemWideGuard(raw_str);
    }

    std::env::var_os("USERPROFILE")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join("Desktop"))
}

fn create_shell_link() -> Result<ComObject, String> {
    let mut raw: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `CLSID_SHELL_LINK` and `IID_I_SHELL_LINK_W` are valid COM GUIDs and `raw` receives an owning `IShellLinkW`.
    let hr = unsafe {
        CoCreateInstance(
            &CLSID_SHELL_LINK,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_I_SHELL_LINK_W,
            &mut raw,
        )
    };
    if hr < 0 || raw.is_null() {
        return Err(format!(
            "Could not create IShellLinkW COM object (HRESULT 0x{hr:08x})."
        ));
    }
    Ok(ComObject { ptr: raw })
}

fn set_shell_link_wide_str(
    link: &ComObject,
    slot: usize,
    value: &std::ffi::OsStr,
    label: &str,
) -> Result<(), String> {
    let wide = os_str_to_wide_null(value);
    // SAFETY: Caller supplies an initialized `IShellLinkW` and a single-`LPCWSTR` setter slot (`SetPath`, `SetWorkingDirectory`, or `SetDescription`).
    let hr = unsafe {
        let setter: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16) -> i32 =
            std::mem::transmute(link.vtable_slot(slot));
        setter(link.ptr, wide.as_ptr())
    };
    if hr < 0 {
        return Err(format!(
            "Failed setting shortcut {label} (HRESULT 0x{hr:08x})."
        ));
    }
    Ok(())
}

fn configure_shell_link(link: &ComObject, spec: WindowsShortcutSpec<'_>) -> Result<(), String> {
    set_shell_link_wide_str(
        link,
        VTBL_ISHELL_LINK_SET_PATH,
        spec.target_path.as_os_str(),
        "target",
    )?;

    if let Some(work_dir) = spec.working_dir.or_else(|| spec.target_path.parent())
        && !work_dir.as_os_str().is_empty()
    {
        set_shell_link_wide_str(
            link,
            VTBL_ISHELL_LINK_SET_WORKING_DIR,
            work_dir.as_os_str(),
            "working directory",
        )?;
    }

    if !spec.description.trim().is_empty() {
        set_shell_link_wide_str(
            link,
            VTBL_ISHELL_LINK_SET_DESCRIPTION,
            std::ffi::OsStr::new(spec.description),
            "description",
        )?;
    }

    if let Some(icon) = spec.icon_path {
        let icon_wide = os_str_to_wide_null(icon.as_os_str());
        // SAFETY: `SetIconLocation` at slot 17 on `IShellLinkW` takes `(LPCWSTR, int)`.
        let hr = unsafe {
            let set_icon: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, i32) -> i32 =
                std::mem::transmute(link.vtable_slot(VTBL_ISHELL_LINK_SET_ICON_LOCATION));
            set_icon(link.ptr, icon_wide.as_ptr(), 0)
        };
        if hr < 0 {
            return Err(format!(
                "Failed setting shortcut icon location (HRESULT 0x{hr:08x})."
            ));
        }
    }

    Ok(())
}

fn save_shell_link(link: &ComObject, shortcut_path: &Path) -> Result<(), String> {
    let mut persist_raw: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `QueryInterface` is slot 0 on `IUnknown` / `IShellLinkW`; `IID_I_PERSIST_FILE` writes an owned `IPersistFile`.
    let qi_hr = unsafe {
        let qi: unsafe extern "system" fn(
            *mut std::ffi::c_void,
            *const ComGuid,
            *mut *mut std::ffi::c_void,
        ) -> i32 = std::mem::transmute(link.vtable_slot(VTBL_IUNKNOWN_QUERY_INTERFACE));
        qi(link.ptr, &IID_I_PERSIST_FILE, &mut persist_raw)
    };
    if qi_hr < 0 || persist_raw.is_null() {
        return Err(format!(
            "Could not obtain IPersistFile from ShellLink (HRESULT 0x{qi_hr:08x})."
        ));
    }
    let persist = ComObject { ptr: persist_raw };

    let lnk_wide = os_str_to_wide_null(shortcut_path.as_os_str());
    // SAFETY: `IPersistFile::Save` is at slot 6 and borrows `lnk_wide` for the duration of the write.
    let save_hr = unsafe {
        let save: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, i32) -> i32 =
            std::mem::transmute(persist.vtable_slot(VTBL_IPERSIST_FILE_SAVE));
        save(persist.ptr, lnk_wide.as_ptr(), 1)
    };
    if save_hr < 0 {
        return Err(format!(
            "Could not save shortcut `{}` (HRESULT 0x{save_hr:08x}).",
            shortcut_path.display()
        ));
    }
    Ok(())
}

/// Creates or overwrites a Windows Explorer `.lnk` shortcut pointing to `spec.target_path`.
pub fn create_desktop_shortcut(spec: WindowsShortcutSpec<'_>) -> Result<(), String> {
    if spec.shortcut_path.as_os_str().is_empty() || spec.target_path.as_os_str().is_empty() {
        return Err("Shortcut path and target executable path must be non-empty.".to_string());
    }
    if let Some(parent) = spec
        .shortcut_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Could not prepare shortcut folder `{}`: {err}",
                parent.display()
            )
        })?;
    }

    let _com = ComGuard::init()
        .ok_or_else(|| "Could not initialize Windows COM for shortcut creation.".to_string())?;
    let link = create_shell_link()?;
    configure_shell_link(&link, spec)?;
    save_shell_link(&link, spec.shortcut_path)
}

struct RegKeyGuard(*mut std::ffi::c_void);

impl Drop for RegKeyGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` is an open `HKEY` handle returned by `RegCreateKeyExW` or `RegOpenKeyExW`.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
}

fn validate_reg_text(text: &str, label: &str, allow_empty: bool) -> Result<(), String> {
    if !allow_empty && text.trim().is_empty() {
        return Err(format!("Registry {label} must not be empty."));
    }
    if text.chars().any(|c| matches!(c, '\0' | '\r' | '\n')) {
        return Err(format!(
            "Registry {label} contains invalid control characters."
        ));
    }
    if text.chars().count() > MAX_REG_STRING_CHARS {
        return Err(format!(
            "Registry {label} exceeds maximum length ({MAX_REG_STRING_CHARS} characters)."
        ));
    }
    Ok(())
}

/// Writes a `REG_SZ` string value under `HKEY_CURRENT_USER\<subkey>`, creating the subkey if needed.
pub fn set_hkcu_reg_sz(subkey: &str, value_name: &str, value: &str) -> Result<(), String> {
    validate_reg_text(subkey, "subkey", false)?;
    validate_reg_text(value_name, "value name", false)?;
    validate_reg_text(value, "value", true)?;

    let subkey_wide = str_to_wide_null(subkey);
    let name_wide = str_to_wide_null(value_name);
    let data_wide = str_to_wide_null(value);
    let byte_len = u32::try_from(data_wide.len().saturating_mul(2))
        .map_err(|_| "Registry string byte length overflow.".to_string())?;

    let mut raw_key: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `HKEY_CURRENT_USER` is a valid predefined root key, `subkey_wide` is NUL-terminated UTF-16,
    // and `raw_key` receives an owned `HKEY` handle closed on drop via `RegKeyGuard`.
    let open_status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey_wide.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut raw_key,
            std::ptr::null_mut(),
        )
    };
    if open_status != ERROR_SUCCESS || raw_key.is_null() {
        return Err(format!(
            "Could not open or create HKCU\\{subkey} (Win32 error {open_status})."
        ));
    }
    let _guard = RegKeyGuard(raw_key);

    // SAFETY: `raw_key` is open with `KEY_SET_VALUE`, `name_wide` is NUL-terminated UTF-16,
    // and `data_wide` holds `byte_len` bytes of NUL-terminated UTF-16LE `REG_SZ` data.
    let set_status = unsafe {
        RegSetValueExW(
            raw_key,
            name_wide.as_ptr(),
            0,
            REG_SZ,
            data_wide.as_ptr().cast::<u8>(),
            byte_len,
        )
    };
    if set_status != ERROR_SUCCESS {
        return Err(format!(
            "Could not write registry value `{value_name}` under HKCU\\{subkey} (Win32 error {set_status})."
        ));
    }
    Ok(())
}

/// Deletes `value_name` from `HKEY_CURRENT_USER\<subkey>`, returning `Ok(true)` if removed or `Ok(false)` if absent.
pub fn delete_hkcu_reg_value(subkey: &str, value_name: &str) -> Result<bool, String> {
    validate_reg_text(subkey, "subkey", false)?;
    validate_reg_text(value_name, "value name", false)?;

    let subkey_wide = str_to_wide_null(subkey);
    let name_wide = str_to_wide_null(value_name);
    let mut raw_key: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `HKEY_CURRENT_USER` is a valid root key and `subkey_wide` is NUL-terminated UTF-16.
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_wide.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut raw_key,
        )
    };
    if open_status == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    if open_status != ERROR_SUCCESS || raw_key.is_null() {
        return Err(format!(
            "Could not open HKCU\\{subkey} (Win32 error {open_status})."
        ));
    }
    let _guard = RegKeyGuard(raw_key);

    // SAFETY: `raw_key` is open with `KEY_SET_VALUE` and `name_wide` is NUL-terminated UTF-16.
    let del_status = unsafe { RegDeleteValueW(raw_key, name_wide.as_ptr()) };
    match del_status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        other => Err(format!(
            "Could not delete registry value `{value_name}` from HKCU\\{subkey} (Win32 error {other})."
        )),
    }
}

/// Reads a `REG_SZ` or `REG_EXPAND_SZ` string value from `HKEY_CURRENT_USER\<subkey>`, returning `Ok(None)` if absent.
pub fn read_hkcu_reg_sz(subkey: &str, value_name: &str) -> Result<Option<String>, String> {
    validate_reg_text(subkey, "subkey", false)?;
    validate_reg_text(value_name, "value name", false)?;

    let subkey_wide = str_to_wide_null(subkey);
    let name_wide = str_to_wide_null(value_name);
    let mut raw_key: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `HKEY_CURRENT_USER` is a valid root key and `subkey_wide` is NUL-terminated UTF-16.
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_wide.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut raw_key,
        )
    };
    if open_status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if open_status != ERROR_SUCCESS || raw_key.is_null() {
        return Err(format!(
            "Could not open HKCU\\{subkey} (Win32 error {open_status})."
        ));
    }
    let _guard = RegKeyGuard(raw_key);

    let mut reg_type: u32 = 0;
    let mut byte_len: u32 = 0;
    // SAFETY: Passing `lp_data = null` with non-null `lpcb_data` queries the value type and byte length per Win32 docs.
    let probe_status = unsafe {
        RegQueryValueExW(
            raw_key,
            name_wide.as_ptr(),
            std::ptr::null_mut(),
            &mut reg_type,
            std::ptr::null_mut(),
            &mut byte_len,
        )
    };
    if probe_status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if probe_status != ERROR_SUCCESS {
        return Err(format!(
            "Could not query registry value `{value_name}` under HKCU\\{subkey} (Win32 error {probe_status})."
        ));
    }
    if reg_type != REG_SZ && reg_type != REG_EXPAND_SZ {
        return Err(format!(
            "Registry value `{value_name}` under HKCU\\{subkey} has unsupported type {reg_type}."
        ));
    }
    if byte_len == 0 {
        return Ok(Some(String::new()));
    }
    if (byte_len as usize) > MAX_REG_STRING_BYTES {
        return Err(format!(
            "Registry value `{value_name}` under HKCU\\{subkey} exceeds maximum size ({MAX_REG_STRING_BYTES} bytes)."
        ));
    }

    let mut buf = vec![0_u8; byte_len as usize];
    let mut actual_len = byte_len;
    // SAFETY: `buf` has `actual_len` writable bytes and remains alive for `RegQueryValueExW`.
    let read_status = unsafe {
        RegQueryValueExW(
            raw_key,
            name_wide.as_ptr(),
            std::ptr::null_mut(),
            &mut reg_type,
            buf.as_mut_ptr(),
            &mut actual_len,
        )
    };
    if read_status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if read_status != ERROR_SUCCESS {
        return Err(format!(
            "Could not read registry value `{value_name}` under HKCU\\{subkey} (Win32 error {read_status})."
        ));
    }
    if reg_type != REG_SZ && reg_type != REG_EXPAND_SZ {
        return Err(format!(
            "Registry value `{value_name}` under HKCU\\{subkey} changed to unsupported type {reg_type}."
        ));
    }

    let bounded_len = (actual_len as usize).min(buf.len());
    let (pairs, _) = buf[..bounded_len].as_chunks::<2>();
    let mut units = Vec::with_capacity(pairs.len());
    for &[lo, hi] in pairs {
        let unit = u16::from_le_bytes([lo, hi]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Ok(Some(String::from_utf16_lossy(&units)))
}

/// Opens the native Windows Explorer folder picker dialog and returns the selected directory, if confirmed.
pub fn pick_folder(title: &str, initial_dir: Option<&Path>) -> Option<PathBuf> {
    let _com = ComGuard::init()?;
    let dialog = create_file_open_dialog()?;
    if !configure_folder_dialog(&dialog, title, initial_dir) {
        return None;
    }
    // SAFETY: `GetActiveWindow` takes no pointers; `IModalWindow::Show` is slot 3 and `GetResult` is slot 20.
    let result_item = unsafe {
        let owner = GetActiveWindow();
        let show: unsafe extern "system" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32 =
            std::mem::transmute(dialog.vtable_slot(VTBL_IMODAL_WINDOW_SHOW));
        if show(dialog.ptr, owner) < 0 {
            return None;
        }
        let mut item_raw: *mut std::ffi::c_void = std::ptr::null_mut();
        let get_result: unsafe extern "system" fn(
            *mut std::ffi::c_void,
            *mut *mut std::ffi::c_void,
        ) -> i32 = std::mem::transmute(dialog.vtable_slot(VTBL_IFILE_DIALOG_GET_RESULT));
        if get_result(dialog.ptr, &mut item_raw) < 0 || item_raw.is_null() {
            return None;
        }
        ComObject { ptr: item_raw }
    };
    read_shell_item_path(&result_item)
}

struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: `ClipboardGuard` is created only after `OpenClipboard` succeeded on this thread.
        unsafe {
            CloseClipboard();
        }
    }
}

fn copy_clipboard_hglobal_bytes(format: u32) -> Option<Vec<u8>> {
    // SAFETY: Caller holds an open clipboard via `ClipboardGuard` on the current thread.
    unsafe {
        if IsClipboardFormatAvailable(format) == 0 {
            return None;
        }
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle);
        if size == 0 || size > MAX_CLIPBOARD_IMAGE_BYTES {
            return None;
        }
        let ptr = GlobalLock(handle);
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), size).to_vec();
        GlobalUnlock(handle);
        Some(bytes)
    }
}

fn read_clipboard_registered_image(name: &str, magic: &[u8]) -> Option<Vec<u8>> {
    let wide = str_to_wide_null(name);
    // SAFETY: `wide` is a NUL-terminated UTF-16 string valid for this call.
    let fmt = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if fmt == 0 {
        return None;
    }
    let bytes = copy_clipboard_hglobal_bytes(fmt)?;
    if !bytes.starts_with(magic) {
        return None;
    }
    Some(if magic == b"\x89PNG\r\n\x1a\n" {
        trim_png_trailing_bytes(bytes)
    } else {
        bytes
    })
}

fn read_clipboard_webp_format(name: &str) -> Option<Vec<u8>> {
    let wide = str_to_wide_null(name);
    // SAFETY: `wide` is a NUL-terminated UTF-16 string valid for this call.
    let fmt = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if fmt == 0 {
        return None;
    }
    let bytes = copy_clipboard_hglobal_bytes(fmt)?;
    (bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP").then_some(bytes)
}

fn read_clipboard_hdrop_images() -> Vec<PathBuf> {
    let mut out = Vec::new();
    // SAFETY: Caller holds an open clipboard via `ClipboardGuard` on the current thread.
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP) == 0 {
            return out;
        }
        let h_drop = GetClipboardData(CF_HDROP);
        if h_drop.is_null() {
            return out;
        }
        let count = DragQueryFileW(h_drop, u32::MAX, std::ptr::null_mut(), 0);
        for index in 0..count.min(64) {
            let needed = DragQueryFileW(h_drop, index, std::ptr::null_mut(), 0);
            if needed == 0 || needed > 32_768 {
                continue;
            }
            let mut buf = vec![0_u16; needed as usize + 1];
            let copied = DragQueryFileW(h_drop, index, buf.as_mut_ptr(), buf.len() as u32);
            if copied == 0 {
                continue;
            }
            let path = PathBuf::from(String::from_utf16_lossy(&buf[..copied as usize]));
            if path.is_file() && is_supported_image_extension(&path) {
                out.push(path);
            }
        }
    }
    out
}

fn open_clipboard() -> Option<ClipboardGuard> {
    // SAFETY: `GetActiveWindow` takes no pointers; `OpenClipboard` accepts an HWND or null.
    let opened = unsafe {
        let hwnd = GetActiveWindow();
        if OpenClipboard(hwnd) != 0 {
            true
        } else {
            OpenClipboard(std::ptr::null_mut()) != 0
        }
    };
    opened.then_some(ClipboardGuard)
}

/// Reads plain UTF-16 text (`CF_UNICODETEXT`) from the Windows clipboard, if present.
pub fn read_clipboard_text() -> Option<String> {
    let _guard = open_clipboard()?;
    let bytes = copy_clipboard_hglobal_bytes(CF_UNICODETEXT)?;
    let (pairs, _) = bytes[..bytes.len().min(MAX_CLIPBOARD_TEXT_BYTES)].as_chunks::<2>();
    let mut units = Vec::with_capacity(pairs.len());
    for &[lo, hi] in pairs {
        let unit = u16::from_le_bytes([lo, hi]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    let text = String::from_utf16_lossy(&units)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    (!text.is_empty()).then_some(text)
}

/// Writes UTF-16 text (`CF_UNICODETEXT`) onto the Windows clipboard.
pub fn write_clipboard_text(text: &str) -> bool {
    let Some(_guard) = open_clipboard() else {
        return false;
    };
    let mut wide: Vec<u16> = text
        .encode_utf16()
        .filter(|&c| c != 0)
        .take(MAX_CLIPBOARD_TEXT_BYTES / 2)
        .collect();
    wide.push(0);
    let byte_len = wide.len().saturating_mul(2);
    // SAFETY: Caller holds an open clipboard via `ClipboardGuard`; `GlobalAlloc` allocates
    // `byte_len` movable bytes that are copied while locked and transferred to Windows when
    // `SetClipboardData` succeeds (freed explicitly on failure).
    unsafe {
        if EmptyClipboard() == 0 {
            return false;
        }
        let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if hmem.is_null() {
            return false;
        }
        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            GlobalFree(hmem);
            return false;
        }
        let raw_bytes: Vec<u8> = wide.iter().flat_map(|u| u.to_le_bytes()).collect();
        std::ptr::copy_nonoverlapping(raw_bytes.as_ptr(), ptr.cast::<u8>(), raw_bytes.len());
        GlobalUnlock(hmem);
        if SetClipboardData(CF_UNICODETEXT, hmem).is_null() {
            GlobalFree(hmem);
            return false;
        }
    }
    true
}

/// Reads ordered candidate image payloads (`PNG`, `WebP`, `JPEG`, `GIF`, `TIFF`,
/// `CF_DIBV5`, `CF_DIB`, `CF_HDROP`) from the Windows clipboard so callers can fall back
/// cleanly if one provider format fails decoding.
pub fn read_clipboard_image_candidates() -> Vec<crate::ClipboardImage> {
    let Some(_guard) = open_clipboard() else {
        return Vec::new();
    };
    let mut out = Vec::new();

    for (name, magic) in [
        ("PNG", &b"\x89PNG\r\n\x1a\n"[..]),
        ("image/png", &b"\x89PNG\r\n\x1a\n"[..]),
        ("image/jpeg", &[0xFF, 0xD8, 0xFF][..]),
        ("JFIF", &[0xFF, 0xD8, 0xFF][..]),
        ("GIF", b"GIF89a"),
        ("GIF", b"GIF87a"),
        ("image/gif", b"GIF89a"),
        ("image/gif", b"GIF87a"),
        ("image/tiff", b"II*\x00"),
        ("image/tiff", b"MM\x00*"),
    ] {
        if let Some(bytes) = read_clipboard_registered_image(name, magic) {
            out.push(crate::ClipboardImage::EncodedBytes(bytes));
        }
    }
    if let Some(webp) = read_clipboard_webp_format("image/webp") {
        out.push(crate::ClipboardImage::EncodedBytes(webp));
    }
    for format in [CF_DIBV5, CF_DIB] {
        if let Some(dib) = copy_clipboard_hglobal_bytes(format)
            && let Some(bmp) = dib_to_bmp_bytes(&dib)
        {
            out.push(crate::ClipboardImage::EncodedBytes(bmp));
        }
    }
    for path in read_clipboard_hdrop_images() {
        out.push(crate::ClipboardImage::FilePath(path));
    }
    out
}

/// Reads the highest-priority image payload (`PNG`, `CF_DIBV5`, `CF_DIB`, or `CF_HDROP`)
/// from the Windows clipboard.
pub fn read_clipboard_image() -> Option<crate::ClipboardImage> {
    read_clipboard_image_candidates().into_iter().next()
}

const WH_GETMESSAGE: i32 = 3;
const WH_CALLWNDPROC: i32 = 4;

fn to_raw_packet(info: &POINTER_PEN_INFO) -> RawPenPacket {
    RawPenPacket {
        pointer_id: info.pointer_info.pointer_id,
        pointer_flags: info.pointer_info.pointer_flags,
        pen_flags: info.pen_flags,
        pen_mask: info.pen_mask,
        pressure: info.pressure,
        rotation: info.rotation,
        tilt_x: info.tilt_x,
        tilt_y: info.tilt_y,
        pixel_x: info.pointer_info.pt_pixel_location.x,
        pixel_y: info.pointer_info.pt_pixel_location.y,
        dw_time: info.pointer_info.dw_time,
    }
}

fn read_pen_history(pointer_id: u32) -> Vec<RawPenPacket> {
    // SAFETY: `buf` is stack-allocated and zero-initialized; `GetPointerPenInfoHistory` writes at
    // most `MAX_HISTORY` entries into `buf` and returns non-zero on success.
    let history_packets = unsafe {
        let mut buf: [POINTER_PEN_INFO; MAX_HISTORY] = std::mem::zeroed();
        let mut count = MAX_HISTORY as u32;
        if GetPointerPenInfoHistory(pointer_id, &mut count, buf.as_mut_ptr()) != 0 && count > 0 {
            let n = (count as usize).min(MAX_HISTORY);
            // Windows returns history newest-first; reverse to deliver oldest-to-newest.
            Some(buf[..n].iter().rev().map(to_raw_packet).collect::<Vec<_>>())
        } else {
            None
        }
    };
    if let Some(pkts) = history_packets {
        return pkts;
    }

    // SAFETY: `info` is stack-allocated and zero-initialized and outlives the call.
    unsafe {
        let mut info: POINTER_PEN_INFO = std::mem::zeroed();
        if GetPointerPenInfo(pointer_id, &mut info) != 0 {
            vec![to_raw_packet(&info)]
        } else {
            Vec::new()
        }
    }
}

fn clear_active_pen() {
    BARREL_DOWN.store(false, Ordering::Relaxed);
    SECONDARY_BARREL_DOWN.store(false, Ordering::Relaxed);
    PASTE_REQUESTED.store(false, Ordering::Relaxed);
    PEN_ID.store(NO_PEN, Ordering::Relaxed);
    PEN_END.store(0, Ordering::Relaxed);
    HAS_PEN_TIME.store(false, Ordering::Relaxed);
    let mut guard = SAMPLE_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    guard.latest = None;
}

fn is_vk_down(vk: i32) -> bool {
    // SAFETY: `GetKeyState` takes a virtual-key integer and reads thread key state with no pointers.
    unsafe { (GetKeyState(vk) as u16 & 0x8000) != 0 }
}

fn maybe_clear_stale_pen_on_mouse(msg_time_ms: u32) {
    let pen_id = PEN_ID.load(Ordering::Relaxed);
    if pen_id == NO_PEN {
        return;
    }
    // SAFETY: `GetMessageExtraInfo` reads thread-local metadata for the current queued message.
    let extra = unsafe { GetMessageExtraInfo() };
    let is_promoted = is_pointer_promoted_mouse_extra_info(extra);
    if is_promoted {
        return;
    }
    // SAFETY: `info` is stack-allocated and zero-initialized before querying `GetPointerPenInfo`.
    let os_alive = unsafe {
        let mut info: POINTER_PEN_INFO = std::mem::zeroed();
        GetPointerPenInfo(pen_id, &mut info) != 0
    };
    let elapsed = HAS_PEN_TIME
        .load(Ordering::Relaxed)
        .then(|| msg_time_ms.wrapping_sub(LAST_PEN_TIME_MS.load(Ordering::Relaxed)));
    if should_clear_stale_pen_on_mouse(true, is_promoted, os_alive, elapsed) {
        clear_active_pen();
    }
}

fn handle_message(message: u32, w_param: usize, read_history: bool, msg_time_ms: Option<u32>) {
    if !matches!(
        message,
        WM_POINTERDOWN
            | WM_POINTERUPDATE
            | WM_POINTERUP
            | WM_POINTERENTER
            | WM_POINTERLEAVE
            | WM_POINTERCAPTURECHANGED
    ) {
        return;
    }
    let prev = {
        let guard = SAMPLE_STATE.lock().unwrap_or_else(PoisonError::into_inner);
        PenState {
            barrel: BARREL_DOWN.load(Ordering::Relaxed),
            barrel_secondary: SECONDARY_BARREL_DOWN.load(Ordering::Relaxed),
            end: decode_end(PEN_END.load(Ordering::Relaxed)),
            pen_id: Some(PEN_ID.load(Ordering::Relaxed)).filter(|&id| id != NO_PEN),
            contact_end: decode_end(CONTACT_END.load(Ordering::Relaxed)),
            latest: guard.latest,
        }
    };
    let (next, samples) = prev.step(message, w_param, |id| {
        if read_history {
            read_pen_history(id)
        } else {
            Vec::new()
        }
    });
    if !prev.barrel && next.barrel {
        BARREL_PRESS_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    if !prev.barrel_secondary && next.barrel_secondary {
        SECONDARY_BARREL_PRESS_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    BARREL_DOWN.store(next.barrel, Ordering::Relaxed);
    SECONDARY_BARREL_DOWN.store(next.barrel_secondary, Ordering::Relaxed);
    PEN_END.store(encode_end(next.end), Ordering::Relaxed);
    PEN_ID.store(next.pen_id.unwrap_or(NO_PEN), Ordering::Relaxed);
    CONTACT_END.store(encode_end(next.contact_end), Ordering::Relaxed);
    if next.pen_id.is_some() {
        if let Some(t) = msg_time_ms
            && matches!(message, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERENTER)
        {
            LAST_PEN_TIME_MS.store(t, Ordering::Relaxed);
            HAS_PEN_TIME.store(true, Ordering::Relaxed);
        }
    } else {
        HAS_PEN_TIME.store(false, Ordering::Relaxed);
    }

    let mut guard = SAMPLE_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    guard.latest = next.latest;
    for s in samples {
        if guard.queue.len() >= MAX_QUEUED_SAMPLES {
            guard.queue.pop_front();
        }
        guard.queue.push_back(s);
    }
}

unsafe extern "system" fn get_msg_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    // Only record queued pointer messages when removed (`PM_REMOVE`); `PM_NOREMOVE` peeks
    // visit the same queued message multiple times and would duplicate history entries.
    if code >= 0 && (w_param & PM_REMOVE) != 0 && l_param != 0 {
        // SAFETY: Win32 guarantees `l_param` points to a valid `MSG` when `code >= 0`.
        let msg = unsafe { &*(l_param as *const MSG) };
        if msg.message == WM_KEYDOWN {
            let initial_press = (msg.l_param & (1 << 30)) == 0;
            let ctrl_v = msg.w_param == VK_V && is_vk_down(VK_CONTROL) && !is_vk_down(VK_MENU);
            let shift_insert = msg.w_param == VK_INSERT && is_vk_down(VK_SHIFT);
            if initial_press && (ctrl_v || shift_insert) {
                PASTE_REQUESTED.store(true, Ordering::Relaxed);
            }
        } else if matches!(
            msg.message,
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_MOUSEWHEEL
        ) {
            maybe_clear_stale_pen_on_mouse(msg.time);
        } else {
            handle_message(msg.message, msg.w_param, true, Some(msg.time));
        }
    }
    // SAFETY: Passing through to the next hook in the chain per Win32 docs.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

unsafe extern "system" fn call_wnd_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && l_param != 0 {
        // SAFETY: Win32 guarantees `l_param` points to a valid `CWPSTRUCT` when `code >= 0`.
        let msg = unsafe { &*(l_param as *const CWPSTRUCT) };
        match msg.message {
            WM_POINTERENTER => {
                // SAFETY: `GetMessageTime` takes no pointers and returns the Win32 message tick.
                let t = unsafe { GetMessageTime() } as u32;
                handle_message(msg.message, msg.w_param, true, Some(t));
            }
            WM_POINTERLEAVE | WM_POINTERCAPTURECHANGED => {
                handle_message(msg.message, msg.w_param, false, None);
            }
            WM_POINTERDEVICECHANGE | WM_POINTERDEVICEINRANGE => {
                DEVICES_DIRTY.store(true, Ordering::Relaxed);
            }
            WM_KILLFOCUS => clear_active_pen(),
            _ => {}
        }
    }
    // SAFETY: Passing through to the next hook in the chain per Win32 docs.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

/// Thread-local Windows pointer hook handle. Removed cleanly on drop.
pub struct Monitor {
    hooks: [*mut std::ffi::c_void; 2],
}

impl Monitor {
    pub fn install() -> Result<Self, crate::Error> {
        // SAFETY: `GetCurrentThreadId` has no preconditions.
        let tid = unsafe { GetCurrentThreadId() };
        // SAFETY: Installs thread-local `WH_GETMESSAGE` and `WH_CALLWNDPROC` hooks on the current UI thread.
        let (h_get, h_wnd) = unsafe {
            (
                SetWindowsHookExW(WH_GETMESSAGE, get_msg_proc, std::ptr::null_mut(), tid),
                SetWindowsHookExW(WH_CALLWNDPROC, call_wnd_proc, std::ptr::null_mut(), tid),
            )
        };
        if h_get.is_null() {
            if !h_wnd.is_null() {
                // SAFETY: `h_wnd` is non-null from `SetWindowsHookExW`.
                unsafe {
                    UnhookWindowsHookEx(h_wnd);
                }
            }
            return Err(crate::Error::Platform(
                "SetWindowsHookExW(WH_GETMESSAGE) returned null".into(),
            ));
        }
        Ok(Self {
            hooks: [h_get, h_wnd],
        })
    }

    pub fn tracks_leave(&self) -> bool {
        !self.hooks[1].is_null()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        for hook in self.hooks {
            if !hook.is_null() {
                // SAFETY: `hook` is a non-null handle returned by `SetWindowsHookExW` and unhooked once.
                unsafe {
                    UnhookWindowsHookEx(hook);
                }
            }
        }
        clear_active_pen();
        BARREL_PRESS_COUNT.store(0, Ordering::Relaxed);
        SECONDARY_BARREL_PRESS_COUNT.store(0, Ordering::Relaxed);
        CONTACT_END.store(0, Ordering::Relaxed);
        let mut guard = SAMPLE_STATE.lock().unwrap_or_else(PoisonError::into_inner);
        guard.queue.clear();
    }
}

pub fn is_barrel_button_down() -> bool {
    BARREL_DOWN.load(Ordering::Relaxed)
}

pub fn is_secondary_barrel_button_down() -> bool {
    SECONDARY_BARREL_DOWN.load(Ordering::Relaxed)
}

/// Returns and clears the count of primary barrel button presses observed since the last poll.
pub fn take_barrel_press_count() -> u32 {
    BARREL_PRESS_COUNT.swap(0, Ordering::Relaxed)
}

/// Returns and clears the count of secondary barrel button presses observed since the last poll.
pub fn take_secondary_barrel_press_count() -> u32 {
    SECONDARY_BARREL_PRESS_COUNT.swap(0, Ordering::Relaxed)
}

/// Returns and clears whether Windows reported a pointer device plug/unplug change.
pub fn take_pen_devices_dirty() -> bool {
    DEVICES_DIRTY.swap(false, Ordering::Relaxed)
}

/// Queries Windows for connected integrated or external pen digitizers.
pub fn connected_pen_devices() -> Vec<crate::ConnectedPenDevice> {
    let mut out = Vec::new();
    let mut count: u32 = 0;
    // SAFETY: Passing `std::ptr::null_mut()` with a valid `&mut count` queries the device count per Win32 docs.
    let ok = unsafe { GetPointerDevices(&mut count, std::ptr::null_mut()) };
    if ok != 0 && count > 0 {
        let n = (count as usize).min(MAX_POINTER_DEVICES);
        // SAFETY: `POINTER_DEVICE_INFO` is a plain C POD struct where all-zero bits are valid.
        let mut buf: Vec<POINTER_DEVICE_INFO> = vec![unsafe { std::mem::zeroed() }; n];
        let mut read_count = n as u32;
        // SAFETY: `buf` has capacity and length `n >= read_count`, and remains live for this call.
        if unsafe { GetPointerDevices(&mut read_count, buf.as_mut_ptr()) } != 0 {
            let returned = (read_count as usize).min(n);
            for dev in &buf[..returned] {
                let kind = match dev.pointer_device_type {
                    POINTER_DEVICE_TYPE_INTEGRATED_PEN => Some(crate::PenDeviceKind::Integrated),
                    POINTER_DEVICE_TYPE_EXTERNAL_PEN => Some(crate::PenDeviceKind::External),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let name = crate::win_ink::sanitize_pen_device_name(
                        &dev.product_string,
                        kind == crate::PenDeviceKind::External,
                    );
                    out.push(crate::ConnectedPenDevice { name, kind });
                }
            }
        }
    }

    if out.is_empty() {
        // SAFETY: `GetSystemMetrics` takes an integer metric index and uses no pointers.
        let metrics = unsafe { GetSystemMetrics(SM_DIGITIZER) };
        if (metrics & NID_READY) != 0 {
            if (metrics & NID_INTEGRATED_PEN) != 0 {
                out.push(crate::ConnectedPenDevice {
                    name: "Integrated Stylus".to_string(),
                    kind: crate::PenDeviceKind::Integrated,
                });
            }
            if (metrics & NID_EXTERNAL_PEN) != 0 {
                out.push(crate::ConnectedPenDevice {
                    name: "External Pen Tablet".to_string(),
                    kind: crate::PenDeviceKind::External,
                });
            }
        }
    }

    out
}

/// Returns and clears the queued Windows paste shortcut signal (`Ctrl+V` or `Shift+Insert`).
pub fn take_paste_requested() -> bool {
    PASTE_REQUESTED.swap(false, Ordering::Relaxed)
}

/// Active Win32 pointer ID while a stylus is in digitizer proximity (`None` after `WM_POINTERLEAVE`).
pub fn pen_pointer_id() -> Option<u64> {
    let id = PEN_ID.load(Ordering::Relaxed);
    (id != NO_PEN).then_some(u64::from(id))
}

/// Cloaks or uncloaks a Win32 window via DWM (`DWMWA_CLOAK`) so hidden transitions never flash.
pub fn set_window_cloaked(hwnd: isize, cloaked: bool) -> bool {
    const DWMWA_CLOAK: u32 = 13;
    if hwnd == 0 {
        return false;
    }
    let handle = std::ptr::without_provenance_mut::<std::ffi::c_void>(hwnd as usize);
    // SAFETY: `IsWindow` safely validates whether `handle` refers to an existing Win32 window.
    if unsafe { IsWindow(handle) } == 0 {
        return false;
    }
    let value: i32 = i32::from(cloaked);
    // SAFETY: `handle` was verified by `IsWindow` and `value` is a live `BOOL` (`i32`) matching `DWMWA_CLOAK`.
    let hr = unsafe {
        DwmSetWindowAttribute(
            handle,
            DWMWA_CLOAK,
            (&raw const value).cast(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    hr >= 0
}

pub fn pen_end() -> Option<bool> {
    decode_end(PEN_END.load(Ordering::Relaxed))
}

pub fn take_pen_end() -> Option<bool> {
    decode_end(CONTACT_END.swap(0, Ordering::Relaxed)).or_else(pen_end)
}

pub fn latest_sample() -> Option<Sample> {
    SAMPLE_STATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .latest
}

pub fn drain_samples() -> Vec<Sample> {
    SAMPLE_STATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .queue
        .drain(..)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_device_info_layout_matches_winuser_abi() {
        let expected = if cfg!(target_pointer_width = "64") {
            1080
        } else {
            1064
        };
        assert_eq!(std::mem::size_of::<POINTER_DEVICE_INFO>(), expected);
    }

    #[test]
    fn connected_pen_devices_queries_cleanly_without_panicking() {
        let devices = connected_pen_devices();
        for dev in devices {
            assert!(!dev.name.is_empty());
        }
    }

    #[test]
    fn folder_dialog_com_guid_and_vtable_constants_match_abi() {
        assert_eq!(std::mem::size_of::<ComGuid>(), 16);
        assert_eq!(CLSID_FILE_OPEN_DIALOG.data1, 0xdc1c_5a9c);
        assert_eq!(IID_I_FILE_DIALOG.data1, 0x42f8_5136);
        assert_eq!(IID_I_SHELL_ITEM.data1, 0x4382_6d1e);
        assert_eq!(FOLDERID_DESKTOP.data1, 0xb4bf_cc3a);
        assert_eq!(CLSID_SHELL_LINK.data1, 0x0002_1401);
        assert_eq!(IID_I_SHELL_LINK_W.data1, 0x0002_14f9);
        assert_eq!(IID_I_PERSIST_FILE.data1, 0x0000_010b);
        assert_eq!(IUNKNOWN_VTBL_LEN, 3);
        assert_eq!(IFILE_DIALOG_VTBL_LEN, 27);
        assert_eq!(ISHELL_ITEM_VTBL_LEN, 8);
        assert_eq!(ISHELL_LINK_VTBL_LEN, 21);
        assert_eq!(IPERSIST_FILE_VTBL_LEN, 9);
    }

    #[test]
    fn desktop_dir_resolves_and_creates_valid_lnk_in_temp_dir() {
        assert!(desktop_dir().is_some());

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("papercraft_lnk_test_{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let dummy_exe = dir.join("papercraft.exe");
        std::fs::write(&dummy_exe, b"").unwrap();
        let lnk_path = dir.join("PaperCraft.lnk");

        create_desktop_shortcut(WindowsShortcutSpec {
            shortcut_path: &lnk_path,
            target_path: &dummy_exe,
            working_dir: Some(&dir),
            description: "PaperCraft shortcut test",
            icon_path: Some(&dummy_exe),
        })
        .unwrap();

        let bytes = std::fs::read(&lnk_path).unwrap();
        assert!(bytes.len() > 76);
        // Shell Link Binary File Format always begins with HeaderSize = 0x0000004C.
        assert_eq!(&bytes[..4], &[0x4c, 0x00, 0x00, 0x00]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hkcu_registry_set_read_delete_roundtrip_in_ephemeral_key() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let subkey = format!(
            r"Software\PaperCraftRegTest_{}_{}",
            std::process::id(),
            nanos
        );
        struct RegCleanup(String);
        impl Drop for RegCleanup {
            fn drop(&mut self) {
                let wide = str_to_wide_null(&self.0);
                // SAFETY: `HKEY_CURRENT_USER` is valid and `wide` is NUL-terminated UTF-16 for an ephemeral test key.
                unsafe {
                    let _ = RegDeleteKeyW(HKEY_CURRENT_USER, wide.as_ptr());
                }
            }
        }
        let _cleanup = RegCleanup(subkey.clone());
        let value_name = "PaperCraft Server";

        assert_eq!(read_hkcu_reg_sz(&subkey, value_name).unwrap(), None);

        let cmd =
            r#""C:\Program Files\Alyx Labs\PaperCraft Server\papercraft-server.exe" --autostart"#;
        set_hkcu_reg_sz(&subkey, value_name, cmd).unwrap();
        assert_eq!(
            read_hkcu_reg_sz(&subkey, value_name).unwrap().as_deref(),
            Some(cmd)
        );

        let unicode_cmd = r#""D:\Apps\Área de Trabalho\papercraft-server.exe" --autostart"#;
        set_hkcu_reg_sz(&subkey, value_name, unicode_cmd).unwrap();
        assert_eq!(
            read_hkcu_reg_sz(&subkey, value_name).unwrap().as_deref(),
            Some(unicode_cmd)
        );

        assert!(delete_hkcu_reg_value(&subkey, value_name).unwrap());
        assert!(!delete_hkcu_reg_value(&subkey, value_name).unwrap());
        assert_eq!(read_hkcu_reg_sz(&subkey, value_name).unwrap(), None);

        assert!(set_hkcu_reg_sz("", value_name, cmd).is_err());
        assert!(set_hkcu_reg_sz(&subkey, value_name, "bad\nline").is_err());
    }

    #[test]
    fn set_window_cloaked_rejects_null_and_invalid_handles() {
        assert!(!set_window_cloaked(0, true));
        assert!(!set_window_cloaked(-1, false));
    }
}
