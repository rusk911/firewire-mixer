mod app;
pub mod commands;
pub mod families;
pub mod transport;

pub fn register_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::list_devices,
        commands::get_dice_global_state,
        commands::set_dice_nickname,
        commands::set_dice_clock_source,
        commands::set_dice_sample_rate,
        commands::get_matrix_state,
        commands::set_matrix_cell,
        commands::set_matrix_cells,
        commands::get_router_state,
        commands::set_route,
        commands::get_saved_mixer_state,
        commands::save_mixer_configuration,
        commands::stage_mixer_configuration,
        commands::discard_mixer_configuration,
        commands::start_meter_polling,
        commands::stop_meter_polling,
    ])
}

pub fn run() {
    app::run(register_commands(tauri::Builder::default()));
}