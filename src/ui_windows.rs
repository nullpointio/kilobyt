//! Native Win32 shell for Kilobyt (PRD §8).
//!
//! Design rules this file follows:
//! * only standard controls (STATIC / BUTTON / EDIT / ListView / PROGRESS), never
//!   custom-painted ones, so the system theme and DPI behaviour stay Windows' business;
//! * the batch runs on a `std::thread`; the worker touches the UI exclusively through
//!   one `PostMessageW(WM_APP_EV)` after pushing to a shared queue, so no user32 call
//!   ever runs off the UI thread;
//! * every `unsafe` block is small and says why it is sound.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kilobyt::{Settings, E_CANCELLED};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetDC, GetDeviceCaps, GetStockObject, ReleaseDC, SetBkMode,
    UpdateWindow, HBRUSH, HDC, HFONT, LOGPIXELSY, WHITE_BRUSH,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
    OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, EM_SETLIMITTEXT, ICC_LISTVIEW_CLASSES, ICC_PROGRESS_CLASS,
    INITCOMMONCONTROLSEX, LVCFMT_LEFT, LVCF_FMT, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW,
    LVIF_TEXT, LVITEMW, LVM_DELETEITEM, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW,
    LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMTEXTW, LVNI_SELECTED,
    LVN_GETINFOTIPW, LVN_KEYDOWN, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_EX_INFOTIP,
    LVS_REPORT, LVS_SHOWSELALWAYS, NMHDR, NMLVGETINFOTIPW, NMLVKEYDOWN, PBM_SETPOS, PBM_SETRANGE32,
    PROGRESS_CLASSW, TOOLTIPS_CLASSW, TTF_IDISHWND, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH,
    TTS_ALWAYSTIP, TTTOOLINFOW,
};
use windows_sys::Win32::UI::Shell::{
    DragAcceptFiles, DragFinish, DragQueryFileW, ShellExecuteW, HDROP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, IsDialogMessageW, KillTimer, LoadCursorW, LoadIconW, LoadImageW, MoveWindow,
    PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    BS_AUTORADIOBUTTON, ES_AUTOHSCROLL, ES_NUMBER, GWLP_USERDATA, HICON, IDC_ARROW,
    IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LR_DEFAULTSIZE, MINMAXINFO, MSG, SM_CXSCREEN,
    SM_CXSMICON, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW, SW_SHOWNORMAL, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DPICHANGED, WM_DROPFILES, WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCCREATE, WM_NOTIFY, WM_SETFONT,
    WM_SETICON, WM_SIZE, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_MAXIMIZEBOX,
    WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

// ---------------------------------------------------------------------------
// Win32 bits that the enabled `windows-sys` features do not expose.
//
// The crate's feature list covers Foundation/Gdi/LibraryLoader/Controls/Dialogs/
// Shell/WindowsAndMessaging only. Clipboard access (Win32_System_DataExchange) and
// `GetAsyncKeyState` (Win32_UI_Input_KeyboardAndMouse) live outside it, and
// Cargo.toml is not ours to edit, so the five calls are declared here. user32 and
// kernel32 are always loaded, so these imports can never fail to resolve.
// ---------------------------------------------------------------------------
mod raw {
    use super::HWND;
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        pub fn OpenClipboard(hwnd: HWND) -> i32;
        pub fn EmptyClipboard() -> i32;
        pub fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
        pub fn CloseClipboard() -> i32;
        pub fn GetAsyncKeyState(key: i32) -> i16;
        pub fn EnableWindow(hwnd: HWND, enable: i32) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        pub fn CoTaskMemFree(mem: *mut c_void);
    }

    /// `BROWSEINFOW` + `SHBrowseForFolderW` are gated behind the
    /// `Win32_UI_Shell_Common` feature in windows-sys, so the two shell calls and
    /// their C struct are declared here. Layout is the documented one (8 fields,
    /// `iImage` included).
    #[repr(C)]
    #[allow(non_snake_case)] // C field names keep the layout checkable against the SDK header
    pub struct BrowseInfoW {
        pub hwndOwner: HWND,
        pub pidlRoot: *mut c_void,
        pub pszDisplayName: *mut u16,
        pub lpszTitle: *const u16,
        pub ulFlags: u32,
        pub lpfn: *const c_void,
        pub lParam: isize,
        pub iImage: i32,
    }

    #[link(name = "shell32")]
    extern "system" {
        pub fn SHBrowseForFolderW(bi: *const BrowseInfoW) -> *mut c_void;
        pub fn SHGetPathFromIDListW(pidl: *const c_void, path: *mut u16) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        pub fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        pub fn GlobalUnlock(mem: *mut c_void) -> i32;
    }
}

/// Control / command identifiers. `1` and `2` are left free: `IsDialogMessageW`
/// reports Enter and Esc as `IDOK` / `IDCANCEL`, which is exactly the binding
/// PRD §8 asks for.
const ID_ADD: i32 = 100;
const ID_LIST: i32 = 101;
const ID_KB: i32 = 102;
const ID_CHK_DIMS: i32 = 103;
const ID_W: i32 = 104;
const ID_H: i32 = 105;
const ID_RB_SAME: i32 = 106;
const ID_RB_THIS: i32 = 107;
const ID_DIR: i32 = 108;
const ID_BROWSE: i32 = 109;
const ID_COMPRESS: i32 = 110;
const ID_CANCEL_RUN: i32 = 111;
const ID_OPEN_FOLDER: i32 = 112;

const IDOK: i32 = 1;
const IDCANCEL: i32 = 2;
const ID_COPY: i32 = 200;

/// Private message: "the shared event queue has new entries".
const WM_APP_EV: u32 = WM_APP + 1;
const TIMER_DRAIN: usize = 1;
const TIMER_PULSE: usize = 2;

// Control styles/messages the enabled feature set does not export. Values are
// the documented ones from commctrl.h / winuser.h.
const ICC_STANDARD_CLASSES: u32 = 0x0000_4000;

// Virtual-key codes (the feature that exports them is not enabled).
const VK_RETURN: i32 = 0x0D;
const VK_ESCAPE: i32 = 0x1B;
const VK_DELETE: i32 = 0x2E;
const VK_CONTROL: i32 = 0x11;
const VK_O: i32 = 0x4F;

const EM_CHAR_LIMIT: usize = 6; // chars allowed in the numeric edits

/// Shown under the empty list's headers; also the marker `refresh` uses to know
/// that no real status has been written yet.
const EMPTY_HINT: &str = "No photos yet — drag some in, or click Add photos…";

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Status {
    Queued,
    Working,
    Done,
    Failed(String),
    Cancelled,
}

impl Status {
    fn text(&self) -> String {
        match self {
            Status::Queued => "Queued".to_string(),
            Status::Working => "Working…".to_string(),
            Status::Done => "Done".to_string(),
            Status::Failed(reason) => format!("Failed: {reason}"),
            Status::Cancelled => "Cancelled".to_string(),
        }
    }

    /// Column-sized version: the first sentence of a failure reason, so the
    /// list shows *why* rather than a mid-word cut-off. The whole reason is
    /// still available through `text()` (infotip and failure dialog).
    fn column(&self) -> String {
        let Status::Failed(reason) = self else {
            return self.text();
        };
        let head = reason.split_once(". ").map(|(a, _)| a).unwrap_or(reason);
        let mut line = format!("Failed: {head}");
        if line.chars().count() > 34 {
            line = line.chars().take(33).collect::<String>() + "…";
        }
        line
    }
}

struct Item {
    path: PathBuf,
    name: String,
    size_kb: f64,
    result: String,
    status: Status,
}

/// One UI-thread event produced by the worker.
enum Ev {
    Started(usize),
    Finished {
        idx: usize,
        result: String,
        status: Status,
        out: Option<PathBuf>,
    },
    Done,
}

/// Where outputs go.
enum Target {
    SameFolder,
    ThisFolder(PathBuf),
}

struct App {
    hwnd: HWND,
    dpi: u32,
    font: HFONT,
    font_big: HFONT,

    zone: HWND,
    drop_main: HWND,
    drop_sub: HWND,
    add_btn: HWND,
    tip: HWND,

    list: HWND,

    lbl_max: HWND,
    kb_edit: HWND,
    lbl_kb: HWND,
    chk_dims: HWND,
    lbl_w: HWND,
    w_edit: HWND,
    lbl_h: HWND,
    h_edit: HWND,
    lbl_save: HWND,
    rb_same: HWND,
    rb_this: HWND,
    dir_edit: HWND,
    browse_btn: HWND,

    compress_btn: HWND,
    cancel_btn: HWND,
    open_btn: HWND,
    bar: HWND,
    status: HWND,

    items: Vec<Item>,
    running: bool,
    cancel: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<Ev>>>,
    total: usize,
    done_count: usize,
    fail_count: usize,
    /// Sub-file ramp for the progress bar, in percent of one photo's slot.
    pulse: u32,
    last_out: Option<PathBuf>,
    last_kb: u32,
    modal: HWND,
    close_when_done: bool,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// NUL-terminated UTF-16. The caller keeps it alive across the API call.
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn zeroed<T>() -> T {
    // SAFETY: every struct we zero is a plain C struct of integers/pointers,
    // for which all-zero is a valid (null / 0) initial state.
    unsafe { std::mem::zeroed() }
}

fn scale(v: i32, dpi: u32) -> i32 {
    ((v as i64 * dpi as i64) / 96) as i32
}

fn lp_of<T>(p: &T) -> LPARAM {
    p as *const T as isize
}

fn get_text(hwnd: HWND) -> String {
    // SAFETY: `buf` is sized from the control's own reported length and the
    // call is told exactly how many units it may write.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn set_text(hwnd: HWND, s: &str) {
    let t = w(s);
    // SAFETY: `t` is NUL-terminated and outlives the call; Windows copies it.
    unsafe {
        SetWindowTextW(hwnd, t.as_ptr());
    }
}

fn app_mut(hwnd: HWND) -> Option<&'static mut App> {
    // SAFETY: GWLP_USERDATA only ever holds the `Box<App>` we leaked in `run()`,
    // or 0 before it is installed / after it is freed in WM_DESTROY.
    unsafe {
        let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        p.as_mut()
    }
}

/// The icon compiled into the executable: `assets/kilobyt.rc` declares it as
/// `1 ICON`, so resource id 1 is the whole multi-resolution group and Windows
/// picks the frame that fits `size`.
///
/// Falls back to the stock application icon, so a build made with the resource
/// compiler skipped still shows *an* icon rather than none.
fn app_icon(size: i32) -> HICON {
    // SAFETY: null asks for the module handle of this executable; the resource
    // id is a literal and the flags are the documented pick-best-frame pair.
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let icon = LoadImageW(
            hinst,
            1 as *const u16,
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR | LR_DEFAULTSIZE,
        );
        if icon.is_null() {
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
        } else {
            icon
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() -> i32 {
    // SAFETY: standard process bootstrap; every handle is checked before use.
    unsafe {
        enable_dpi_awareness();

        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&icc);

        let hinst = GetModuleHandleW(std::ptr::null());
        let class = w("KilobytWindow");
        let cursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: app_icon(0),
            hCursor: cursor,
            hbrBackground: (16 + 1) as HBRUSH, // COLOR_BTNFACE + 1: plain system face
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return 1;
        }

        // Failure-list modal gets its own tiny class.
        let fclass = w("KilobytFailures");
        let fwc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(fail_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: cursor,
            hbrBackground: (16 + 1) as HBRUSH,
            lpszMenuName: std::ptr::null(),
            lpszClassName: fclass.as_ptr(),
        };
        RegisterClassW(&fwc);

        let dpi = screen_dpi();
        let title = w("Kilobyt");
        let window_style =
            (WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX) | WS_CLIPCHILDREN;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: scale(760, dpi),
            bottom: scale(620, dpi),
        };
        AdjustWindowRect(&mut frame, window_style, 0);
        let win_w = frame.right - frame.left;
        let win_h = frame.bottom - frame.top;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let win_x = (screen_w - win_w) / 2;
        let win_y = (screen_h - win_h) / 2;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            window_style,
            win_x,
            win_y,
            win_w,
            win_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return 1;
        }

        let app = build_ui(hwnd, dpi);
        let app_ptr = Box::into_raw(Box::new(app));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);

        DragAcceptFiles(hwnd, 1);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        layout(app_mut(hwnd).expect("app installed above"));
        refresh(app_mut(hwnd).expect("app installed above"));

        // SAFETY: MSG is a plain C struct.
        let mut msg: MSG = zeroed();
        loop {
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r <= 0 {
                break;
            }
            // Global shortcuts, ahead of IsDialogMessageW so Tab still works:
            // this is the only way to see keys while a child EDIT has focus.
            if msg.message == WM_KEYDOWN && handle_hotkey(hwnd, msg.wParam as i32) {
                continue;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        msg.wParam as i32
    }
}

unsafe fn enable_dpi_awareness() {
    // Resolved at runtime: `SetProcessDpiAwarenessContext` only exists on
    // Windows 10 1607+, and a static import would stop the exe from loading on
    // anything older.
    let name = b"SetProcessDpiAwarenessContext\0";
    let user32 = GetModuleHandleW(w("user32.dll").as_ptr());
    if user32.is_null() {
        return;
    }
    let proc = GetProcAddress(user32, name.as_ptr());
    if let Some(f) = proc {
        type SetCtx = unsafe extern "system" fn(isize) -> i32;
        // SAFETY: the export has exactly this signature (HANDLE -> BOOL).
        let f: SetCtx = std::mem::transmute(f);
        // -4 == DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        f(-4);
    }
}

fn screen_dpi() -> u32 {
    // SAFETY: GetDC(NULL) yields a screen DC that is released immediately.
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return 96;
        }
        let dpi = GetDeviceCaps(dc, LOGPIXELSY as i32);
        ReleaseDC(std::ptr::null_mut(), dc);
        if dpi <= 0 {
            96
        } else {
            dpi as u32
        }
    }
}

fn make_font(pt: i32, dpi: u32, weight: i32) -> HFONT {
    let height = -((pt * dpi as i32 + 36) / 72);
    let face = w("Segoe UI");
    // SAFETY: 14-argument GDI call with a valid, NUL-terminated face name.
    let f = unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr(),
        )
    };
    if f.is_null() {
        // SAFETY: stock objects are process-wide and must not be deleted.
        unsafe { GetStockObject(17) as HFONT } // DEFAULT_GUI_FONT
    } else {
        f
    }
}

// ---------------------------------------------------------------------------
// UI construction
// ---------------------------------------------------------------------------

fn build_ui(hwnd: HWND, dpi: u32) -> App {
    let font = make_font(9, dpi, 400);
    let font_big = make_font(16, dpi, 600);

    // SAFETY: all children are created under `hwnd`; strings outlive each call.
    unsafe {
        let zone = mk(
            hwnd,
            w("STATIC").as_ptr(),
            "",
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_BORDER,
            WS_EX_CLIENTEDGE,
            0,
        );

        let drop_main = mk(
            hwnd,
            w("STATIC").as_ptr(),
            "Drop photos here",
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0,
            0,
        );
        let drop_sub = mk(
            hwnd,
            w("STATIC").as_ptr(),
            "They stay on this PC. Nothing is uploaded.",
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0,
            0,
        );

        let add = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "Add photos…",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            ID_ADD,
        );

        let list = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w("SysListView32").as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | (LVS_REPORT | LVS_SHOWSELALWAYS),
            0,
            0,
            0,
            0,
            hwnd,
            ID_LIST as usize as *mut c_void,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            0,
            (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_INFOTIP) as isize,
        );

        // No empty-state overlay: it used to hide the list (and its column
        // headers) entirely. The list stays visible with headers, and the hint
        // lives in the status line instead.

        let lbl_max = mk(
            hwnd,
            w("STATIC").as_ptr(),
            "Max size",
            WS_CHILD | WS_VISIBLE,
            0,
            0,
        );
        let kb_edit = mk(
            hwnd,
            w("EDIT").as_ptr(),
            "100",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | (ES_AUTOHSCROLL as u32)
                | (ES_NUMBER as u32),
            0,
            ID_KB,
        );
        let lbl_kb = mk(
            hwnd,
            w("STATIC").as_ptr(),
            "KB",
            WS_CHILD | WS_VISIBLE,
            0,
            0,
        );

        let chk = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "Limit dimensions",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
            0,
            ID_CHK_DIMS,
        );
        let lbl_w = mk(hwnd, w("STATIC").as_ptr(), "W", WS_CHILD | WS_VISIBLE, 0, 0);
        let w_edit = mk(
            hwnd,
            w("EDIT").as_ptr(),
            "1600",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | (ES_AUTOHSCROLL as u32)
                | (ES_NUMBER as u32),
            0,
            ID_W,
        );
        let lbl_h = mk(hwnd, w("STATIC").as_ptr(), "H", WS_CHILD | WS_VISIBLE, 0, 0);
        let h_edit = mk(
            hwnd,
            w("EDIT").as_ptr(),
            "1600",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | (ES_AUTOHSCROLL as u32)
                | (ES_NUMBER as u32),
            0,
            ID_H,
        );

        let lbl_save = mk(
            hwnd,
            w("STATIC").as_ptr(),
            "Save to",
            WS_CHILD | WS_VISIBLE,
            0,
            0,
        );
        let rb_same = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "Same folder as original",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | (BS_AUTORADIOBUTTON as u32),
            0,
            ID_RB_SAME,
        );
        let rb_this = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "This folder:",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | (BS_AUTORADIOBUTTON as u32),
            0,
            ID_RB_THIS,
        );
        let dir_edit = mk(
            hwnd,
            w("EDIT").as_ptr(),
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | (ES_AUTOHSCROLL as u32)
                | ES_READONLY_U32,
            0,
            ID_DIR,
        );
        let browse = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "Browse…",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            ID_BROWSE,
        );

        let compress = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "Compress",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
            0,
            ID_COMPRESS,
        );
        let cancel_btn = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "Cancel",
            WS_CHILD | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            ID_CANCEL_RUN,
        );
        let open_btn = mk(
            hwnd,
            w("BUTTON").as_ptr(),
            "Open folder",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            ID_OPEN_FOLDER,
        );
        let bar = CreateWindowExW(
            0,
            PROGRESS_CLASSW,
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            hwnd,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        let status = mk(
            hwnd,
            w("STATIC").as_ptr(),
            "Ready",
            WS_CHILD | WS_VISIBLE,
            0,
            0,
        );

        // Tooltip that turns into an infotip for the list (full path per row).
        let tip = CreateWindowExW(
            0,
            TOOLTIPS_CLASSW,
            std::ptr::null(),
            WS_POPUP | TTS_ALWAYSTIP,
            0,
            0,
            0,
            0,
            hwnd,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        SendMessageW(tip, TTM_SETMAXTIPWIDTH, 0, 600);
        let mut ti: TTTOOLINFOW = zeroed();
        ti.cbSize = size_of::<TTTOOLINFOW>() as u32;
        ti.uFlags = TTF_IDISHWND;
        ti.hwnd = hwnd;
        ti.uId = list as usize;
        // LPSTR_TEXTCALLBACKW (-1): the text comes from LVN_GETINFOTIPW, so no
        // buffer has to outlive this function.
        ti.lpszText = -1isize as *mut u16;
        SendMessageW(tip, TTM_ADDTOOLW, 0, lp_of(&ti));

        // Columns: File / Size / Result / Status.
        for (i, (label, width)) in [
            ("File", 240),
            ("Size", 70),
            ("Result", 120),
            ("Status", 150),
        ]
        .iter()
        .enumerate()
        {
            let mut col: LVCOLUMNW = zeroed();
            let text = w(label);
            col.mask = LVCF_FMT | LVCF_WIDTH | LVCF_TEXT | LVCF_SUBITEM;
            col.fmt = LVCFMT_LEFT;
            col.cx = scale(*width, dpi);
            col.pszText = text.as_ptr() as *mut u16;
            col.iSubItem = i as i32;
            SendMessageW(list, LVM_INSERTCOLUMNW, i, lp_of(&col));
        }

        // The zone frame is a sibling that overlaps the two labels and the
        // button, so their stacking order is stated explicitly instead of being
        // assumed from creation order.
        for h in [drop_main, drop_sub, add] {
            SetWindowPos(
                h,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        // Radio default: same folder.
        SendMessageW(rb_same, BM_SETCHECK_U32, 1, 0);

        for h in [
            zone, drop_main, drop_sub, add, list, lbl_max, kb_edit, lbl_kb, chk, lbl_w, w_edit,
            lbl_h, h_edit, lbl_save, rb_same, rb_this, dir_edit, browse, compress, cancel_btn,
            open_btn, bar, status,
        ] {
            if !h.is_null() {
                SendMessageW(h, WM_SETFONT, font as usize, 1);
            }
        }
        SendMessageW(drop_main, WM_SETFONT, font_big as usize, 1);
        // ICON_BIG (1) drives the taskbar and Alt-Tab; ICON_SMALL (0) the
        // title bar. Both come from the same compiled-in group.
        SendMessageW(hwnd, WM_SETICON, 1, app_icon(0) as isize);
        SendMessageW(
            hwnd,
            WM_SETICON,
            0,
            app_icon(GetSystemMetrics(SM_CXSMICON)) as isize,
        );

        // Character limits on the numeric fields.
        for h in [kb_edit, w_edit, h_edit] {
            SendMessageW(h, EM_SETLIMITTEXT, EM_CHAR_LIMIT, 0);
        }

        App {
            hwnd,
            dpi,
            font,
            font_big,
            zone,
            drop_main,
            drop_sub,
            add_btn: add,
            tip,
            list,
            lbl_max,
            kb_edit,
            lbl_kb,
            chk_dims: chk,
            lbl_w,
            w_edit,
            lbl_h,
            h_edit,
            lbl_save,
            rb_same,
            rb_this,
            dir_edit,
            browse_btn: browse,
            compress_btn: compress,
            cancel_btn,
            open_btn,
            bar,
            status,
            items: Vec::new(),
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            events: Arc::new(Mutex::new(Vec::new())),
            total: 0,
            done_count: 0,
            fail_count: 0,
            pulse: 0,
            last_out: None,
            last_kb: 100,
            modal: std::ptr::null_mut(),
            close_when_done: false,
        }
    }
}

const ES_READONLY_U32: u32 = 0x0800; // ES_READONLY is `i32` in windows-sys
const BS_PUSHBUTTON: u32 = 0;
const BS_DEFPUSHBUTTON: u32 = 1;
const BS_AUTOCHECKBOX: u32 = 3;
const BM_SETCHECK_U32: u32 = 0x00F1;

unsafe fn mk(parent: HWND, class: *const u16, text: &str, style: u32, ex: u32, id: i32) -> HWND {
    let t = w(text);
    // SAFETY: class/text are NUL-terminated and only read during the call.
    CreateWindowExW(
        ex,
        class,
        t.as_ptr(),
        style,
        0,
        0,
        0,
        0,
        parent,
        id as usize as *mut c_void,
        std::ptr::null_mut(),
        std::ptr::null(),
    )
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

fn layout(app: &mut App) {
    // SAFETY: `rc` is a valid RECT owned by this frame.
    let mut rc: RECT = zeroed();
    unsafe {
        GetClientRect(app.hwnd, &mut rc);
    }
    let (w, h) = (rc.right, rc.bottom);
    if w <= 0 || h <= 0 {
        return;
    }

    let dpi = app.dpi;
    let s = |v: i32| scale(v, dpi);

    // Fixed client geometry: 760x620. Left margin 16, right edge 752, so the
    // settings panel and the action row both end at the same x.
    let left = s(16);
    let right = s(752);
    let zone_top = s(8);
    let zone_h = s(88);
    let list_top = s(104);
    let list_w = s(480); // 16..496
    let panel_x = s(512);
    let panel_w = s(240); // 512..752
    let btn_h = s(32);
    let btn_y = h - s(16) - btn_h;
    let list_h = s(396); // 104..500

    // Drop zone: y 8..96, full width up to the common right edge. Only the
    // Add photos button lives on the right side of this band.
    mv(app.zone, left, zone_top, right - left, zone_h);
    let add_w = s(140);
    let add_x = right - s(12) - add_w;
    mv(app.add_btn, add_x, zone_top + s(18), add_w, s(34));
    let drop_x = left + s(16);
    let drop_w = add_x - s(16) - drop_x;
    mv(app.drop_main, drop_x, zone_top + s(8), drop_w, s(34));
    mv(app.drop_sub, drop_x, zone_top + s(44), drop_w, s(20));

    // List view: 16..496, 104..500.
    mv(app.list, left, list_top, list_w, list_h);

    // Columns are proportional: Status and Result are the columns a user reads
    // when something fails, so they never collapse to a sliver (D-02). Status
    // also grows to fit its longest message (up to 45 % of the list). The sum is
    // always exactly the list's inner width, so no horizontal scrollbar.
    let avail = (list_w - s(26)).max(s(120));
    let size_w = s(72);
    let result_w = ((avail * 22) / 100).max(s(90));
    let longest_status = app
        .items
        .iter()
        .map(|i| i.status.column().chars().count())
        .max()
        .unwrap_or(4) as i32;
    let wanted = s(8) * longest_status + s(20);
    let mut status_w = wanted.clamp(s(120), (avail * 45) / 100);
    let mut file_w = avail - size_w - result_w - status_w;
    if file_w < s(80) {
        let cut = (s(80) - file_w).min((status_w - s(120)).max(0));
        status_w -= cut;
        file_w = avail - size_w - result_w - status_w;
    }
    // SAFETY: columns 0..3 exist; LVM_SETCOLUMNWIDTH takes plain integers.
    unsafe {
        SendMessageW(app.list, LVM_SETCOLUMNWIDTH, 0, file_w.max(1) as isize);
        SendMessageW(app.list, LVM_SETCOLUMNWIDTH, 1, size_w as isize);
        SendMessageW(app.list, LVM_SETCOLUMNWIDTH, 2, result_w as isize);
        SendMessageW(app.list, LVM_SETCOLUMNWIDTH, 3, status_w as isize);
    }

    // Settings panel: every control stays inside x=512..752.
    let mut y = list_top;
    mv(app.lbl_max, panel_x, y, s(120), s(18));
    y += s(20);
    mv(app.kb_edit, panel_x, y, s(90), s(24));
    mv(app.lbl_kb, panel_x + s(96), y + s(4), s(30), s(18));
    y += s(36);
    mv(app.chk_dims, panel_x, y, panel_w, s(22));
    y += s(26);
    let edit_w = s(90);
    mv(app.lbl_w, panel_x, y + s(4), s(14), s(18));
    mv(app.w_edit, panel_x + s(18), y, edit_w, s(24));
    let h_x = panel_x + s(18) + edit_w + s(12);
    mv(app.lbl_h, h_x, y + s(4), s(14), s(18));
    mv(app.h_edit, h_x + s(18), y, edit_w, s(24));
    y += s(40);
    mv(app.lbl_save, panel_x, y, s(120), s(18));
    y += s(20);
    mv(app.rb_same, panel_x, y, panel_w, s(22));
    y += s(26);
    mv(app.rb_this, panel_x, y, panel_w, s(22));
    y += s(26);
    // Browse sits flush with the panel's right edge; the read-only folder edit
    // shrinks to leave room instead of pushing Browse off-screen.
    let browse_w = s(78);
    let browse_x = panel_x + panel_w - browse_w;
    let dir_w = panel_w - browse_w - s(8);
    mv(app.dir_edit, panel_x, y, dir_w, s(24));
    mv(app.browse_btn, browse_x, y - s(1), browse_w, s(26));

    // Bottom action row: right-aligned at x=752. Cancel shares Compress's
    // rectangle and is simply shown on top while a batch is running.
    let compress_w = s(120);
    let compress_x = right - compress_w;
    mv(app.compress_btn, compress_x, btn_y, compress_w, btn_h);
    mv(app.cancel_btn, compress_x, btn_y, compress_w, btn_h);
    let open_w = s(110);
    let open_x = compress_x - s(8) - open_w;
    mv(app.open_btn, open_x, btn_y, open_w, btn_h);

    // Status line and progress bar above the action row, left-aligned.
    let text_w = (open_x - s(12) - left).max(s(120));
    mv(app.status, left, s(508), text_w, s(18));
    mv(app.bar, left, s(530), text_w, s(10));
}

fn mv(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    // SAFETY: plain positioning call on a live child window.
    unsafe {
        MoveWindow(hwnd, x, y, w.max(1), h.max(1), 1);
    }
}

// ---------------------------------------------------------------------------
// Progress bar
//
// The engine exposes no sub-file progress: `fit_cancellable` returns only when
// a photo is fully fitted. A purely determinate bar therefore cannot move
// during a single 50 s encode, which is why the old bar looked frozen (D-03).
// The fill is therefore a real base (finished photos) plus a slow ramp that
// advances while a photo is in flight. It is capped and snapped back to the
// truthful value at the end of the batch, and it can only reach 100 % when
// every queued photo produced an output.
// ---------------------------------------------------------------------------

/// Repaint the bar from the model: completed photos (hundredths) plus the
/// in-flight ramp. Never reaches the end while the batch is still running.
const PULSE_TICK_MS: u32 = 400;
const PULSE_CAP: u32 = 90; // per-cent of one photo's slot

fn set_bar_pct(app: &App, pos: usize) {
    // SAFETY: plain progress-bar message on a live child.
    unsafe {
        SendMessageW(app.bar, PBM_SETPOS, pos, 0);
    }
}

fn paint_bar(app: &App) {
    let total = app.total * 100;
    if total == 0 {
        return;
    }
    let base = finished_count(app) * 100;
    let mut pos = (base + app.pulse as usize).min(total.saturating_sub(1));
    if !app.running {
        pos = ok_count(app) * 100;
    }
    set_bar_pct(app, pos);
}

fn ok_count(app: &App) -> usize {
    app.items
        .iter()
        .filter(|i| matches!(i.status, Status::Done))
        .count()
}

fn finished_count(app: &App) -> usize {
    app.items
        .iter()
        .filter(|i| {
            matches!(
                i.status,
                Status::Done | Status::Failed(_) | Status::Cancelled
            )
        })
        .count()
}

/// Ask the worker to stop. The bar drops back to zero immediately and stays
/// there: a cancelled run is never allowed to look complete.
fn begin_cancel(app: &mut App) {
    if !app.running {
        return;
    }
    app.cancel.store(true, Ordering::SeqCst);
    set_text(app.status, "Cancelling…");
    app.pulse = 0;
    // SAFETY: we own this timer id.
    unsafe {
        KillTimer(app.hwnd, TIMER_PULSE);
    }
    set_bar_pct(app, 0);
}

// ---------------------------------------------------------------------------
// State refresh
// ---------------------------------------------------------------------------

fn read_settings(app: &App) -> Option<Settings> {
    let kb: u32 = get_text(app.kb_edit).trim().parse().ok()?;
    if !(1..=99999).contains(&kb) {
        return None;
    }
    let dims = unsafe { SendMessageW(app.chk_dims, BM_GETCHECK, 0, 0) } == BM_CHECKED;
    if dims {
        let cw: u32 = get_text(app.w_edit).trim().parse().ok()?;
        let ch: u32 = get_text(app.h_edit).trim().parse().ok()?;
        if !(16..=20000).contains(&cw) || !(16..=20000).contains(&ch) {
            return None;
        }
        Some(Settings {
            target_kb: kb,
            max_width: cw,
            max_height: ch,
        })
    } else {
        Some(Settings {
            target_kb: kb,
            max_width: 0,
            max_height: 0,
        })
    }
}

const BM_GETCHECK: u32 = 0x00F0;
const BM_CHECKED: isize = 1;

fn refresh(app: &mut App) {
    let dims = unsafe { SendMessageW(app.chk_dims, BM_GETCHECK, 0, 0) } == BM_CHECKED;
    let this_folder = unsafe { SendMessageW(app.rb_this, BM_GETCHECK, 0, 0) } == BM_CHECKED;
    let idle = !app.running;

    let settings_ok = read_settings(app).is_some();
    let has_rows = !app.items.is_empty();

    // SAFETY: EnableWindow / ShowWindow on live children; no pointers involved.
    unsafe {
        for h in [
            app.add_btn,
            app.list,
            app.kb_edit,
            app.chk_dims,
            app.rb_same,
            app.rb_this,
            app.zone,
            app.drop_main,
        ] {
            raw::EnableWindow(h, i32::from(idle));
        }
        raw::EnableWindow(app.w_edit, i32::from(idle && dims));
        raw::EnableWindow(app.h_edit, i32::from(idle && dims));
        raw::EnableWindow(app.dir_edit, i32::from(idle && this_folder));
        raw::EnableWindow(app.browse_btn, i32::from(idle && this_folder));
        raw::EnableWindow(app.compress_btn, i32::from(idle && has_rows && settings_ok));
        raw::EnableWindow(app.open_btn, i32::from(app.last_out.is_some()));

        ShowWindow(app.cancel_btn, if app.running { SW_SHOW } else { SW_HIDE });
        ShowWindow(app.open_btn, if app.running { SW_HIDE } else { SW_SHOW });
    }

    // The list is always on show, so its column headers stay visible even when
    // there are no rows (D-06). The hint lives in the status line instead of an
    // overlay window, and is replaced as soon as the first row arrives.
    if !has_rows && !app.running && app.done_count == 0 {
        set_text(app.status, EMPTY_HINT);
    } else if has_rows && get_text(app.status) == EMPTY_HINT {
        set_text(app.status, "Ready");
    }
}

// ---------------------------------------------------------------------------
// List helpers
// ---------------------------------------------------------------------------

fn set_row(app: &App, idx: usize) {
    let Some(it) = app.items.get(idx) else { return };
    let r = it.result.clone();
    let st = it.status.column();
    let size = size_text(it.size_kb);
    // SAFETY: each LVITEMW borrows a string that outlives the SendMessage call,
    // and LVM_SETITEMTEXTW only reads it.
    unsafe {
        for (sub, text) in [(1usize, size), (2, r), (3, st)] {
            let t = w(&text);
            let mut item: LVITEMW = zeroed();
            item.mask = LVIF_TEXT;
            item.iItem = idx as i32;
            item.iSubItem = sub as i32;
            item.pszText = t.as_ptr() as *mut u16;
            SendMessageW(app.list, LVM_SETITEMTEXTW, idx, lp_of(&item));
        }
    }
}

/// Size column text. `size_kb` is decimal kilobytes (bytes / 1000), so values at
/// or above 1000 KB are shown as MB with a unit rather than a bare number (D-04).
fn size_text(size_kb: f64) -> String {
    if size_kb >= 1000.0 {
        format!("{:.1} MB", size_kb / 1000.0)
    } else if size_kb >= 1.0 {
        format!("{:.0} KB", size_kb)
    } else {
        "—".to_string()
    }
}

fn add_rows(app: &mut App, paths: Vec<PathBuf>) {
    for p in paths {
        if app.items.iter().any(|it| it.path == p) {
            continue;
        }
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned());
        let size_kb = std::fs::metadata(&p)
            .map(|m| m.len() as f64 / 1000.0)
            .unwrap_or(0.0);
        let idx = app.items.len();
        app.items.push(Item {
            path: p,
            name: name.clone(),
            size_kb,
            result: "—".to_string(),
            status: Status::Queued,
        });

        // SAFETY: LVITEMW points at a NUL-terminated string alive for the call.
        unsafe {
            let t = w(&name);
            let mut item: LVITEMW = zeroed();
            item.mask = LVIF_TEXT;
            item.iItem = idx as i32;
            item.iSubItem = 0;
            item.pszText = t.as_ptr() as *mut u16;
            SendMessageW(app.list, LVM_INSERTITEMW, 0, lp_of(&item));
        }
        set_row(app, idx);
    }
    layout(app);
    refresh(app);
}

fn remove_selected(app: &mut App) {
    if app.running || app.items.is_empty() {
        return;
    }
    let mut selected: Vec<usize> = Vec::new();
    // SAFETY: LVM_GETNEXTITEM returns -1 ("none") or a valid row index.
    unsafe {
        let mut start: isize = -1;
        loop {
            let n = SendMessageW(
                app.list,
                LVM_GETNEXTITEM,
                start as usize,
                LVNI_SELECTED as isize,
            );
            if n < 0 {
                break;
            }
            selected.push(n as usize);
            start = n;
        }
        selected.sort_unstable();
        selected.dedup();
        for idx in selected.iter().rev() {
            SendMessageW(app.list, LVM_DELETEITEM, *idx, 0);
        }
    }
    for idx in selected.iter().rev() {
        if *idx < app.items.len() {
            app.items.remove(*idx);
        }
    }
    refresh(app);
}

// ---------------------------------------------------------------------------
// Batch
// ---------------------------------------------------------------------------

fn start_batch(app: &mut App) {
    if app.running || app.items.is_empty() {
        return;
    }
    let Some(settings) = read_settings(app) else {
        set_text(
            app.status,
            "Check the settings: 1–99999 KB, sides 16–20000 px.",
        );
        return;
    };
    let target = if unsafe { SendMessageW(app.rb_this, BM_GETCHECK, 0, 0) } == BM_CHECKED {
        let dir = PathBuf::from(get_text(app.dir_edit));
        if dir.as_os_str().is_empty() || !dir.is_dir() {
            set_text(app.status, "Pick an output folder first.");
            return;
        }
        Target::ThisFolder(dir)
    } else {
        Target::SameFolder
    };

    app.last_kb = settings.target_kb;
    app.total = app.items.len();
    app.done_count = 0;
    app.fail_count = 0;
    app.last_out = None;
    app.cancel = Arc::new(AtomicBool::new(false));
    app.running = true;

    for it in app.items.iter_mut() {
        it.result = "—".to_string();
        it.status = Status::Queued;
    }
    for idx in 0..app.items.len() {
        set_row(app, idx);
    }

    // SAFETY: progress range 0..=(total*100); the bar exists.
    app.pulse = 0;
    unsafe {
        SendMessageW(app.bar, PBM_SETRANGE32, 0, (app.total * 100) as isize);
        SetTimer(app.hwnd, TIMER_DRAIN, 250, None);
        SetTimer(app.hwnd, TIMER_PULSE, PULSE_TICK_MS, None);
    }
    set_bar_pct(app, 0);
    set_text(app.status, &format!("Compressing 0 of {}…", app.total));

    let jobs: Vec<(usize, PathBuf, String)> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| (i, it.path.clone(), it.name.clone()))
        .collect();
    let hwnd_raw = app.hwnd as isize;
    let events = Arc::clone(&app.events);
    let cancel = Arc::clone(&app.cancel);

    // The worker owns no HWND: it posts one message per event and never calls
    // a user32 function that touches a window.
    std::thread::spawn(move || worker(hwnd_raw, jobs, settings, target, events, cancel));
    refresh(app);
}

fn worker(
    hwnd_raw: isize,
    jobs: Vec<(usize, PathBuf, String)>,
    settings: Settings,
    target: Target,
    events: Arc<Mutex<Vec<Ev>>>,
    cancel: Arc<AtomicBool>,
) {
    let mut reserved: HashMap<PathBuf, HashSet<String>> = HashMap::new();

    let post = |ev: Ev| {
        if let Ok(mut q) = events.lock() {
            q.push(ev);
        }
        // SAFETY: PostMessageW is documented thread-safe; the HWND was captured
        // on the UI thread and is not dereferenced here.
        unsafe {
            PostMessageW(hwnd_raw as HWND, WM_APP_EV, 0, 0);
        }
    };

    for (idx, path, name) in jobs {
        if cancel.load(Ordering::SeqCst) {
            post(Ev::Finished {
                idx,
                result: "—".to_string(),
                status: Status::Cancelled,
                out: None,
            });
            continue;
        }
        post(Ev::Started(idx));

        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                post(Ev::Finished {
                    idx,
                    result: "—".to_string(),
                    status: Status::Failed("File is gone.".to_string()),
                    out: None,
                });
                continue;
            }
        };

        let outcome = kilobyt::fit_cancellable_owned(bytes, &name, &settings, &cancel);
        if !outcome.ok {
            let status = if outcome.err_code == E_CANCELLED {
                Status::Cancelled
            } else {
                Status::Failed(if outcome.err_detail.is_empty() {
                    "Could not fit this file.".to_string()
                } else {
                    outcome.err_detail
                })
            };
            post(Ev::Finished {
                idx,
                result: "—".to_string(),
                status,
                out: None,
            });
            continue;
        }

        let dir = match &target {
            Target::SameFolder => path.parent().map(Path::to_path_buf).unwrap_or_default(),
            Target::ThisFolder(d) => d.clone(),
        };
        let stem = kilobyt::stem_of(&name);
        let slot = reserved.entry(dir.clone()).or_default();

        match kilobyt::commit(&outcome.jpeg, &dir, &stem, slot) {
            Ok(out) => {
                let kb = (outcome.best_bytes + 500) / 1000;
                post(Ev::Finished {
                    idx,
                    result: format!("{} KB · {}×{}", kb, outcome.width, outcome.height),
                    status: Status::Done,
                    out: Some(out),
                });
            }
            Err(msg) => {
                // `commit` reports "<CODE>: <message>"; the code is for logs only.
                let reason = msg
                    .split_once(": ")
                    .map(|(_, m)| m.to_string())
                    .unwrap_or(msg);
                post(Ev::Finished {
                    idx,
                    result: "—".to_string(),
                    status: Status::Failed(reason),
                    out: None,
                });
            }
        }
    }

    post(Ev::Done);
}

fn drain(app: &mut App) {
    let batch: Vec<Ev> = match app.events.lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => return,
    };
    let mut rows_changed = false;
    for ev in batch {
        match ev {
            Ev::Started(idx) => {
                if let Some(it) = app.items.get_mut(idx) {
                    it.status = Status::Working;
                }
                set_row(app, idx);
                // New photo: restart the ramp within its slot so the bar is
                // visibly alive during the long encode.
                app.pulse = 0;
                paint_bar(app);
            }
            Ev::Finished {
                idx,
                result,
                status,
                out,
            } => {
                let failed = matches!(status, Status::Failed(_));
                if let Some(it) = app.items.get_mut(idx) {
                    it.result = result;
                    it.status = status;
                }
                if failed {
                    app.fail_count += 1;
                }
                if out.is_some() {
                    app.last_out = out;
                }
                app.done_count += 1;
                app.pulse = 0;
                rows_changed = true;
                set_row(app, idx);
                paint_bar(app);
                if app.running {
                    set_text(
                        app.status,
                        &format!("Compressing {} of {}…", app.done_count, app.total),
                    );
                }
            }
            Ev::Done => finish_batch(app),
        }
    }
    if rows_changed {
        // Status text changed size (e.g. a failure reason), so the column widths
        // are recomputed to keep the message readable.
        layout(app);
    }
}

fn finish_batch(app: &mut App) {
    if !app.running {
        return;
    }
    app.running = false;
    app.cancel.store(false, Ordering::SeqCst);
    // SAFETY: we set this timer ourselves in start_batch.
    unsafe {
        KillTimer(app.hwnd, TIMER_DRAIN);
    }

    let total = app.total;
    let ok = app
        .items
        .iter()
        .filter(|i| matches!(i.status, Status::Done))
        .count();
    let failed = app
        .items
        .iter()
        .filter(|i| matches!(i.status, Status::Failed(_)))
        .count();
    let cancelled = app
        .items
        .iter()
        .filter(|i| matches!(i.status, Status::Cancelled))
        .count();

    app.running = false;
    app.pulse = 0;
    // SAFETY: we set this timer ourselves in start_batch.
    unsafe {
        KillTimer(app.hwnd, TIMER_PULSE);
    }
    let final_pos = if cancelled > 0 {
        0
    } else if failed > 0 {
        ok * 100
    } else {
        total * 100
    };
    // Final fill: full only when every queued photo produced an output; zero
    // after a cancel; partial when anything failed (D-03).
    set_bar_pct(app, final_pos);

    let line = if cancelled > 0 {
        format!("{} of {} done · {} cancelled", ok, total, cancelled)
    } else if failed == 0 && ok > 0 {
        format!(
            "{} of {} done · all at or under {} KB",
            ok, total, app.last_kb
        )
    } else if failed > 0 {
        format!("{} of {} done · {} failed", ok, total, failed)
    } else {
        "Ready".to_string()
    };
    set_text(app.status, &line);

    // The failure text is wider than "Queued"/"Done": refit the columns before
    // the modal (which runs its own message loop) is shown.
    layout(app);
    refresh(app);

    if failed > 0 {
        let fails: Vec<(String, String)> = app
            .items
            .iter()
            .filter_map(|it| match &it.status {
                Status::Failed(reason) => Some((it.name.clone(), reason.clone())),
                _ => None,
            })
            .collect();
        show_failures(app, &fails);
    }
}

// ---------------------------------------------------------------------------
// Failure modal
// ---------------------------------------------------------------------------

struct FailDlg {
    text: String,
    closed: bool,
}

unsafe extern "system" fn fail_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            if id == IDOK || id == IDCANCEL {
                if let Some(d) = dlg_mut(hwnd) {
                    d.closed = true;
                }
                DestroyWindow(hwnd);
            } else if id == ID_COPY {
                if let Some(d) = dlg_mut(hwnd) {
                    let text = d.text.clone();
                    set_clipboard(hwnd, &text);
                }
            }
            0
        }
        WM_CLOSE => {
            if let Some(d) = dlg_mut(hwnd) {
                d.closed = true;
            }
            DestroyWindow(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn dlg_mut(hwnd: HWND) -> Option<&'static mut FailDlg> {
    // SAFETY: GWLP_USERDATA on the modal window holds the Box<FailDlg> we leak
    // in `show_failures` until its message loop ends.
    unsafe {
        let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut FailDlg;
        p.as_mut()
    }
}

fn show_failures(app: &mut App, fails: &[(String, String)]) {
    if app.modal_ok() {
        return;
    }
    let body: String = fails
        .iter()
        .map(|(name, reason)| format!("{name} — {reason}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    let title = w("Some photos could not be compressed");

    let dlg = Box::into_raw(Box::new(FailDlg {
        text: body.clone(),
        closed: false,
    }));
    // SAFETY: the modal window is created owned by the main window, then all
    // children are positioned on it; the loop below pumps until it closes.
    unsafe {
        let class = w("KilobytFailures");
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            0,
            0,
            0,
            0,
            app.hwnd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if hwnd.is_null() {
            drop(Box::from_raw(dlg));
            return;
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, dlg as isize);

        let dpi = app.dpi;
        let s = |v: i32| scale(v, dpi);
        let face = w("STATIC");
        let edit_class = w("EDIT");
        let button = w("BUTTON");
        let msg_class = w("STATIC");
        let hint = w("These stayed untouched. Nothing was written for them.");
        let copy_label = w("Copy");
        let close_label = w("Close");

        let hint_h = CreateWindowExW(
            0,
            face.as_ptr(),
            hint.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            s(12),
            s(10),
            s(400),
            s(18),
            hwnd,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            edit_class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | 0x0004 | 0x0800, // ES_MULTILINE|ES_READONLY
            s(12),
            s(32),
            s(480),
            s(240),
            hwnd,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        let _ = msg_class;
        let body_w = w(&body_as_crlf(&body));
        SetWindowTextW(edit, body_w.as_ptr());

        let copy = CreateWindowExW(
            0,
            button.as_ptr(),
            copy_label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            s(12),
            s(286),
            s(90),
            s(28),
            hwnd,
            ID_COPY as usize as *mut c_void,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        let close = CreateWindowExW(
            0,
            button.as_ptr(),
            close_label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
            s(402),
            s(286),
            s(90),
            s(28),
            hwnd,
            IDOK as usize as *mut c_void,
            std::ptr::null_mut(),
            std::ptr::null(),
        );

        for h in [hint_h, edit, copy, close] {
            SendMessageW(h, WM_SETFONT, app.font as usize, 1);
        }

        // Center on the owner. The children are laid out for a 504x326 client
        // area, so the frame is added by AdjustWindowRect rather than guessed.
        let mut prc: RECT = zeroed();
        GetClientRect(app.hwnd, &mut prc);
        let (cw, ch) = (s(504), s(326));
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: cw,
            bottom: ch,
        };
        AdjustWindowRect(&mut frame, WS_POPUP | WS_CAPTION | WS_SYSMENU, 0);
        let (ow, oh) = (frame.right - frame.left, frame.bottom - frame.top);
        let (px, py) = (prc.right / 2 - ow / 2, prc.bottom / 2 - oh / 2);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            px,
            py,
            ow,
            oh,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);

        app.modal = hwnd;
        raw::EnableWindow(app.hwnd, 0);

        let mut msg: MSG = zeroed();
        loop {
            let closed = dlg_mut(hwnd).map(|d| d.closed).unwrap_or(true);
            if closed {
                break;
            }
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r == 0 {
                PostQuitMessage(msg.wParam as i32);
                break;
            }
            if r < 0 {
                break;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        app.modal = std::ptr::null_mut();
        raw::EnableWindow(app.hwnd, 1);
        SetForegroundWindow(app.hwnd);
        drop(Box::from_raw(dlg));
    }
}

impl App {
    fn modal_ok(&self) -> bool {
        !self.modal.is_null()
    }
}

fn body_as_crlf(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn set_clipboard(hwnd: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * size_of::<u16>();
    // SAFETY: standard clipboard hand-off. The HGLOBAL is allocated GMEM_MOVEABLE,
    // filled through GlobalLock and then owned by the clipboard (never freed by us).
    unsafe {
        if raw::OpenClipboard(hwnd) == 0 {
            return;
        }
        raw::EmptyClipboard();
        let mem = raw::GlobalAlloc(0x0002, bytes); // GMEM_MOVEABLE
        if !mem.is_null() {
            let dst = raw::GlobalLock(mem);
            if dst.is_null() {
                // Allocation that cannot be locked must not be handed over.
                let _ = raw::GlobalUnlock(mem);
            } else {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), dst.cast::<u16>(), wide.len());
                raw::GlobalUnlock(mem);
                raw::SetClipboardData(13, mem); // CF_UNICODETEXT
            }
        }
        raw::CloseClipboard();
    }
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

fn pick_files(app: &mut App) -> Vec<PathBuf> {
    let mut buf = vec![0u16; 64 * 1024];
    // "Images\0*.jpg;...\0All files\0*.*\0\0" — filter pairs, double-NUL terminated.
    let filter: Vec<u16> = "Images\0*.jpg;*.jpeg;*.png;*.bmp\0All files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title = w("Add photos");
    let mut ofn: OPENFILENAMEW = zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = app.hwnd;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_ALLOWMULTISELECT
        | OFN_EXPLORER
        | OFN_FILEMUSTEXIST
        | OFN_PATHMUSTEXIST
        | OFN_HIDEREADONLY
        | OFN_NOCHANGEDIR;

    // SAFETY: OPENFILENAMEW points at buffers that live for the whole call.
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return Vec::new();
    }

    let parts: Vec<String> = buf
        .split(|&c| c == 0)
        .take_while(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    match parts.len() {
        0 => Vec::new(),
        1 => vec![PathBuf::from(&parts[0])],
        _ => {
            let dir = Path::new(&parts[0]).to_path_buf();
            parts[1..].iter().map(|n| dir.join(n)).collect()
        }
    }
}

fn pick_folder(app: &mut App) {
    let title = w("Choose where to save the compressed photos");
    let mut display = [0u16; 260];
    let bi = raw::BrowseInfoW {
        hwndOwner: app.hwnd,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: display.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: 0x0000_0001 | 0x0000_0010 | 0x0000_0040, // RETURNONLYFSDIRS|EDITBOX|NEWDIALOGSTYLE
        lpfn: std::ptr::null(),
        lParam: 0,
        iImage: 0,
    };

    // SAFETY: the dialog is modal to `hwnd`; the returned PIDL is ours and is
    // released with CoTaskMemFree, as the shell requires.
    unsafe {
        let pidl = raw::SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return;
        }
        let mut buf = [0u16; 260];
        if raw::SHGetPathFromIDListW(pidl, buf.as_mut_ptr()) != 0 {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let dir = String::from_utf16_lossy(&buf[..len]);
            set_text(app.dir_edit, &dir);
            refresh(app);
        }
        raw::CoTaskMemFree(pidl);
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn open_output(app: &App) {
    let Some(dir) = app.last_out.as_ref().and_then(|p| p.parent()) else {
        return;
    };
    let op = w("open");
    let path = w(&dir.to_string_lossy());
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        ShellExecuteW(
            app.hwnd,
            op.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn handle_hotkey(hwnd: HWND, vk: i32) -> bool {
    // SAFETY: GetAsyncKeyState reads global key state only.
    let ctrl = unsafe { raw::GetAsyncKeyState(VK_CONTROL) } < 0;
    let Some(app) = app_mut(hwnd) else {
        return false;
    };
    match vk {
        VK_O if ctrl => {
            if app.running {
                return true;
            }
            let paths = pick_files(app);
            if !paths.is_empty() {
                add_rows(app, paths);
            }
            true
        }
        VK_RETURN => {
            start_batch(app);
            true
        }
        VK_ESCAPE => {
            begin_cancel(app);
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let Some(app) = app_mut(hwnd) else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };

    match msg {
        WM_CREATE => 0,

        WM_DROPFILES => {
            let hdrop = lp as HDROP;
            if !app.running {
                let mut paths = Vec::new();
                let count = DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0);
                for i in 0..count {
                    let len = DragQueryFileW(hdrop, i, std::ptr::null_mut(), 0);
                    let mut buf = vec![0u16; len as usize + 1];
                    let n = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32);
                    if n > 0 {
                        paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])));
                    }
                }
                DragFinish(hdrop);
                add_rows(app, paths);
            } else {
                DragFinish(hdrop);
            }
            0
        }

        WM_SIZE => {
            layout(app);
            0
        }

        WM_GETMINMAXINFO => {
            let info = &mut *(lp as *mut MINMAXINFO);
            // The window is fixed-size: 760x620 client area at any DPI. The
            // track size is the frame size, so compute it from the same style.
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: scale(760, app.dpi),
                bottom: scale(620, app.dpi),
            };
            let style = (WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX) | WS_CLIPCHILDREN;
            AdjustWindowRect(&mut frame, style, 0);
            let fw = frame.right - frame.left;
            let fh = frame.bottom - frame.top;
            info.ptMinTrackSize.x = fw;
            info.ptMinTrackSize.y = fh;
            info.ptMaxTrackSize.x = fw;
            info.ptMaxTrackSize.y = fh;
            0
        }

        WM_DPICHANGED => {
            let new_dpi = (wp & 0xffff) as u32;
            if new_dpi != app.dpi {
                app.dpi = new_dpi;
                // Recreate both fonts, then restyle every control.
                let old = [app.font, app.font_big];
                app.font = make_font(9, new_dpi, 400);
                app.font_big = make_font(16, new_dpi, 600);
                for h in [
                    app.zone,
                    app.drop_main,
                    app.drop_sub,
                    app.add_btn,
                    app.list,
                    app.lbl_max,
                    app.kb_edit,
                    app.lbl_kb,
                    app.chk_dims,
                    app.lbl_w,
                    app.w_edit,
                    app.lbl_h,
                    app.h_edit,
                    app.lbl_save,
                    app.rb_same,
                    app.rb_this,
                    app.dir_edit,
                    app.browse_btn,
                    app.compress_btn,
                    app.cancel_btn,
                    app.open_btn,
                    app.bar,
                    app.status,
                ] {
                    SendMessageW(h, WM_SETFONT, app.font as usize, 1);
                }
                SendMessageW(app.drop_main, WM_SETFONT, app.font_big as usize, 1);
                for f in old {
                    if !f.is_null() {
                        DeleteObject(f);
                    }
                }
                // Keep the client area fixed at 760x620 at the new DPI; only
                // honour the suggested top-left position.
                let rect = &*(lp as *const RECT);
                let mut frame = RECT {
                    left: 0,
                    top: 0,
                    right: scale(760, new_dpi),
                    bottom: scale(620, new_dpi),
                };
                let style =
                    (WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX) | WS_CLIPCHILDREN;
                AdjustWindowRect(&mut frame, style, 0);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    rect.left,
                    rect.top,
                    frame.right - frame.left,
                    frame.bottom - frame.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            0
        }

        WM_CTLCOLORSTATIC => {
            let ctl = lp as HWND;
            let hdc = wp as HDC;
            // SAFETY: HDC belongs to this paint message; only colours are set.
            unsafe {
                SetBkMode(hdc, 1); // TRANSPARENT: let the parent's fill show through
            }
            if ctl == app.zone || ctl == app.drop_main || ctl == app.drop_sub {
                // The drop target is painted white so it reads as a target.
                return GetStockObject(WHITE_BRUSH) as isize;
            }
            // COLOR_BTNFACE + 1 is the standard "match the dialog" brush.
            (16 + 1) as isize
        }

        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            match id {
                IDOK | IDCANCEL => {
                    // Enter from IsDialogMessageW / Esc.
                    if id == IDCANCEL {
                        begin_cancel(app);
                    } else {
                        start_batch(app);
                    }
                }
                ID_ADD => {
                    if !app.running {
                        let paths = pick_files(app);
                        if !paths.is_empty() {
                            add_rows(app, paths);
                        }
                    }
                }
                ID_BROWSE => pick_folder(app),
                ID_COMPRESS => start_batch(app),
                ID_CANCEL_RUN => begin_cancel(app),
                ID_OPEN_FOLDER => open_output(app),
                _ => refresh(app),
            }
            0
        }

        WM_NOTIFY => {
            let hdr = &*(lp as *const NMHDR);
            if hdr.hwndFrom == app.list {
                if hdr.code == LVN_KEYDOWN {
                    let key = &*(lp as *const NMLVKEYDOWN);
                    if key.wVKey as i32 == VK_DELETE {
                        remove_selected(app);
                    }
                }
                if hdr.code == LVN_GETINFOTIPW {
                    let info = &mut *(lp as *mut NMLVGETINFOTIPW);
                    if let Some(it) = app.items.get(info.iItem as usize) {
                        // Column text is clipped by the column width, so the
                        // infotip carries the whole story: size, the full
                        // failure reason and the path (D-02).
                        let tip = format!(
                            "{} · {} · {} · {}",
                            it.name,
                            size_text(it.size_kb),
                            it.status.text(),
                            it.path.display()
                        );
                        if info.cchTextMax > 1 {
                            let dst = info.pszText;
                            let cap = info.cchTextMax as usize;
                            // SAFETY: the control sized `pszText` to `cchTextMax`
                            // UTF-16 units and only reads what we write.
                            let wide: Vec<u16> = tip.encode_utf16().take(cap - 1).collect();
                            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                            *dst.add(wide.len()) = 0;
                        }
                    }
                }
            }
            0
        }

        WM_APP_EV => {
            drain(app);
            if app.close_when_done && !app.running {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_TIMER => {
            if wp == TIMER_PULSE && app.running {
                app.pulse = (app.pulse + 1).min(PULSE_CAP);
                paint_bar(app);
            }
            drain(app);
            0
        }

        WM_CLOSE => {
            if app.running {
                app.close_when_done = true;
                begin_cancel(app);
                set_text(app.status, "Finishing the current photo, then closing…");
            } else {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_DESTROY => {
            let font = app.font;
            let font_big = app.font_big;
            let tip = app.tip;
            // Free the leaked App now that no message can reach it.
            let app_ptr = app as *mut App;
            if !font.is_null() {
                DeleteObject(font);
            }
            if !font_big.is_null() {
                DeleteObject(font_big);
            }
            if !tip.is_null() {
                DestroyWindow(tip);
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(app_ptr));
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
