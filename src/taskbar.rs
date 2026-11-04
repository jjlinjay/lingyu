//! 任务栏页：固定应用 + 当前可见窗口，点图标切换或启动。
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Item {
    /// 正在运行的窗口句柄；固定但没运行时为 None。
    pub hwnd: Option<isize>,
    pub title: String,
    pub exe: PathBuf,
    pub pinned: bool,
    pub missing: bool,
}

struct Preview {
    hwnd: isize,
    title: String,
    since: f64,
    seen: f64,
    at: f64,
    texture: Option<TextureHandle>,
}

pub(crate) struct Taskbar {
    pub apps: Vec<Item>,
    pub page: usize,
    pub updated: f64,
    icons: HashMap<String, Option<TextureHandle>>,
    preview: Option<Preview>,
    pub(crate) menu: Option<Item>,
}

impl Default for Taskbar {
    fn default() -> Self {
        Self { apps: Vec::new(), page: 0, updated: -10.0, icons: HashMap::new(), preview: None, menu: None }
    }
}

fn key(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

impl Taskbar {
    pub(crate) fn refresh(&mut self, now: f64, pins: &[String]) {
        self.updated = now;
        let running = list();
        let mut apps: Vec<Item> = Vec::with_capacity(running.len() + pins.len());
        for pin in pins {
            let exe = PathBuf::from(pin);
            let hit = running.iter().find(|app| key(&app.exe) == key(&exe));
            let missing = !exe.exists();
            let title = hit.map(|app| app.title.clone()).unwrap_or_else(|| {
                exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "应用".into())
            });
            apps.push(Item { hwnd: hit.map(|app| app.hwnd), title, exe, pinned: true, missing });
        }
        for app in running {
            if apps.iter().any(|item| key(&item.exe) == key(&app.exe)) {
                continue;
            }
            apps.push(Item { hwnd: Some(app.hwnd), title: app.title, exe: app.exe, pinned: false, missing: false });
        }
        self.apps = apps;
        let keep: Vec<String> = self.apps.iter().map(|app| key(&app.exe)).collect();
        self.icons.retain(|path, _| keep.contains(path));

    }


    pub(crate) fn icon(&mut self, ctx: &egui::Context, app: &Item) -> Option<TextureHandle> {
        let key = key(&app.exe);
        if let Some(icon) = self.icons.get(&key) {
            return icon.clone();
        }
        let cached = cache_path(&app.exe).and_then(|path| load_cache(&path));
        let image = cached.or_else(|| native_icon(&app.exe));
        if let Some(image) = &image {
            if let Some(path) = cache_path(&app.exe) {
                save_cache(&path, image);
            }
        }
        let texture = image.map(|image| ctx.load_texture(key.clone(), image, TextureOptions::LINEAR));
        self.icons.insert(key, texture.clone());
        texture
    }

    /// 预览是否已经过了延迟，用于撑高岛体。
    pub(crate) fn preview_visible(&self, _now: f64) -> bool {
        self.preview.is_some()
    }

    fn clear_preview(&mut self) {
        self.preview = None;
    }
}

/// 每页最多显示的图标数。
pub(crate) const CAPACITY: usize = 7;


/// 展开面板宽度（与 app.rs 的 PW 一致），用于居中。
const PANEL_WIDTH: f32 = 419.0;
/// 单行布局需要的宽度（跟随任务栏页窗口数量自适应）
const ROW_GAP: f32 = 8.0;
/// 主屏幕宽（GetSystemMetrics(SM_CXSCREEN)）
#[cfg(windows)]
fn screen_width() -> f32 {
    (winapi_metrics() as f32).max(1024.0)
}
#[cfg(not(windows))]
fn screen_width() -> f32 {
    1920.0
}
#[cfg(windows)]
fn winapi_metrics() -> i32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};
    unsafe { GetSystemMetrics(SM_CXSCREEN) }
}


impl crate::app::App {
    pub(crate) fn page_taskbar(&mut self, c: &crate::paint::Card) {
        use crate::paint::H;
        use crate::theme::{self, W};
        if self.t - self.taskbar.updated > 2.0 {
            let pins = self.settings.dock_pins.clone();
            self.taskbar.refresh(self.t, &pins);
        }
        let ctx = c.ui.ctx().clone();
        let size = match self.settings.dock_icon { 28 => 28.0, 40 => 40.0, _ => 32.0 };
        let _ = CAPACITY;
        if self.taskbar.apps.is_empty() {
            c.text(210.0, 78.0, "没有可见窗口，去设置里固定几个应用", 11.0, W::Regular, theme::GRAY, H::Center);
            return;
        }
        let apps: Vec<Item> = self.taskbar.apps.clone();
        let gap = ROW_GAP;
        let panel = self.expanded_width();
        let total = apps.len() as f32 * size + (apps.len().saturating_sub(1)) as f32 * gap;
        let left = (panel - total) / 2.0;
        let top = 34.0 + (32.0 - size) * 0.5;
        let mut clicked: Option<(PathBuf, Option<isize>, bool)> = None;
        let mut hovered_title: Option<String> = None;
        if let Some(app) = self.taskbar.menu.clone() {
            let mut rows: Vec<(String, u8)> = Vec::new();
            if app.hwnd.is_some() {
                rows.push(("切换窗口".into(), 1));
                rows.push(("最小化".into(), 2));
                rows.push(("关闭窗口".into(), 3));
            }
            rows.push((if app.pinned { "取消固定" } else { "固定到任务栏" }.into(), if app.pinned { 4 } else { 5 }));
            rows.push(("取消".into(), 0));
            let mut picked: Option<u8> = None;
            for (i, (label, code)) in rows.iter().enumerate() {
                let y = 30.0 + i as f32 * 16.4;
                let hit = c.hit(14.0, y, panel - 28.0, 16.0, ("tb-menu", *code, i));
                let (hover, _) = c.hover_t(&hit);
                if hover > 0.01 {
                    c.fill(14.0, y, panel - 28.0, 16.0, 6.0, Color32::from_white_alpha((hover * 30.0) as u8));
                }
                c.text(26.0, y + 11.5, label, 9.5, W::Regular, theme::WHITE, H::Left);
                if hit.clicked() { picked = Some(*code); }
            }
            if let Some(code) = picked {
                self.taskbar.menu = None;
                match code {
                    1 => if let Some(hwnd) = app.hwnd { activate(hwnd); self.collapse_after_dock(); },
                    2 => if let Some(hwnd) = app.hwnd { minimize(hwnd); self.taskbar.menu = None; },
                    3 => if let Some(hwnd) = app.hwnd { close_window(hwnd); self.taskbar.updated = -10.0; },
                    4 => self.dock_remove(app.exe.clone()),
                    5 => self.dock_add(app.exe.clone()),
                    0 => {}
                    _ => {}
                }
            }
            return;
        }
        for (index, app) in apps.iter().enumerate() {
            let x = left + index as f32 * (size + gap);
            let response = c.hit(x, 28.0, size + gap, 56.0, ("tb", app.hwnd.unwrap_or(-1), index));
            let (hover, _) = c.hover_t(&response);
            c.fill(x, top, size, size, 8.0, Color32::from_white_alpha((8.0 + hover * 26.0) as u8));
            if let Some(texture) = self.taskbar.icon(&ctx, app) {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                c.painter.add(egui::epaint::RectShape::filled(c.rect(x, top, size, size), 8.0 * c.s, Color32::WHITE).with_texture(texture.id(), uv));
            } else {
                let letter = app.exe.file_stem().map(|s| s.to_string_lossy().chars().next().unwrap_or('?').to_ascii_uppercase()).unwrap_or('?');
                c.fill(x, top, size, size, 8.0, Color32::from_rgb(52, 54, 60));
                c.text(x + size / 2.0, top + size / 2.0 + 5.0, &letter.to_string(), 13.0, W::Bold, theme::WHITE, H::Center);
            }
            if app.hwnd.is_none() && !app.missing {
                c.fill(x, top, size, size, 8.0, Color32::from_black_alpha(84));
            }
            if app.missing {
                c.painter.rect_stroke(c.rect(x, top, size, size), 8.0 * c.s, egui::Stroke::new(1.4 * c.s, theme::RED_TOP), egui::StrokeKind::Inside);
            }
            if let Some(hwnd) = app.hwnd {
                let dot = c.rect(x + size / 2.0 - 2.2, top + size + 1.3, 4.4, 4.4);
                c.painter.circle_filled(dot.center(), 2.2 * c.s, if is_minimized(hwnd) { Color32::from_rgb(120, 120, 128) } else { theme::SOFT_GREEN });
            }
            let label = c.fit(&app.title, 8.0, W::Regular, (size + gap).max(38.0));
            c.text(x + size / 2.0, 78.0, &label, 8.0, W::Regular, if app.hwnd.is_some() || app.missing { theme::GRAY } else { theme::DIM }, H::Center);
            if response.hovered() { hovered_title = Some(app.title.clone()); }
            if response.secondary_clicked() { self.taskbar.menu = Some(app.clone()); }
            if response.clicked() {
                if app.missing {
                    self.say("应用路径不存在，去设置 → 页面管理 → 固定 Dock 里移除");
                } else {
                    clicked = Some((app.exe.clone(), app.hwnd, false));
                }
            }
        }
        if let Some(title) = &hovered_title {
            c.text(panel / 2.0, 92.0, &c.fit(title, 9.0, W::Regular, 320.0), 9.0, W::Regular, theme::WHITE, H::Center);
        }
        if let Some((exe, hwnd, _from_preview)) = clicked {
            if let Some(hwnd) = hwnd {
                activate(hwnd);
            } else {
                match std::process::Command::new(&exe).spawn() {
                    Ok(_) => { self.taskbar.updated = -10.0; }
                    Err(e) => self.say(format!("无法启动 {}：{}", exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), e)),
                }
            }
            self.taskbar.clear_preview();
            self.collapse_after_dock();
        }
    }

}

impl crate::taskbar::Taskbar {
    pub(crate) fn row_width(&self, size: f32) -> f32 {
        let n = self.apps.len();
        let w = n as f32 * size + (n.saturating_sub(1)) as f32 * ROW_GAP + 28.0;
        let cap = (screen_width() - 240.0).max(PANEL_WIDTH);
        w.max(PANEL_WIDTH).min(cap)
    }
}

fn fit_rect(area: egui::Rect, size: egui::Vec2) -> egui::Rect {    if size.x <= 0.0 || size.y <= 0.0 { return area; }
    let scale = (area.width() / size.x).min(area.height() / size.y);
    let w = size.x * scale;
    let h = size.y * scale;
    egui::Rect::from_center_size(area.center(), egui::vec2(w, h))
}

impl crate::app::App {
    /// 固定一个应用到任务栏页（重复则忽略）。
    pub fn dock_add(&mut self, path: PathBuf) {
        let text = path.to_string_lossy().into_owned();
        if self.settings.dock_pins.iter().any(|pin| pin.eq_ignore_ascii_case(&text)) {
            self.say("这个应用已经固定在任务栏了");
            return;
        }
        self.settings.dock_pins.push(text);
        self.settings.save();
        self.taskbar.updated = -10.0;
        self.say("已固定到任务栏页");
    }

    /// 从任务栏页取消固定。
    pub fn dock_remove(&mut self, path: PathBuf) {
        let text = path.to_string_lossy().into_owned();
        let before = self.settings.dock_pins.len();
        self.settings.dock_pins.retain(|pin| !pin.eq_ignore_ascii_case(&text));
        if self.settings.dock_pins.len() != before {
            self.settings.save();
            self.taskbar.updated = -10.0;
            self.say("已取消固定");
        }
    }
}

pub(crate) fn activate(hwnd: isize) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU};
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic, SetForegroundWindow, ShowWindow, SwitchToThisWindow, SW_MINIMIZE, SW_RESTORE};
        let window = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);
        if GetForegroundWindow() == window && !IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_MINIMIZE);
            return;
        }
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        if !SetForegroundWindow(window).as_bool() {
            // 某些系统窗口会拒绝后台进程抢前台：先模拟 Alt 解除前台锁，再重试。
            keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            if !SetForegroundWindow(window).as_bool() {
                SwitchToThisWindow(window, true);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = hwnd;
}

#[cfg(windows)]
fn minimize(hwnd: isize) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MINIMIZE};
        let _ = ShowWindow(windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void), SW_MINIMIZE);
    }
}

#[cfg(not(windows))]
fn minimize(_hwnd: isize) {}

#[cfg(windows)]
fn close_window(hwnd: isize) {
    unsafe {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        let _ = PostMessageW(windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

#[cfg(not(windows))]
fn close_window(_hwnd: isize) {}

#[cfg(windows)]
fn is_minimized(hwnd: isize) -> bool {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::IsIconic(windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void)).as_bool()
    }
}

#[cfg(not(windows))]
fn is_minimized(_hwnd: isize) -> bool { false }

#[cfg(windows)]
fn list() -> Vec<Running> {
    use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, MAX_PATH};
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<Running>);
        if !IsWindowVisible(hwnd).as_bool() || GetWindowTextLengthW(hwnd) == 0 {
            return BOOL(1);
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return BOOL(1);
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return BOOL(1);
        };
        let mut buffer = [0u16; MAX_PATH as usize * 2];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        if ok.is_err() {
            return BOOL(1);
        }
        let exe = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
        let name = exe.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if matches!(name.as_str(), "applicationframehost.exe" | "textinputhost.exe" | "shellexperiencehost.exe" | "startmenuexperiencehost.exe" | "searchhost.exe") {
            return BOOL(1);
        }
        let mut title_buffer = vec![0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title_buffer) as usize;
        let title = String::from_utf16_lossy(&title_buffer[..len.min(title_buffer.len())]);
        if !list.iter().any(|app| app.exe == exe) {
            list.push(Running { hwnd: hwnd.0 as isize, title, exe });
        }
        BOOL(1)
    }

    let mut apps: Vec<Running> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut apps as *mut Vec<Running> as isize));
    }
    apps
}

#[cfg(not(windows))]
fn list() -> Vec<Running> {
    Vec::new()
}

#[derive(Clone)]
struct Running {
    hwnd: isize,
    title: String,
    exe: PathBuf,
}

fn cache_path(exe: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let appdata = std::env::var_os("APPDATA")?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    exe.to_string_lossy().to_ascii_lowercase().hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(exe) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified() {
            if let Ok(stamp) = modified.duration_since(std::time::UNIX_EPOCH) {
                stamp.as_secs().hash(&mut hasher);
            }
        }
    }
    let dir = PathBuf::from(appdata).join("LingYu").join("icon-cache");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(format!("{:016x}.png", hasher.finish())))
}

fn load_cache(path: &Path) -> Option<ColorImage> {
    let image = image::open(path).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.pixels().map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
    Some(ColorImage { size, pixels })
}

fn save_cache(path: &Path, image: &ColorImage) {
    let Some(buffer) = image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, image.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), p.a()]).collect()) else { return; };
    let _ = buffer.save(path);
}

#[cfg(windows)]
fn native_icon(path: &std::path::Path) -> Option<ColorImage> {
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HGDIOBJ};
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO, LoadImageW, IMAGE_ICON, LR_LOADFROMFILE, HICON};

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        // 优先按 64px 加载，多数应用带 64/256 图标资源，缩放后比 32px 清楚。
        let mut icon = LoadImageW(None, PCWSTR(wide.as_ptr()), IMAGE_ICON, 64, 64, LR_LOADFROMFILE).map(|handle| HICON(handle.0)).unwrap_or_default();
        if icon.is_invalid() {
            let mut info = SHFILEINFOW::default();
            let result = SHGetFileInfoW(PCWSTR(wide.as_ptr()), Default::default(), Some(&mut info), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_LARGEICON);
            if result == 0 || info.hIcon.is_invalid() {
                return None;
            }
            icon = info.hIcon;
        }
        let mut icon_info = ICONINFO::default();
        if GetIconInfo(icon, &mut icon_info).is_err() {
            let _ = DestroyIcon(icon);
            return None;
        }
        let bitmap = icon_info.hbmColor;
        let mut source = BITMAP::default();
        let got = GetObjectW(HGDIOBJ(bitmap.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut source as *mut BITMAP as *mut c_void));
        let (width, height) = (source.bmWidth.max(1), source.bmHeight.max(1));
        let dc: HDC = GetDC(None);
        let mut header = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let lines = GetDIBits(dc, bitmap, 0, height as u32, Some(pixels.as_mut_ptr() as *mut c_void), &mut header, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        if !icon_info.hbmMask.is_invalid() { let _ = DeleteObject(HGDIOBJ(icon_info.hbmMask.0)); }
        if !icon_info.hbmColor.is_invalid() { let _ = DeleteObject(HGDIOBJ(icon_info.hbmColor.0)); }
        let _ = DestroyIcon(icon);
        if got == 0 || lines == 0 {
            return None;
        }
        let mut image = vec![Color32::TRANSPARENT; (width * height) as usize];
        for i in 0..image.len() {
            let b = pixels[i * 4];
            let g = pixels[i * 4 + 1];
            let r = pixels[i * 4 + 2];
            let a = pixels[i * 4 + 3];
            image[i] = Color32::from_rgba_unmultiplied(r, g, b, if a == 0 { 255 } else { a });
        }
        Some(ColorImage { size: [width as usize, height as usize], pixels: image })
    }
}

#[cfg(not(windows))]
fn native_icon(_path: &std::path::Path) -> Option<ColorImage> {
    None
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: windows::Win32::Foundation::HWND, hdc: windows::Win32::Graphics::Gdi::HDC, flags: u32) -> windows::Win32::Foundation::BOOL;
}

#[cfg(windows)]
fn capture_window(hwnd: isize) -> Option<ColorImage> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, PW_RENDERFULLCONTENT};
    unsafe {
        let window = HWND(hwnd as *mut std::ffi::c_void);
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() { return None; }
        let width = (rect.right - rect.left).clamp(1, 900);
        let height = (rect.bottom - rect.top).clamp(1, 600);
        let screen: HDC = GetDC(None);
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let old = SelectObject(memory, HGDIOBJ(bitmap.0));
        let printed = PrintWindow(window, memory, PW_RENDERFULLCONTENT).as_bool();
        let mut header = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let lines = GetDIBits(memory, bitmap, 0, height as u32, Some(pixels.as_mut_ptr() as *mut c_void), &mut header, DIB_RGB_COLORS);
        SelectObject(memory, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        if !printed || lines == 0 { return None; }
        let mut image = vec![Color32::BLACK; (width * height) as usize];
        let mut nonblack = 0usize;
        for i in 0..image.len() {
            let b = pixels[i * 4];
            let g = pixels[i * 4 + 1];
            let r = pixels[i * 4 + 2];
            if r > 8 || g > 8 || b > 8 { nonblack += 1; }
            image[i] = Color32::from_rgb(r, g, b);
        }
        if nonblack < image.len() / 50 { return None; }
        Some(ColorImage { size: [width as usize, height as usize], pixels: image })
    }
}

#[cfg(not(windows))]
fn capture_window(_hwnd: isize) -> Option<ColorImage> {
    None
}
