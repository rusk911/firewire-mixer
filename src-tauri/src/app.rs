use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, WebviewUrl, WindowEvent};

use crate::families::dice_eap::matrix::{MatrixStore, MeterUpdate};
use crate::families::dice_eap::mixer::{self, MixerSessions, RestoreStatus};
use crate::families::DeviceFamily;
use crate::transport::discovery::{self, DeviceInfo};

const TRAY_ID: &str = "firemix-tray";
const DEVICE_EVENT: &str = "devices-changed";

struct AppState<R: Runtime> {
    devices: Mutex<Vec<DeviceInfo>>,
    scan_lock: tokio::sync::Mutex<()>,
    tray_menu: Menu<R>,
    quit_requested: AtomicBool,
    scanner: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl<R: Runtime> AppState<R> {
    fn new(tray_menu: Menu<R>) -> Self {
        Self {
            devices: Mutex::new(Vec::new()),
            scan_lock: tokio::sync::Mutex::new(()),
            tray_menu,
            quit_requested: AtomicBool::new(false),
            scanner: Mutex::new(None),
        }
    }
}

pub fn run<R: Runtime>(builder: tauri::Builder<R>) {
    let app = builder
        .setup(|app| {
            // Tauri's WebKitGTK runtime dispatches the GLib default context on Linux.
            let _main_context = glib::MainContext::default();
            let menu = build_tray_menu(app, &[])?;
            let icon = app.default_window_icon().cloned().ok_or_else(|| {
                std::io::Error::other("FireMix default window icon is not configured")
            })?;

            tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .tooltip("FireMix")
                .menu(&menu)
                .on_menu_event(handle_tray_menu)
                .build(app)?;

            app.manage(AppState::new(menu));
            let matrix_store = Arc::new(MatrixStore::default());
            let event_app = app.handle().clone();
            matrix_store.set_meter_emitter(move |update: MeterUpdate| {
                if let Err(error) = event_app.emit("meter-update", update) {
                    eprintln!("Failed to emit matrix meter update: {error}");
                }
            });
            app.manage(matrix_store);
            app.manage(Arc::new(RestoreStatus::default()));
            app.manage(Arc::new(MixerSessions::default()));
            app.manage(Arc::new(tokio::sync::Mutex::new(())));
            let app_handle = app.handle().clone();
            let scanner = tauri::async_runtime::spawn(async move {
                loop {
                    refresh_devices(&app_handle).await;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            });
            *app.state::<AppState<R>>().scanner.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(scanner);

            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("Failed to initialize FireMix: {error}");
            std::process::exit(1);
        });

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let quit_requested = app
                .try_state::<AppState<R>>()
                .is_some_and(|state| state.quit_requested.load(Ordering::Acquire));
            if !quit_requested {
                api.prevent_exit();
            }
        }
    });
}

async fn refresh_devices<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState<R>>();
    let _scan_guard = state.scan_lock.lock().await;
    if state.quit_requested.load(Ordering::Acquire) {
        return;
    }
    let scan_result = tauri::async_runtime::spawn_blocking(discovery::list_devices).await;
    if state.quit_requested.load(Ordering::Acquire) {
        return;
    }
    let devices = match scan_result {
        Ok(Ok(devices)) => devices,
        Ok(Err(error)) => {
            eprintln!("FireWire scan failed: {error}");
            return;
        }
        Err(error) => {
            eprintln!("FireWire scan task failed: {error}");
            return;
        }
    };

    let previous = state.devices.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    for device in devices.iter().filter(|device| device.family == Some(DeviceFamily::DiceEap)) {
        let already_connected = previous.iter().any(|old| old.node_path == device.node_path && old.guid == device.guid);
        if already_connected {
            continue;
        }
        let node = device.node_path.clone();
        app.state::<Arc<MixerSessions>>().forget(&node);
        let matrix_store = app.state::<Arc<MatrixStore>>().inner().clone();
        let restore_node = node.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            matrix_store.invalidate(&restore_node);
            mixer::restore_configuration(&restore_node, &matrix_store)
        }).await;
        let error = match result {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(format!("Restore task failed: {error}")),
        };
        if let Some(error) = &error {
            eprintln!("DICE configuration restore failed for {node}: {error}");
        }
        app.state::<Arc<RestoreStatus>>().set(&node, error);
    }
    if state.quit_requested.load(Ordering::Acquire) {
        return;
    }

    let previous = {
        let mut current = state
            .devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *current == devices {
            return;
        }
        std::mem::replace(&mut *current, devices.clone())
    };

    if let Err(error) = rebuild_tray_menu(app, &state.tray_menu, &devices) {
        eprintln!("Failed to update FireMix tray menu: {error}");
    }

    for device in previous.iter().filter(|device| device.supported) {
        if !devices.iter().any(|current| current.node_path == device.node_path) {
            app.state::<Arc<MatrixStore>>().invalidate(&device.node_path);
            app.state::<Arc<RestoreStatus>>().set(&device.node_path, None);
            close_mixer_window(app, &device.node_path);
        }
    }

    if let Err(error) = app.emit(DEVICE_EVENT, &devices) {
        eprintln!("Failed to emit device update: {error}");
    }
}

fn build_tray_menu<R: Runtime, M: Manager<R>>(
    manager: &M,
    devices: &[DeviceInfo],
) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(manager)?;
    populate_tray_menu(manager, &menu, devices)?;
    Ok(menu)
}

fn rebuild_tray_menu<R: Runtime, M: Manager<R>>(
    manager: &M,
    menu: &Menu<R>,
    devices: &[DeviceInfo],
) -> tauri::Result<()> {
    for index in (0..menu.items()?.len()).rev() {
        menu.remove_at(index)?;
    }
    populate_tray_menu(manager, menu, devices)
}

fn populate_tray_menu<R: Runtime, M: Manager<R>>(
    manager: &M,
    menu: &Menu<R>,
    devices: &[DeviceInfo],
) -> tauri::Result<()> {
    let supported = devices.iter().filter(|device| device.supported).collect::<Vec<_>>();
    let status = if supported.is_empty() {
        "No FireWire devices found".to_owned()
    } else {
        format!("FireMix - {} device(s) connected", supported.len())
    };
    menu.append(&MenuItem::with_id(
        manager,
        "device-status",
        status,
        false,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(manager)?)?;

    for device in supported {
        let nickname = device.nickname.as_deref().unwrap_or(&device.name);
        let label = if nickname == device.name {
            device.name.clone()
        } else {
            format!("{} - {}", device.name, nickname)
        };
        menu.append(&MenuItem::with_id(
            manager,
            format!("open_device:{}", device.node_path),
            label,
            true,
            None::<&str>,
        )?)?;
    }

    menu.append(&PredefinedMenuItem::separator(manager)?)?;
    menu.append(&MenuItem::with_id(
        manager,
        "rescan",
        "Rescan Now",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        manager,
        "quit",
        "Quit",
        true,
        None::<&str>,
    )?)?;
    Ok(())
}

fn handle_tray_menu<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if id == "rescan" {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            refresh_devices(&app).await;
        });
    } else if id == "quit" {
        if let Some(state) = app.try_state::<AppState<R>>() {
            if state.quit_requested.swap(true, Ordering::AcqRel) {
                return;
            }
            if let Some(scanner) = state.scanner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                scanner.abort();
            }
        }
        if let Some(matrix_store) = app.try_state::<Arc<MatrixStore>>() {
            matrix_store.shutdown();
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let matrix_store = app.state::<Arc<MatrixStore>>().inner().clone();
            let sessions = app.state::<Arc<MixerSessions>>().inner().clone();
            let status = app.state::<Arc<RestoreStatus>>().inner().clone();
            if let Err(error) = tauri::async_runtime::spawn_blocking(move || sessions.flush_all(&matrix_store, &status)).await {
                eprintln!("Mixer save task failed during exit: {error}");
            }
            app.exit(0);
        });
    } else if let Some(node_path) = id.strip_prefix("open_device:") {
        open_mixer_window(app, node_path);
    }
}

fn open_mixer_window<R: Runtime>(app: &AppHandle<R>, node_path: &str) {
    let device = app
        .try_state::<AppState<R>>()
        .and_then(|state| {
            state
                .devices
                .lock()
                .ok()?
                .iter()
                .find(|device| device.node_path == node_path && device.supported)
                .cloned()
        });
    let Some(device) = device else {
        return;
    };
    let Some(label) = window_label(node_path) else {
        return;
    };

    if let Some(window) = app.get_webview_window(&label) {
        if let Err(error) = window.show() {
            eprintln!("Failed to show mixer window {label}: {error}");
        }
        if let Err(error) = window.set_focus() {
            eprintln!("Failed to focus mixer window {label}: {error}");
        }
        return;
    }

    let url = format!("index.html?node={node_path}");
    let nickname = device.nickname.as_deref().unwrap_or(&device.name);
    match WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(format!("FireMix - {nickname}"))
        .inner_size(1100.0, 700.0)
        .min_inner_size(720.0, 520.0)
        .build()
    {
        Ok(window) => {
            let matrix_store = app.state::<Arc<MatrixStore>>().inner().clone();
            let sessions = app.state::<Arc<MixerSessions>>().inner().clone();
            let status = app.state::<Arc<RestoreStatus>>().inner().clone();
            let node_path = node_path.to_owned();
            window.on_window_event(move |event| {
                if matches!(event, WindowEvent::Destroyed) {
                    matrix_store.stop_peak_polling(&node_path);
                    let store = matrix_store.clone();
                    let sessions = sessions.clone();
                    let status = status.clone();
                    let node = node_path.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(error) = sessions.flush(&node, &store, &status) {
                            eprintln!("Could not save mixer for {node} on window close: {error}");
                        }
                    });
                }
            });
        }
        Err(error) => eprintln!("Failed to open mixer window for {node_path}: {error}"),
    }
}

fn close_mixer_window<R: Runtime>(app: &AppHandle<R>, node_path: &str) {
    if let Some(label) = window_label(node_path) {
        if let Some(window) = app.get_webview_window(&label) {
            if let Err(error) = window.close() {
                eprintln!("Failed to close disconnected mixer window {label}: {error}");
            }
        }
    }
}

fn window_label(node_path: &str) -> Option<String> {
    let node = node_path.strip_prefix("/dev/fw")?;
    if node.is_empty() || !node.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("mixer_fw{node}"))
}