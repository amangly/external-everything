// Copied from Palepale crates/df/src/common/overlay.rs at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
// Adaptations: product branding/storage, capture-visible surfaces, owned teardown.
use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    BOOL, CloseHandle, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, SetWaitableTimerEx,
    TIMER_ALL_ACCESS, WaitForSingleObject,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, LogicalToPhysicalPointForPerMonitorDPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, FindWindowW,
    GWL_EXSTYLE, GWLP_HWNDPARENT, GetClassNameW, GetClientRect, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST,
    IsWindowVisible, LWA_ALPHA, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, SM_CXSCREEN,
    SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    WM_CLOSE, WM_DESTROY, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result as WinResult, w};

static VIEW_W: AtomicI32 = AtomicI32::new(0);
static VIEW_H: AtomicI32 = AtomicI32::new(0);
type PixelBounds = (i32, i32, i32, i32);

fn valid_bounds(b: PixelBounds) -> bool {
    b.2 >= 64 && b.3 >= 64
}

fn bounds_match(a: PixelBounds, b: PixelBounds, tolerance: i32) -> bool {
    (a.0 - b.0).abs() <= tolerance
        && (a.1 - b.1).abs() <= tolerance
        && (a.2 - b.2).abs() <= tolerance
        && (a.3 - b.3).abs() <= tolerance
}

fn own_game(overlay: HWND, game: HWND) {
    unsafe {
        let want = game.0 as isize;
        if want == 0 {
            return;
        }
        let cur = GetWindowLongPtrW(overlay, GWLP_HWNDPARENT);
        if cur != want {
            SetWindowLongPtrW(overlay, GWLP_HWNDPARENT, want);
        }
    }
}

fn choose_game_bounds(
    monitor: Option<PixelBounds>,
    window: Option<PixelBounds>,
    client: Option<PixelBounds>,
) -> Option<PixelBounds> {
    let monitor = monitor.filter(|&b| valid_bounds(b));
    let client = client.filter(|&b| valid_bounds(b));
    match (monitor, client) {
        (Some(mon), Some(cli)) => {
            // A full-monitor window can expose a DPI-virtualized client rect;
            // retain physical monitor pixels there. A genuinely windowed or
            // centered lower-resolution game must use its client viewport.
            if window.is_some_and(|win| bounds_match(win, mon, 3)) {
                Some(mon)
            } else {
                Some(cli)
            }
        }
        (Some(mon), None) => Some(mon),
        (None, Some(cli)) => Some(cli),
        (None, None) => None,
    }
}

fn parse_aspect_ratio(value: &str) -> Option<f64> {
    let value = value.trim();
    let ratio = if let Some((w, h)) = value.split_once([':', '/']) {
        w.trim().parse::<f64>().ok()? / h.trim().parse::<f64>().ok()?
    } else {
        value.parse::<f64>().ok()?
    };
    (ratio.is_finite() && (0.5..=4.0).contains(&ratio)).then_some(ratio)
}

fn customer_aspect_ratio() -> Option<f64> {
    static ASPECT: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    *ASPECT.get_or_init(|| {
        let configured = std::env::var("EXTERNAL_EVERYTHING_OVERLAY_ASPECT_RATIO")
            .ok()
            .or_else(|| {
                let path = super::paths::app_dir()?.join("overlay-aspect.txt");
                std::fs::read_to_string(path).ok()
            });
        let value = configured?;
        if value.trim().is_empty() || value.trim().eq_ignore_ascii_case("auto") {
            return None;
        }
        let aspect = parse_aspect_ratio(&value);
        if aspect.is_none() {
            super::olog::line("invalid overlay aspect ratio; using full game viewport");
        }
        aspect
    })
}

fn fit_aspect(bounds: PixelBounds, aspect: Option<f64>) -> PixelBounds {
    let Some(aspect) = aspect else { return bounds };
    let (x, y, w, h) = bounds;
    if !valid_bounds(bounds) {
        return bounds;
    }
    let (fit_w, fit_h) = if w as f64 / h as f64 > aspect {
        (((h as f64 * aspect).round() as i32).clamp(64, w), h)
    } else {
        (w, ((w as f64 / aspect).round() as i32).clamp(64, h))
    };
    (x + (w - fit_w) / 2, y + (h - fit_h) / 2, fit_w, fit_h)
}

pub fn display_size() -> (i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_CXSCREEN).max(64),
            GetSystemMetrics(SM_CYSCREEN).max(64),
        )
    }
}

fn log_sync_once(game: HWND, monitor: PixelBounds, selected: PixelBounds) {
    use std::sync::atomic::{AtomicIsize, Ordering};
    static LOGGED_FOR: AtomicIsize = AtomicIsize::new(0);
    let key = game.0 as isize;
    if key == 0 || LOGGED_FOR.load(Ordering::Relaxed) == key {
        return;
    }
    LOGGED_FOR.store(key, Ordering::Relaxed);
    let (sm_w, sm_h) = display_size();
    let dpi = unsafe { GetDpiForWindow(game) };
    let mut cr = RECT::default();
    let cr_ok = unsafe { GetClientRect(game, &mut cr) }.is_ok();
    let mut tl = POINT { x: 0, y: 0 };
    if cr_ok {
        unsafe {
            let _ = ClientToScreen(game, &mut tl);
        }
    }
    let msg = format!(
        "sync game={:x} dpi={dpi} monitor=({},{}) {}x{} selected=({},{}) {}x{} SM=({sm_w}x{sm_h}) client={}x{} tl=({},{})",
        key as usize,
        monitor.0,
        monitor.1,
        monitor.2,
        monitor.3,
        selected.0,
        selected.1,
        selected.2,
        selected.3,
        cr.right - cr.left,
        cr.bottom - cr.top,
        tl.x,
        tl.y
    );
    if let Some(dir) = crate::common::paths::app_dir() {
        let path = dir.join("overlay.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let _ = writeln!(f, "{ms} {msg}");
        }
    }
}

// Convert a (logical, virtualized) rect obtained via GetClientRect/ClientToScreen on the target
// window into physical pixels. When the target has a lower DPI awareness than our
// per-monitor-aware-V2 process, Windows returns the target's local coord space; this scales
// it back up to physical using LogicalToPhysicalPointForPerMonitorDPI. Points that already
// come back in physical space (both windows same awareness) are unchanged.
fn physical_rect_from_game(game: HWND, x: i32, y: i32, w: i32, h: i32) -> (i32, i32, i32, i32) {
    unsafe {
        let dpi = GetDpiForWindow(game);
        // 96 == DPI-unaware/SYS-aware reporting untranslated coords. Anything higher and the
        // target already reports physical for its monitor context.
        if dpi != 0 && dpi <= 96 {
            let mut tl = POINT { x, y };
            let mut br = POINT { x: x + w, y: y + h };
            let _ = LogicalToPhysicalPointForPerMonitorDPI(game, &mut tl);
            let _ = LogicalToPhysicalPointForPerMonitorDPI(game, &mut br);
            return (tl.x, tl.y, (br.x - tl.x).max(64), (br.y - tl.y).max(64));
        }
        (x, y, w, h)
    }
}

pub fn view_size() -> (f32, f32) {
    let w = VIEW_W.load(Ordering::Relaxed);
    let h = VIEW_H.load(Ordering::Relaxed);
    if w >= 64 && h >= 64 {
        (w as f32, h as f32)
    } else {
        let (w, h) = display_size();
        (w as f32, h as f32)
    }
}

pub fn view_frame() -> (f32, f32, f32, f32) {
    let (w, h) = view_size();
    let cx = (w as i32 / 2) as f32;
    let cy = (h as i32 / 2) as f32;
    (w, h, cx, cy)
}

/// Query the attached game's physical viewport without creating, moving, or
/// owning an overlay window. A CLI process has no published overlay dimensions.
pub fn diagnostic_view_frame(pid: u32) -> Option<(f32, f32, f32, f32)> {
    let game = find_game(pid)?;
    unsafe {
        let mon = MonitorFromWindow(game, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor = (!mon.is_invalid() && GetMonitorInfoW(mon, &mut mi).as_bool()).then_some((
            mi.rcMonitor.left,
            mi.rcMonitor.top,
            mi.rcMonitor.right - mi.rcMonitor.left,
            mi.rcMonitor.bottom - mi.rcMonitor.top,
        ));
        let mut wr = RECT::default();
        let window = GetWindowRect(game, &mut wr)
            .ok()
            .map(|_| (wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top));
        let mut cr = RECT::default();
        let client = GetClientRect(game, &mut cr).ok().and_then(|_| {
            let mut tl = POINT { x: 0, y: 0 };
            if !ClientToScreen(game, &mut tl).as_bool() {
                return None;
            }
            Some(physical_rect_from_game(
                game,
                tl.x,
                tl.y,
                cr.right - cr.left,
                cr.bottom - cr.top,
            ))
        });
        let (_, _, w, h) = fit_aspect(
            choose_game_bounds(monitor, window, client)?,
            customer_aspect_ratio(),
        );
        Some((w as f32, h as f32, (w / 2) as f32, (h / 2) as f32))
    }
}

fn publish_view(w: i32, h: i32) {
    VIEW_W.store(w, Ordering::Relaxed);
    VIEW_H.store(h, Ordering::Relaxed);
}

pub struct Overlay {
    hwnd: HWND,
    factory: ID2D1Factory,
    write: IDWriteFactory,
    font: IDWriteTextFormat,
    font_hud: IDWriteTextFormat,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    pub w: i32,
    pub h: i32,
    pub cx: f32,
    pub cy: f32,
    game: HWND,
    game_pid: u32,
    refresh_hz: u32,
    refresh_at: Instant,
    refresh_pid: u32,
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CLOSE || msg == WM_DESTROY {
        super::keys::request_quit();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

impl Overlay {
    pub fn new() -> WinResult<Self> {
        Self::create(false)
    }

    /// Ordinary capture-visible overlay surface.
    pub fn new_visible() -> WinResult<Self> {
        Self::create(false)
    }

    fn create(_capture_excluded: bool) -> WinResult<Self> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let class = w!("ExternalEverythingOverlay");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: inst.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let (sw, sh) = display_size();
            publish_view(sw, sh);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT,
                class,
                w!("External Everything Overlay"),
                WS_POPUP,
                0,
                0,
                sw,
                sh,
                None,
                None,
                inst,
                None,
            )?;
            SetLayeredWindowAttributes(
                hwnd,
                windows::Win32::Foundation::COLORREF(0),
                255,
                LWA_ALPHA,
            )?;
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let font = make_font(&write, 15.0)?;
            let font_hud = make_font(&write, 18.0)?;
            Ok(Self {
                hwnd,
                factory,
                write,
                font,
                font_hud,
                rt: None,
                brush: None,
                w: sw,
                h: sh,
                cx: (sw / 2) as f32,
                cy: (sh / 2) as f32,
                game: HWND::default(),
                game_pid: 0,
                refresh_hz: display_hz(),
                refresh_at: Instant::now() - Duration::from_secs(2),
                refresh_pid: 0,
            })
        }
    }

    fn ensure_rt(&mut self) -> WinResult<()> {
        if self.rt.is_some() {
            return Ok(());
        }
        unsafe {
            // D2D identity DPI: 1 DIP = 1 physical pixel on every monitor.
            //
            // D2D's default DPI is the OS DPI (144 at 150% scale), which means
            // DrawLine(1720, 720) gets scaled by dpi/96 and lands at physical
            // (2580, 1080), pushing everything toward the bottom-right. Pinning
            // the render target to D2D's identity DPI (USER_DEFAULT_SCREEN_DPI)
            // makes every draw coord a raw physical pixel, and self.w / self.h
            // already come from MonitorFromWindow so the surface auto-fits the
            // actual monitor - no scale is baked into the code.
            const IDENTITY_DPI: f32 = 96.0; // D2D convention: USER_DEFAULT_SCREEN_DPI
            let rtp = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: IDENTITY_DPI,
                dpiY: IDENTITY_DPI,
                ..Default::default()
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U {
                    width: self.w.max(64) as u32,
                    height: self.h.max(64) as u32,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
            };
            let rt = self.factory.CreateHwndRenderTarget(&rtp, &hp)?;
            rt.SetDpi(IDENTITY_DPI, IDENTITY_DPI);
            let brush = rt.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
            self.brush = Some(brush);
            self.rt = Some(rt);
        }
        Ok(())
    }

    pub fn pump(&self) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, self.hwnd, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn apply_bounds(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let w = w.max(64);
        let h = h.max(64);
        let mut wr = RECT::default();
        let placed = unsafe { GetWindowRect(self.hwnd, &mut wr) }.is_ok();
        let same = placed
            && wr.left == x
            && wr.top == y
            && wr.right - wr.left == w
            && wr.bottom - wr.top == h
            && self.w == w
            && self.h == h;
        if !same {
            unsafe {
                let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
            }
        }
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            if let Some(rt) = &self.rt {
                let size = D2D_SIZE_U {
                    width: w as u32,
                    height: h as u32,
                };
                unsafe {
                    let _ = rt.Resize(&size);
                }
            }
        }
        self.cx = self.w as f32 * 0.5;
        self.cy = self.h as f32 * 0.5;
        publish_view(self.w, self.h);
    }

    pub fn sync(&mut self, pid: u32) {
        if self.refresh_pid != pid || self.refresh_at.elapsed() >= Duration::from_secs(1) {
            self.refresh_hz = display_hz_for_pid(pid);
            self.refresh_at = Instant::now();
            self.refresh_pid = pid;
        }
        let cached = !self.game.is_invalid()
            && self.game_pid == pid
            && unsafe { IsWindowVisible(self.game) }.as_bool();
        let game = if cached {
            Some(self.game)
        } else {
            find_game(pid)
        };
        if let Some(game) = game {
            self.game = game;
            self.game_pid = pid;
            own_game(self.hwnd, game);
            unsafe {
                let mon = MonitorFromWindow(game, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let monitor = if !mon.is_invalid() && GetMonitorInfoW(mon, &mut mi).as_bool() {
                    let mw = mi.rcMonitor.right - mi.rcMonitor.left;
                    let mh = mi.rcMonitor.bottom - mi.rcMonitor.top;
                    if mw >= 64 && mh >= 64 {
                        Some((mi.rcMonitor.left, mi.rcMonitor.top, mw, mh))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mut wr = RECT::default();
                let window = GetWindowRect(game, &mut wr)
                    .ok()
                    .map(|_| (wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top));
                let mut cr = RECT::default();
                let client = if GetClientRect(game, &mut cr).is_ok() {
                    let mut tl = POINT { x: 0, y: 0 };
                    let _ = ClientToScreen(game, &mut tl);
                    let w = cr.right - cr.left;
                    let h = cr.bottom - cr.top;
                    if w >= 64 && h >= 64 {
                        Some(physical_rect_from_game(game, tl.x, tl.y, w, h))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(bounds) = choose_game_bounds(monitor, window, client) {
                    let bounds = fit_aspect(bounds, customer_aspect_ratio());
                    log_sync_once(game, monitor.unwrap_or(bounds), bounds);
                    self.apply_bounds(bounds.0, bounds.1, bounds.2, bounds.3);
                    return;
                }
            }
        }
        let (sw, sh) = display_size();
        self.apply_bounds(0, 0, sw, sh);
    }

    pub fn pulse_top(&self) {
        if !unsafe { IsWindowVisible(self.hwnd) }.as_bool() {
            return;
        }
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    pub fn screen_rect(&self) -> (i32, i32, i32, i32) {
        unsafe {
            let mut wr = RECT::default();
            if GetWindowRect(self.hwnd, &mut wr).is_ok() {
                return (wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top);
            }
        }
        (0, 0, self.w, self.h)
    }

    pub fn refresh_hz(&self) -> u32 {
        self.refresh_hz
    }

    pub fn render_hz(&self) -> u32 {
        self.refresh_hz.min(120)
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn blank_and_hide(&mut self) {
        if self.begin().is_ok() {
            let _ = self.end();
        }
        self.hide();
    }

    pub fn begin(&mut self) -> WinResult<()> {
        self.ensure_rt()?;
        unsafe {
            if let Some(rt) = &self.rt {
                rt.BeginDraw();
                rt.Clear(Some(&D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }));
            }
        }
        Ok(())
    }

    #[allow(clippy::collapsible_if)] // Retain the copied render-target reset path.
    pub fn end(&mut self) -> WinResult<()> {
        unsafe {
            if let Some(rt) = &self.rt {
                if let Err(error) = rt.EndDraw(None, None) {
                    self.rt = None;
                    self.brush = None;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn brush(&self) -> Option<&ID2D1SolidColorBrush> {
        self.brush.as_ref()
    }

    fn set_col(&self, r: f32, g: f32, b: f32, a: f32) {
        if let Some(br) = self.brush() {
            unsafe {
                br.SetColor(&D2D1_COLOR_F { r, g, b, a });
            }
        }
    }

    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32, r: f32, g: f32, b: f32, a: f32, thick: f32) {
        let Some(rt) = &self.rt else { return };
        let Some(br) = self.brush() else { return };
        self.set_col(r, g, b, a);
        unsafe {
            rt.DrawRectangle(
                &D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                },
                br,
                thick,
                None,
            );
        }
    }

    pub fn line(
        &self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
        thick: f32,
    ) {
        let Some(rt) = &self.rt else { return };
        let Some(br) = self.brush() else { return };
        self.set_col(r, g, b, a);
        unsafe {
            rt.DrawLine(
                windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F { x: x1, y: y1 },
                windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F { x: x2, y: y2 },
                br,
                thick,
                None,
            );
        }
    }

    pub fn circle(&self, x: f32, y: f32, rad: f32, r: f32, g: f32, b: f32, a: f32, thick: f32) {
        if rad < 2.0 {
            return;
        }
        let Some(rt) = &self.rt else { return };
        let Some(br) = self.brush() else { return };
        self.set_col(r, g, b, a);
        unsafe {
            rt.DrawEllipse(
                &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                    point: windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F { x, y },
                    radiusX: rad,
                    radiusY: rad,
                },
                br,
                thick,
                None,
            );
        }
    }

    pub fn text(&self, x: f32, y: f32, s: &str, r: f32, g: f32, b: f32, a: f32, hud: bool) {
        let Some(rt) = &self.rt else { return };
        let Some(br) = self.brush() else { return };
        self.set_col(r, g, b, a);
        let wide: Vec<u16> = s.encode_utf16().collect();
        if wide.is_empty() {
            return;
        }
        let font = if hud { &self.font_hud } else { &self.font };
        unsafe {
            rt.DrawText(
                &wide,
                font,
                &D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + 1024.0,
                    bottom: y + if hud { 22.0 } else { 18.0 },
                },
                br,
                windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn fill(&self, x: f32, y: f32, w: f32, h: f32, r: f32, g: f32, b: f32, a: f32) {
        let Some(rt) = &self.rt else { return };
        let Some(br) = self.brush() else { return };
        self.set_col(r, g, b, a);
        unsafe {
            rt.FillRectangle(
                &D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                },
                br,
            );
        }
    }

    pub fn set_click_through(&self, through: bool) {
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let bit = WS_EX_TRANSPARENT.0 as isize;
            let next = if through { style | bit } else { style & !bit };
            if next != style {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next);
            }
        }
    }

    pub fn measure(&self, s: &str, hud: bool) -> f32 {
        let wide: Vec<u16> = s.encode_utf16().collect();
        if wide.is_empty() {
            return 0.0;
        }
        let font = if hud { &self.font_hud } else { &self.font };
        unsafe {
            if let Ok(layout) = self.write.CreateTextLayout(&wide, font, 4000.0, 64.0) {
                let mut m = windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS::default();
                if layout.GetMetrics(&mut m).is_ok() {
                    return m.width;
                }
            }
        }
        s.len() as f32 * 7.0
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.hide();
        self.brush = None;
        self.rt = None;
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

fn make_font(write: &IDWriteFactory, size: f32) -> WinResult<IDWriteTextFormat> {
    unsafe {
        let mut font = write.CreateTextFormat(
            w!("Microsoft YaHei"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("zh-cn"),
        );
        if font.is_err() {
            font = write.CreateTextFormat(
                w!("SimHei"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("zh-cn"),
            );
        }
        let font = font?;
        font.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(font)
    }
}

struct FindGame {
    pid: u32,
    hwnd: HWND,
    area: i32,
}

unsafe extern "system" fn enum_game(h: HWND, lp: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lp.0 as *mut FindGame) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    if pid != ctx.pid {
        return BOOL(1);
    }
    if !unsafe { IsWindowVisible(h) }.as_bool() {
        return BOOL(1);
    }
    let mut r = RECT::default();
    if unsafe { GetClientRect(h, &mut r) }.is_err() || r.right < 200 || r.bottom < 200 {
        return BOOL(1);
    }
    let area = r.right.saturating_mul(r.bottom);
    if area > ctx.area {
        ctx.area = area;
        ctx.hwnd = h;
    }
    BOOL(1)
}

fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

fn window_class(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

fn is_apex_title(title: &str) -> bool {
    let t = title.to_ascii_lowercase();
    t.contains("apex legends") || t == "apex"
}

fn is_fn_title(title: &str) -> bool {
    title.to_ascii_lowercase().contains("fortnite")
}

fn is_endfield_title(title: &str) -> bool {
    let t = title.to_ascii_lowercase();
    t.contains("endfield") || t.contains("arknights")
}

fn is_cs2_title(title: &str) -> bool {
    let t = title.to_ascii_lowercase();
    t.contains("counter-strike 2") || t == "cs2"
}

fn is_pubg_title(title: &str) -> bool {
    let t = title.to_ascii_lowercase();
    t.contains("pubg") || t.contains("playerunknown")
}

fn is_val_title(title: &str) -> bool {
    let t = title.to_ascii_lowercase();
    t.contains("valorant") && !t.contains("riot")
}

fn is_val_window(class: &str, title: &str) -> bool {
    if is_val_title(title) {
        return true;
    }
    if class == "UnrealWindow" {
        let t = title.trim().to_ascii_lowercase();
        return t == "valorant" || is_val_title(&t);
    }
    false
}

unsafe extern "system" fn enum_title(h: HWND, lp: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lp.0 as *mut FindGame) };
    if !unsafe { IsWindowVisible(h) }.as_bool() {
        return BOOL(1);
    }
    let title = window_title(h);
    let class = window_class(h);
    if !is_apex_title(&title)
        && !is_fn_title(&title)
        && !is_val_window(&class, &title)
        && !is_endfield_title(&title)
        && !is_pubg_title(&title)
        && !is_cs2_title(&title)
    {
        return BOOL(1);
    }
    let mut r = RECT::default();
    if unsafe { GetClientRect(h, &mut r) }.is_err() || r.right < 200 || r.bottom < 200 {
        return BOOL(1);
    }
    let area = r.right.saturating_mul(r.bottom);
    if area > ctx.area {
        ctx.area = area;
        ctx.hwnd = h;
    }
    BOOL(1)
}

fn find_game(pid: u32) -> Option<HWND> {
    // PointBlank supplies an explicit PID. An idle launcher stays on the
    // desktop instead of attaching a different game's title fallback.
    if pid == 0 {
        return None;
    }
    let mut ctx = FindGame {
        pid,
        hwnd: HWND::default(),
        area: 0,
    };
    if pid != 0 {
        unsafe {
            let _ = EnumWindows(Some(enum_game), LPARAM(&mut ctx as *mut _ as isize));
        }
        if !ctx.hwnd.is_invalid() {
            return Some(ctx.hwnd);
        }
        // Attached pid has no usable window yet. Do not grab another title
        // (a stale PUBG window used to win this fallback while CS2 was up).
        return None;
    }
    ctx.area = 0;
    ctx.hwnd = HWND::default();
    unsafe {
        let _ = EnumWindows(Some(enum_title), LPARAM(&mut ctx as *mut _ as isize));
    }
    if !ctx.hwnd.is_invalid() {
        return Some(ctx.hwnd);
    }
    unsafe { FindWindowW(None, w!("Apex Legends")) }
        .ok()
        .or_else(|| unsafe { FindWindowW(None, w!("Fortnite")) }.ok())
        .or_else(|| unsafe { FindWindowW(None, w!("PUBG: BATTLEGROUNDS")) }.ok())
        .or_else(|| unsafe { FindWindowW(w!("UnrealWindow"), w!("VALORANT  ")) }.ok())
        .or_else(|| unsafe { FindWindowW(None, w!("VALORANT  ")) }.ok())
        .or_else(|| unsafe { FindWindowW(None, w!("VALORANT")) }.ok())
        .or_else(|| unsafe { FindWindowW(w!("UnrealWindow"), None) }.ok())
}

pub fn display_hz() -> u32 {
    unsafe {
        let mut dm = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if EnumDisplaySettingsW(None, ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
            let hz = dm.dmDisplayFrequency;
            if (48..=500).contains(&hz) {
                return hz;
            }
        }
    }
    165
}

fn display_hz_for_pid(pid: u32) -> u32 {
    let Some(game) = (pid != 0).then(|| find_game(pid)).flatten() else {
        return display_hz();
    };
    unsafe {
        let monitor = MonitorFromWindow(game, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !monitor.is_invalid()
            && GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO).as_bool()
        {
            let mut dm = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            if EnumDisplaySettingsW(
                PCWSTR(info.szDevice.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut dm,
            )
            .as_bool()
                && (48..=500).contains(&dm.dmDisplayFrequency)
            {
                return dm.dmDisplayFrequency;
            }
        }
    }
    display_hz()
}

struct FramePacer {
    next: Option<Instant>,
    timer: Option<HANDLE>,
}

impl FramePacer {
    fn new() -> Self {
        let timer = unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
            .ok()
        };
        Self { next: None, timer }
    }

    fn wait(&mut self, hz: u32) {
        let step = Duration::from_nanos(1_000_000_000 / hz.max(1) as u64);
        let deadline = self.next.unwrap_or_else(Instant::now);
        if let Some(delay) = deadline.checked_duration_since(Instant::now()) {
            let mut waited = false;
            if let Some(timer) = self.timer {
                let ticks = (delay.as_nanos() / 100).min(i64::MAX as u128) as i64;
                if ticks > 0 {
                    let due = -ticks;
                    if unsafe { SetWaitableTimerEx(timer, &due, 0, None, None, None, 0) }.is_ok() {
                        let timeout = u32::try_from(delay.as_millis())
                            .unwrap_or(u32::MAX - 20)
                            .saturating_add(20);
                        if unsafe { WaitForSingleObject(timer, timeout) } != WAIT_OBJECT_0 {
                            self.timer = None;
                            let _ = unsafe { CloseHandle(timer) };
                        }
                        waited = true;
                    }
                }
            }
            if !waited {
                std::thread::sleep(delay);
            }
        }
        let now = Instant::now();
        let next = deadline + step;
        self.next = Some(if now > next + step { now } else { next });
    }
}

impl Drop for FramePacer {
    fn drop(&mut self) {
        if let Some(timer) = self.timer {
            let _ = unsafe { CloseHandle(timer) };
        }
    }
}

pub fn cap_hz(hz: u32) {
    thread_local! {
        static PACER: RefCell<FramePacer> = RefCell::new(FramePacer::new());
    }
    PACER.with(|pacer| pacer.borrow_mut().wait(hz));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_size_live() {
        let (w, h) = display_size();
        assert!(w >= 64 && h >= 64, "{w}x{h}");
    }

    #[test]
    fn frame_pacer_does_not_spin_above_target_rate() {
        let mut pacer = FramePacer::new();
        let started = Instant::now();
        for _ in 0..6 {
            pacer.wait(120);
        }
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn windowed_game_uses_client_viewport() {
        let monitor = (0, 0, 5120, 1440);
        let window = (632, -8, 3856, 1119);
        let client = (640, 0, 3840, 1080);
        assert_eq!(
            choose_game_bounds(Some(monitor), Some(window), Some(client)),
            Some(client)
        );
    }

    #[test]
    fn aspect_override_fits_the_active_viewport() {
        assert_eq!(
            fit_aspect((0, 0, 3440, 1440), Some(16.0 / 9.0)),
            (440, 0, 2560, 1440)
        );
        assert_eq!(
            fit_aspect((-1920, 120, 1920, 1080), Some(4.0 / 3.0)),
            (-1680, 120, 1440, 1080)
        );
        assert_eq!(parse_aspect_ratio("16:9"), Some(16.0 / 9.0));
        assert_eq!(parse_aspect_ratio("100:1"), None);
        assert_eq!(parse_aspect_ratio("auto"), None);
    }

    #[test]
    fn full_monitor_window_rejects_virtualized_client() {
        let monitor = (0, 0, 5120, 1440);
        let client = (0, 0, 3840, 1080);
        assert_eq!(
            choose_game_bounds(Some(monitor), Some(monitor), Some(client)),
            Some(monitor)
        );
    }

    #[test]
    fn valorant_title_not_riot_client() {
        assert!(is_val_title("VALORANT"));
        assert!(is_val_title("VALORANT  "));
        assert!(!is_val_title("Riot Client"));
        assert!(!is_val_title("Apex Legends"));
        assert!(!is_val_title("Fortnite"));
        assert!(is_pubg_title("PUBG: BATTLEGROUNDS"));
        assert!(is_pubg_title("PLAYERUNKNOWN'S BATTLEGROUNDS"));
        assert!(!is_pubg_title("Fortnite"));
    }

    #[test]
    fn endfield_title_matches_client() {
        assert!(is_endfield_title("Endfield"));
        assert!(is_endfield_title("Arknights: Endfield"));
        assert!(!is_endfield_title("Apex Legends"));
    }
}
