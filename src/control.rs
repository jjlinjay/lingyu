//! 控制中心页：Wi-Fi / 蓝牙状态、音量、亮度与系统快捷入口。
use eframe::egui::Color32;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Default)]
pub(crate) struct NetState {
    /// 已连接 Wi-Fi 的名称（SSID）。
    pub wifi: Option<String>,
    /// 蓝牙适配器是否可用。
    pub bluetooth: bool,
    pub at: f64,
}

pub(crate) struct Panel {
    pub net: NetState,
    rx: Option<Receiver<NetState>>,
    asked: f64,
}

impl Default for Panel {
    fn default() -> Self {
        Self { net: NetState { at: -99.0, ..Default::default() }, rx: None, asked: -99.0 }
    }
}

impl Panel {
    pub(crate) fn tick(&mut self, now: f64) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(state) => { self.net = state; self.rx = None; }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => { self.rx = None; }
            }
        }
        if self.rx.is_none() && now - self.asked > 10.0 {
            self.asked = now;
            let (tx, rx) = mpsc::channel();
            self.rx = Some(rx);
            std::thread::spawn(move || {
                let state = probe(now);
                let _ = tx.send(state);
            });
        }
    }

    pub(crate) fn refresh_now(&mut self, now: f64) {
        self.asked = now - 11.0;
    }
}

#[cfg(windows)]
fn probe(now: f64) -> NetState {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = "$ErrorActionPreference='SilentlyContinue';\
[Console]::OutputEncoding=[Text.Encoding]::UTF8;\
netsh wlan show interfaces;\
Write-Output '@@BT';\
if(Get-PnpDevice -Class Bluetooth -Status OK | Select-Object -First 1){Write-Output '1'}else{Write-Output '0'}";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let mut state = NetState { at: now, ..Default::default() };
    let Ok(output) = output else { return state; };
    let text = String::from_utf8_lossy(&output.stdout);
    let (wifi_part, bt_part) = text.split_once("@@BT").unwrap_or((text.as_ref(), ""));
    let mut connected = false;
    for line in wifi_part.lines() {
        let line = line.trim();
        let lower = line.to_ascii_lowercase();
        let is_ssid = (lower.starts_with("ssid") && !lower.starts_with("bssid")) || line.starts_with("SSID");
        if is_ssid {
            if let Some((_, value)) = line.split_once(':') {
                let value = value.trim();
                if !value.is_empty() {
                    state.wifi = Some(value.to_string());
                    connected = true;
                }
            }
        }
        if !connected && (line.contains("已连接") || lower.ends_with("connected") || lower.ends_with(": connected")) {
            connected = true;
        }
    }
    if state.wifi.is_none() && connected {
        state.wifi = Some("已连接".into());
    }
    state.bluetooth = bt_part.lines().any(|line| line.trim() == "1");
    state
}

#[cfg(not(windows))]
fn probe(now: f64) -> NetState {
    NetState { at: now, ..Default::default() }
}

impl crate::app::App {
    pub(crate) fn page_control(&mut self, c: &crate::paint::Card, cmds: &mut Vec<crate::app::Cmd>) {
        use crate::paint::H;
        use crate::theme::{self, W};
        if self.t - self.control.net.at > 10.0 {
            self.control.refresh_now(self.t);
        }
        let net = self.control.net.clone();
        let wifi = net.wifi.clone().unwrap_or_else(|| "未连接".into());
        let bt = if net.bluetooth { "适配器可用" } else { "未检测到" };
        let cards: [(&str, String, &str, Color32); 3] = [
            ("Wi-Fi", wifi, "ms-settings:network-wifi", theme::BLUE),
            ("蓝牙", bt.into(), "ms-settings:bluetooth", Color32::from_rgb(98, 148, 255)),
            ("飞行模式", "点击打开".into(), "ms-settings:network-airplanemode", theme::GRAY),
        ];
        for (index, (title, value, uri, accent)) in cards.iter().enumerate() {
            let x = 22.5 + index as f32 * 127.0;
            let (y, w, h) = (34.0, 120.0, 48.0);
            let hit = c.hit(x, y, w, h, ("ctl-card", index));
            let (hover, pressed) = c.hover_t(&hit);
            c.fill(x, y, w, h, 9.0, Color32::from_white_alpha((7.0 + hover * 12.0 + pressed * 8.0) as u8));
            let dot = c.rect(x + 9.0, y + 13.0, 5.0, 5.0);
            c.painter.circle_filled(dot.center(), 2.4 * c.s, *accent);
            c.text(x + 21.0, y + 18.0, title, 10.0, W::Semibold, theme::GRAY, H::Left);
            let label = c.fit(&value, 10.5, W::Regular, w - 19.0);
            c.text(x + 9.0, y + 37.0, &label, 10.5, W::Regular, theme::WHITE, H::Left);
            if hit.clicked() {
                cmds.push(crate::app::Cmd::Open(std::path::PathBuf::from(*uri)));
            }
        }
        // 音量
        let (volume, muted) = self.volume_ui.unwrap_or((0, false));
        let icon = c.hit(16.0, 96.0, 30.0, 26.0, "ctl-mute");
        let (ih, ip) = c.hover_t(&icon);
        if ih > 0.01 { c.fill(16.0, 96.0, 30.0, 26.0, 8.0, Color32::from_white_alpha((ih * 16.0) as u8)); }
        crate::paint::speaker(&c.scaled(31.0, 109.0, 1.0 - 0.08 * ip), 31.0, 109.0, muted, theme::WHITE);
        if icon.on_hover_text(if muted { "取消静音" } else { "静音" }).clicked() {
            self.volume_ui = Some((volume, !muted));
            self.volume_quiet_until = self.t + 0.9;
            cmds.push(crate::app::Cmd::ToggleMute);
        }
        let (bx, bw, by) = (53.0, 268.0, 109.0);
        let bar = c.hit_drag(bx - 4.0, by - 8.0, bw + 8.0, 16.0, "ctl-vol");
        let (bh, _) = c.hover_t(&bar);
        let th = 4.0 + bh * 1.5;
        let frac = (volume.min(100) as f32 / 100.0).max(0.0);
        c.fill(bx, by - th / 2.0, bw, th, th / 2.0, Color32::from_gray(78));
        if frac > 0.0 { c.fill(bx, by - th / 2.0, (bw * frac).max(th), th, th / 2.0, if muted { Color32::from_gray(130) } else { theme::WHITE }); }
        c.painter.circle_filled(c.rect(bx + bw * frac - 4.0, by - 4.0, 8.0, 8.0).center(), 4.0 * c.s, theme::WHITE);
        if (bar.clicked() || bar.dragged()) && bar.interact_pointer_pos().is_some() {
            if let Some(pointer) = bar.interact_pointer_pos() {
                let value = (((c.to_local(pointer).x - bx) / bw).clamp(0.0, 1.0) * 100.0).round() as u32;
                if self.volume_ui.map(|(v, _)| v) != Some(value) {
                    self.volume_ui = Some((value, false));
                    self.volume_quiet_until = self.t + 0.9;
                    cmds.push(crate::app::Cmd::SetVolume(value));
                }
            }
        }
        c.text(360.0, 113.0, &format!("{}%", volume), 10.0, W::Semibold, theme::GRAY, H::Right);
        // 快捷开关
        let pill = |c: &crate::paint::Card, x: f32, label: &str, on: bool, id: &str| {
            let hit = c.hit(x, 130.0, 92.0, 24.0, id);
            let (hover, _) = c.hover_t(&hit);
            c.fill(x, 130.0, 92.0, 24.0, 12.0, if on { Color32::from_rgb(36, 78, 52) } else { Color32::from_white_alpha((8.0 + hover * 12.0) as u8) });
            if on { c.painter.rect_stroke(c.rect(x, 130.0, 92.0, 24.0), 12.0 * c.s, egui_stroke(1.0, theme::SOFT_GREEN), eframe::egui::StrokeKind::Inside); }
            c.text(x + 46.0, 145.5, label, 10.0, W::Semibold, if on { theme::SOFT_GREEN } else { theme::GRAY }, H::Center);
            hit.clicked()
        };
        if pill(c, 22.0, "提醒声音", self.settings.sound, "ctl-sound") {
            self.settings.sound = !self.settings.sound;
            self.settings.save();
        }
        if pill(c, 122.0, "极简刘海", self.settings.minimal, "ctl-minimal") {
            let next = !self.settings.minimal;
            self.set_minimal(next);
        }
        // 设置按钮
        let open_self = c.hit(22.0, 164.0, 110.0, 26.0, "ctl-open-settings");
        button(c, &open_self, "灵屿设置", theme::WHITE);
        if open_self.clicked() { self.open_settings(); }
        let open_win = c.hit(142.0, 164.0, 116.0, 26.0, "ctl-open-windows");
        button(c, &open_win, "Windows 设置", theme::GRAY);
        if open_win.clicked() { cmds.push(crate::app::Cmd::Open(std::path::PathBuf::from("ms-settings:"))); }
    }
}

fn egui_stroke(width: f32, color: Color32) -> eframe::egui::Stroke {
    eframe::egui::Stroke::new(width, color)
}

fn button(c: &crate::paint::Card, hit: &eframe::egui::Response, label: &str, color: Color32) {
    use crate::paint::H;
    use crate::theme::W;
    let (hover, _) = c.hover_t(hit);
    let rect = hit.rect;
    let local = c.to_local(rect.min);
    let (x, y, w, h) = (local.x, local.y, rect.width() / c.s, rect.height() / c.s);
    c.fill(x, y, w, h, 8.0, Color32::from_white_alpha((9.0 + hover * 12.0) as u8));
    c.text(x + w / 2.0, y + 17.0, label, 10.5, W::Semibold, color, H::Center);
}
