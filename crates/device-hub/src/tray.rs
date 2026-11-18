//! 常駐 (タスクトレイ) と Windows 起動時の自動起動。
//!
//! - 窓の × は終了ではなくトレイへ隠す (`should_close` が false を返す)。
//!   プロセス監視ルール・デバイスのワーカー・ライブライティングは裏で動き続ける。
//! - トレイアイコン: 左クリック/ダブルクリックで窓を出す、右クリックでメニュー
//!   (表示 / Windows 起動時に自動起動 / 終了)。
//! - 自動起動は `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\DeviceHub` に
//!   `"<exe>" --background` を書く。`--background` は窓を出さずにトレイだけで起動する。
//! - 二重起動防止: 名前付きミューテックス。2 つ目は既存インスタンスのトレイ窓へ
//!   「表示」を投げて終わる (2 つが同じ HID を取り合わないように)。
//!
//! トレイは専用スレッドのメッセージ専用窓で回す (GPUI のメッセージループに手を入れない)。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

/// 自動起動のときに渡す引数。
pub const BACKGROUND_ARG: &str = "--background";

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static QUITTING: AtomicBool = AtomicBool::new(false);

/// 終了メニューが選ばれた (= 窓を本当に閉じてよい)。
pub fn quitting() -> bool {
    QUITTING.load(Ordering::Relaxed)
}

/// コマンドラインに `--background` があるか。
pub fn started_in_background() -> bool {
    std::env::args().any(|a| a == BACKGROUND_ARG)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Registry::*;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::Shell::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const CLASS: &str = "DeviceHubTrayWnd";
    const MUTEX: &str = "Local\\DeviceHub.SingleInstance";
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const RUN_VALUE: &str = "DeviceHub";

    const WM_TRAY: u32 = WM_APP + 1;
    /// 2 つ目のインスタンスから「窓を出して」。
    const WM_SHOW_MAIN: u32 = WM_APP + 2;
    const ID_SHOW: usize = 1;
    const ID_AUTOSTART: usize = 2;
    const ID_QUIT: usize = 3;

    static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // ------------------------------------------------------------ 二重起動

    /// 最初のインスタンスなら true。2 つ目なら既存の窓を出させて false。
    pub fn acquire_single_instance() -> bool {
        unsafe {
            let name = w(MUTEX);
            // ハンドルはプロセス終了まで握ったまま (閉じると次の起動が素通りする)。
            let h = CreateMutexW(null(), 0, name.as_ptr());
            if !h.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
                let cls = w(CLASS);
                let other = FindWindowW(cls.as_ptr(), null());
                if !other.is_null() {
                    PostMessageW(other, WM_SHOW_MAIN, 0, 0);
                }
                return false;
            }
            true
        }
    }

    // ------------------------------------------------------------ 自動起動

    fn run_command() -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        Some(format!("\"{}\" {}", exe.display(), BACKGROUND_ARG))
    }

    /// Run キーに自分が登録されているか (パスが違う古い登録は「無し」とみなす)。
    pub fn autostart_enabled() -> bool {
        unsafe {
            let (key, val) = (w(RUN_KEY), w(RUN_VALUE));
            let mut buf = [0u16; 1024];
            let mut len = (buf.len() * 2) as u32;
            let r = RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            );
            if r != ERROR_SUCCESS {
                return false;
            }
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let cur = String::from_utf16_lossy(&buf[..n]);
            run_command().is_some_and(|c| c.eq_ignore_ascii_case(&cur))
        }
    }

    pub fn set_autostart(on: bool) -> bool {
        unsafe {
            let (key, val) = (w(RUN_KEY), w(RUN_VALUE));
            if on {
                let Some(cmd) = run_command() else {
                    return false;
                };
                let data = w(&cmd);
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    val.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                ) == ERROR_SUCCESS
            } else {
                let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr());
                r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND
            }
        }
    }

    // ------------------------------------------------------------ 窓の表示

    pub fn show_main() {
        let h = MAIN_HWND.load(Ordering::Relaxed) as HWND;
        if h.is_null() {
            return;
        }
        unsafe {
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            } else {
                ShowWindow(h, SW_SHOW);
            }
            SetForegroundWindow(h);
        }
    }

    pub fn hide_main() {
        let h = MAIN_HWND.load(Ordering::Relaxed) as HWND;
        if !h.is_null() {
            unsafe {
                ShowWindow(h, SW_HIDE);
            }
        }
    }

    // ------------------------------------------------------------ トレイ

    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid
        }
    }

    fn add_icon(hwnd: HWND) {
        unsafe {
            let mut nid = icon_data(hwnd);
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            // build.rs が埋め込んだ自分のアイコン (リソース ID 1)。無ければ既定。
            let inst = GetModuleHandleW(null());
            let mut icon = LoadImageW(
                inst,
                1 as _,
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            ) as HICON;
            if icon.is_null() {
                icon = LoadIconW(null_mut(), IDI_APPLICATION);
            }
            nid.hIcon = icon;
            let tip = w("Device HUB");
            nid.szTip[..tip.len()].copy_from_slice(&tip);
            Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }

    fn remove_icon(hwnd: HWND) {
        unsafe {
            let nid = icon_data(hwnd);
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    fn show_menu(hwnd: HWND) {
        unsafe {
            let m = CreatePopupMenu();
            if m.is_null() {
                return;
            }
            let show = w("Device HUB を表示");
            let auto = w("Windows 起動時に自動起動");
            let quit_label = w("終了");
            AppendMenuW(m, MF_STRING, ID_SHOW, show.as_ptr());
            let chk = if autostart_enabled() { MF_CHECKED } else { MF_UNCHECKED };
            AppendMenuW(m, MF_STRING | chk, ID_AUTOSTART, auto.as_ptr());
            AppendMenuW(m, MF_SEPARATOR, 0, null());
            AppendMenuW(m, MF_STRING, ID_QUIT, quit_label.as_ptr());
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            // メニュー外クリックで閉じるために前面化が必要 (KB135788)。
            SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(
                m,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                pt.x,
                pt.y,
                0,
                hwnd,
                null(),
            ) as usize;
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(m);
            match cmd {
                ID_SHOW => show_main(),
                ID_AUTOSTART => {
                    set_autostart(!autostart_enabled());
                }
                ID_QUIT => quit(hwnd),
                _ => {}
            }
        }
    }

    fn quit(hwnd: HWND) {
        QUITTING.store(true, Ordering::Relaxed);
        remove_icon(hwnd);
        let h = MAIN_HWND.load(Ordering::Relaxed) as HWND;
        unsafe {
            if !h.is_null() {
                // GPUI 側の should_close が quitting() を見て閉じる → on_window_closed → quit。
                PostMessageW(h, WM_CLOSE, 0, 0);
            }
            PostQuitMessage(0);
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                match lp as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_main(),
                    WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                    _ => {}
                }
                0
            }
            WM_SHOW_MAIN => {
                show_main();
                0
            }
            _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                // エクスプローラー再起動でトレイが作り直された → 載せ直す。
                add_icon(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    /// トレイを起動する (専用スレッド)。`main_hwnd` は GPUI の主窓。
    pub fn install(main_hwnd: isize) {
        MAIN_HWND.store(main_hwnd, Ordering::Relaxed);
        std::thread::Builder::new()
            .name("tray".into())
            .spawn(|| unsafe {
                let inst = GetModuleHandleW(null());
                let cls = w(CLASS);
                let mut wc: WNDCLASSEXW = std::mem::zeroed();
                wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
                wc.lpfnWndProc = Some(wndproc);
                wc.hInstance = inst;
                wc.lpszClassName = cls.as_ptr();
                RegisterClassExW(&wc);
                let tb = w("TaskbarCreated");
                TASKBAR_CREATED.store(RegisterWindowMessageW(tb.as_ptr()), Ordering::Relaxed);
                // TaskbarCreated はブロードキャストなのでメッセージ専用窓には届かない —
                // 通常の (非表示の) トップレベル窓にする (二重起動側は FindWindowW で探す)。
                let hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    cls.as_ptr(),
                    cls.as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    inst,
                    null(),
                );
                if hwnd.is_null() {
                    return;
                }
                add_icon(hwnd);
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                remove_icon(hwnd);
            })
            .ok();
    }
}

#[cfg(windows)]
pub use imp::{acquire_single_instance, autostart_enabled, hide_main, install, set_autostart, show_main};

#[cfg(not(windows))]
mod stub {
    pub fn acquire_single_instance() -> bool {
        true
    }
    pub fn autostart_enabled() -> bool {
        false
    }
    pub fn set_autostart(_: bool) -> bool {
        false
    }
    pub fn show_main() {}
    pub fn hide_main() {}
    pub fn install(_: isize) {}
}
#[cfg(not(windows))]
pub use stub::*;
