use std::{env, thread};

use gtk::glib::{self, Priority};
use std::error::Error;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

const CHECK_WG_STATUS_S: u64 = 1;
const CHECK_WIN_FOCUS_MS: u64 = 500;

#[derive(Clone)]
enum ConnectionStatus {
    Connected,
    Disconnected,
    Error,
}

impl ConnectionStatus {
    fn as_text(&self) -> &'static str {
        match self {
            ConnectionStatus::Connected => "Подключено",
            ConnectionStatus::Disconnected => "Отключено",
            ConnectionStatus::Error => "Ошибка",
        }
    }

    fn color_rgba(&self) -> [u8; 4] {
        match self {
            ConnectionStatus::Connected => [0, 255, 0, 255],
            ConnectionStatus::Disconnected => [128, 128, 128, 255],
            ConnectionStatus::Error => [255, 0, 0, 255],
        }
    }
}

fn create_colored_icon(status: ConnectionStatus) -> Result<Icon, Box<dyn Error>> {
    let size = 16;
    let mut rgba = Vec::with_capacity(size * size * 4);

    let color = status.color_rgba();

    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - size as f32 / 2.0).abs();
            let dy = (y as f32 - size as f32 / 2.0).abs();

            let distance = (dx * dx + dy * dy).sqrt();
            let radius = size as f32 / 2.0;

            if distance <= radius - 1.0 {
                rgba.extend_from_slice(&color);
            } else if distance <= radius {
                let alpha = (radius - distance).clamp(0.0, 1.0);
                rgba.extend_from_slice(&[color[0], color[1], color[2], (alpha * 255.0) as u8])
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    Ok(Icon::from_rgba(rgba, size as u32, size as u32)?)
}

fn check_wireguard_status() -> Result<ConnectionStatus, Box<dyn Error>> {
    let output = std::process::Command::new("sudo")
        .arg("wg")
        .arg("show")
        .output()?;

    if !output.status.success() {
        return Ok(ConnectionStatus::Error);
    }

    let s = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = s.lines().map(|l| l.trim()).collect();

    let mut has_interface = false;
    let mut has_handshake = false;

    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("interface:") {
            has_interface = true;
            i += 1;
            while i < lines.len()
                && !lines[i].starts_with("interface:")
                && !lines[i].starts_with("peer:")
            {
                i += 1;
            }
        } else if lines[i].starts_with("peer:") {
            i += 1;
            while i < lines.len()
                && !lines[i].starts_with("peer:")
                && !lines[i].starts_with("interface:")
            {
                if lines[i].starts_with("latest handshake:") {
                    has_handshake = true;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    if !has_interface {
        Ok(ConnectionStatus::Disconnected)
    } else if has_handshake {
        Ok(ConnectionStatus::Connected)
    } else {
        Ok(ConnectionStatus::Disconnected)
    }
}

fn get_active_window_class() -> Option<String> {
    let root = std::process::Command::new("xprop")
        .args(["-root", "_NET_ACTIVE_WINDOW"])
        .output()
        .ok()?;

    if !root.status.success() {
        return None;
    }

    let root_str = String::from_utf8_lossy(&root.stdout);
    let id = root_str.split_whitespace().last()?.trim_end_matches(',');

    let class_out = std::process::Command::new("xprop")
        .args(["-id", id, "-notype", "WM_CLASS"])
        .output()
        .ok()?;

    if !class_out.status.success() {
        return None;
    }

    let s = String::from_utf8_lossy(&class_out.stdout);

    if let Some(start) = s.find('"') {
        let rest = &s[start..];
        if let Some(end) = rest.rfind('"') {
            return Some(rest[1..end].to_lowercase());
        }
    }
    None
}

fn run_vpn_command(cmd: &str, wg_config: &str) {
    let _ = std::process::Command::new("sudo")
        .arg("wg-quick")
        .arg(cmd)
        .arg(wg_config)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn is_yandex_focused() -> bool {
    get_active_window_class().is_some_and(|c| c.contains("yandex"))
}

fn main() -> Result<(), Box<dyn Error>> {
    gtk::init().expect("Failed to initialize GTK");

    let wg_config: String = env::var("WG_CONFIG").unwrap_or_default();

    println!("Your wg_config: {:?}", wg_config);

    let tray_menu = Menu::new();

    let status_item = MenuItem::new("Статус: Проверка...", false, None);
    tray_menu.append_items(&[&status_item])?;

    tray_menu.append_items(&[&PredefinedMenuItem::separator()])?;

    let quit_item = MenuItem::new("Выход", true, None);
    tray_menu.append_items(&[&quit_item])?;

    let initial_icon = create_colored_icon(ConnectionStatus::Disconnected)?;
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("WireGuard Monitor")
        .with_icon(initial_icon)
        .build()?;

    let (sender, receiver) = glib::MainContext::channel(Priority::DEFAULT);

    let tray_icon_clone = tray_icon.clone();
    let status_item_clone = status_item.clone();

    receiver.attach(None, move |status: ConnectionStatus| {
        let icon = create_colored_icon(status.clone()).unwrap();
        if let Err(e) = tray_icon_clone.set_icon(Some(icon)) {
            eprintln!("Ошибка обновления иконки: {}", e);
        }
        status_item_clone.set_text(format!("Статус: {}", status.as_text()));
        glib::ControlFlow::Continue
    });

    thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(CHECK_WG_STATUS_S));
            let status = check_wireguard_status().unwrap_or(ConnectionStatus::Error);
            if sender.send(status).is_err() {
                break;
            }
        }
    });

    let wg_conf_clone = wg_config.clone();
    thread::spawn(move || {
        let mut was_yandex = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(CHECK_WIN_FOCUS_MS));
            let now_yandex = is_yandex_focused();

            if now_yandex != was_yandex {
                if now_yandex {
                    run_vpn_command("down", &wg_conf_clone);
                } else {
                    run_vpn_command("up", &wg_conf_clone);
                }
                was_yandex = now_yandex;
            }
        }
    });

    let menu_channel = MenuEvent::receiver();
    let quit_id = quit_item.id().clone();

    thread::spawn(move || {
        loop {
            match menu_channel.recv() {
                Ok(event) => {
                    if event.id == quit_id {
                        println!("Завершение работы");
                        gtk::main_quit();
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Ошибка: {}", e);
                    thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    });

    gtk::main();

    Ok(())
}
