use std::thread;

use gtk::glib::{self, Priority};
use std::error::Error;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

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
                let alpha = (radius - distance).max(0.0).min(1.0);
                rgba.extend_from_slice(&[color[0], color[1], color[2], (alpha * 255.0) as u8])
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    Ok(Icon::from_rgba(rgba, size as u32, size as u32)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    gtk::init().expect("Failed to initialize GTK");

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
            std::thread::sleep(std::time::Duration::from_secs(1));
            let status = check_wireguard_status().unwrap_or(ConnectionStatus::Error);
            if sender.send(status).is_err() {
                break;
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
