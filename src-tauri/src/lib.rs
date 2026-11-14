//! The Tauri adapter: commands, events, plugins, logging and media server wiring. A command parses its input,
//! calls a crate and maps the error; no business rule lives here.

mod agents;
pub mod commands;
pub mod error;
pub mod events;
mod logging;
mod system;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jutsu_app::connection::{CallbackPage, HiggsfieldConnectionServices, KeyringVault};
use jutsu_app::queue::RouteConnectivity;
use jutsu_app::{AppDirectories, AppEnvironment, HiggsfieldProviderFactory, MediaServer, PlatformServices};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_opener::OpenerExt;
use tauri_specta::{Builder, collect_commands, collect_events};
use time::UtcOffset;

/// The typed IPC surface: every command, event and type the TypeScript bindings export.
#[allow(clippy::too_many_lines, reason = "one line per command")]
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::app_version,
            commands::connection::connection_status,
            commands::connection::connection_details,
            commands::connection::connect_api_key,
            commands::connection::start_sign_in,
            commands::connection::cancel_sign_in,
            commands::connection::open_sign_in_page,
            commands::connection::open_higgsfield_console,
            commands::connection::sign_out,
            commands::connection::dismiss_connection_alert,
            commands::preferences::get_preferences,
            commands::preferences::set_preferences,
            commands::media::media_url,
            commands::queue::queue_snapshot,
            commands::queue::queue_job_command,
            commands::queue::queue_pause_all,
            commands::queue::queue_resume_all,
            commands::queue::queue_remove_problem,
            commands::queue::queue_clear_problems,
            commands::queue::queue_duplicate,
            commands::queue::queue_show_in_folder,
            commands::queue::queue_progress,
            commands::queue::queue_cancel_all,
            commands::settings::general_settings,
            commands::settings::set_open_at_login,
            commands::settings::clear_prompt_history,
            commands::settings::check_for_updates,
            commands::settings::output_settings,
            commands::settings::output_name_preview,
            commands::settings::choose_output_folder,
            commands::settings::reset_output_folder,
            commands::settings::show_output_folder,
            commands::settings::storage_summary,
            commands::settings::old_drafts,
            commands::settings::trash_old_drafts,
            commands::settings::clear_cache,
            commands::settings::notification_settings,
            commands::settings::notifications_blocked,
            commands::settings::notifications_open_system_settings,
            commands::settings::shortcut_settings,
            commands::settings::shortcut_bindings,
            commands::settings::record_shortcut,
            commands::settings::reset_shortcut,
            commands::settings::restore_default_shortcuts,
            commands::subtitles::subtitle_settings,
            commands::subtitles::subtitles_sheet,
            commands::subtitles::subtitles_start,
            commands::subtitles::subtitles_cancel,
            commands::subtitles::subtitles_runs,
            commands::subtitles::subtitles_take_failure,
            commands::subtitles::subtitles_editor,
            commands::subtitles::subtitles_edit,
            commands::subtitles::subtitles_save,
            commands::subtitles::subtitles_export,
            commands::subtitles::subtitles_set_default_engine,
            commands::subtitles::subtitles_download_model,
            commands::subtitles::subtitles_cancel_download,
            commands::subtitles::subtitles_remove_model,
            commands::create::create_snapshot,
            commands::create::create_edit,
            commands::create::create_change_roles,
            commands::create::create_choose_source,
            commands::create::create_open_source,
            commands::create::create_drop_files,
            commands::create::create_choose_references,
            commands::create::create_choose_frame,
            commands::create::create_paste_image,
            commands::create::create_submit,
            commands::create::create_load_job,
            commands::create::create_open_follow_up,
            commands::create::create_undo_replacement,
            commands::create::create_render_final,
            commands::create::create_replace_prompt,
            commands::create::create_refresh_presets,
            commands::create::create_preset_query,
            commands::create::create_toggle_favorite_preset,
            commands::create::create_motion_library,
            commands::create::create_load_motion_library,
            commands::create::create_use_motion_clip,
            commands::create::create_choose_character_set,
            commands::create::create_add_character_set,
            commands::create::create_save_character_set,
            commands::create::create_higgsfield_references,
            commands::create::create_load_higgsfield,
            commands::create::create_set_element_category,
            commands::create::create_close_higgsfield,
            commands::create::create_add_uploads,
            commands::create::create_open_element,
            commands::create::create_close_element,
            commands::create::create_add_element_images,
            commands::create::create_link_entry,
            commands::create::create_download_link,
            commands::create::create_cancel_link,
            commands::create::create_choose_speech,
            commands::create::create_add_speech,
            commands::images::images_snapshot,
            commands::images::images_edit,
            commands::images::images_set_side,
            commands::images::images_choose_input,
            commands::images::images_generate,
            commands::images::images_cancel,
            commands::images::images_show,
            commands::images::images_delete,
            commands::images::images_clear,
            commands::images::images_retry_saving,
            commands::images::images_dismiss_save_failure,
            commands::characters::characters_snapshot,
            commands::characters::characters_edit,
            commands::characters::characters_choose_images,
            commands::characters::characters_load_more_presets,
            commands::characters::characters_generate,
            commands::characters::create_add_character,
            commands::characters::images_reference_picks,
            commands::characters::images_toggle_reference_pick,
            commands::characters::images_clear_reference_picks,
            commands::characters::create_add_generated,
            commands::images::templates_list,
            commands::images::templates_suggested_name,
            commands::images::templates_save,
            commands::images::templates_rename,
            commands::images::templates_delete,
            commands::images::templates_apply,
            commands::images::prompts_history,
            commands::images::prompts_set_starred,
            commands::library::library_snapshot,
            commands::library::library_counts,
            commands::library::library_thumbnail,
            commands::library::library_detail,
            commands::library::library_image_details,
            commands::library::library_delete_confirmation,
            commands::library::library_set_starred,
            commands::library::library_delete,
            commands::library::library_remove,
            commands::library::library_cull,
            commands::library::library_undo_culling,
            commands::library::library_make_primary,
            commands::library::library_load_history,
            commands::library::library_download,
            commands::library::library_show_in_folder,
            commands::library::library_open,
            commands::library::library_save_inputs,
            commands::library::library_reveal_input,
            commands::library::library_compare,
            commands::library::library_compare_original,
            commands::library::library_compare_candidates,
            commands::library::library_compare_pick,
            commands::library::library_image_compare,
            commands::library::library_image_compare_original,
            commands::library::library_image_variants,
            commands::library::library_image_compare_sides,
            commands::library::library_copy_image,
            commands::library::library_removal_title,
            commands::library::library_template_name,
            commands::library::library_save_template,
            commands::library::library_batch,
            commands::library::library_score_sheet,
            commands::library::library_score,
            commands::library::library_final_section,
            commands::library::library_render_final,
            commands::library::library_finals_sheet,
            commands::library::library_render_finals,
            commands::library::library_join_with_original,
            commands::library::library_export_short_preview,
            commands::library::library_files,
            commands::library::library_edit_in,
            commands::library::library_import_version,
            commands::enhance::enhance_choice,
            commands::enhance::enhance_sheet,
            commands::enhance::enhance_queue,
            commands::enhance::enhance_voices,
            commands::enhance::editing_export_menu,
            commands::enhance::editing_export,
            commands::enhance::editing_export_cancel,
            commands::on_device::on_device_sheet,
            commands::on_device::on_device_start,
            commands::on_device::on_device_cancel,
            commands::on_device::on_device_runs,
            commands::on_device::on_device_offers_image_upscale,
            commands::delivery::delivery_sets,
            commands::delivery::delivery_set,
            commands::delivery::delivery_new,
            commands::delivery::delivery_change,
            commands::delivery::delivery_next_round,
            commands::delivery::delivery_delete,
            commands::delivery::delivery_frame,
            commands::delivery::delivery_export,
            commands::delivery::delivery_cancel,
            commands::delivery::delivery_show_folder,
            commands::delivery::delivery_crop_rect,
            commands::spending::spending_view,
            commands::spending::spending_set_cap_enabled,
            commands::spending::spending_commit_cap,
            commands::spending::spending_commit_low_credits,
            commands::spending::spending_export_csv,
            commands::spending::credit_history,
            commands::spending::credit_history_more,
            commands::spending::credit_history_retry,
            commands::library::library_make_video,
            commands::library::library_bulk,
            commands::library::library_toggle_star,
            commands::agents::agents_settings,
            commands::agents::agents_set_allowed,
            commands::agents::agents_set_allows_unlimited,
            commands::agents::agents_commit_budget,
            commands::agents::agents_send_held_image,
            commands::agents::agents_set_window_sheet,
            commands::settings::install_update,
            commands::settings::set_update_switch,
            commands::system::tray_panel,
            commands::system::tray_open,
            commands::system::tray_fit,
            commands::system::tray_close,
            commands::system::tray_cancel,
            commands::system::tray_reveal,
            commands::system::tray_agents,
            commands::system::tray_quit,
            commands::shell::app_menu_publish,
            commands::shell::about_content,
            commands::shell::about_open_repository,
            commands::shell::sidebar_layout,
            commands::shell::set_sidebar_layout,
            commands::shell::opened_files_choose,
            commands::shell::opened_files_take,
        ])
        .events(collect_events![
            events::ConnectionChanged,
            events::PreferencesChanged,
            events::QueueChanged,
            events::CreateChanged,
            events::LibraryChanged,
            events::ImagesChanged,
            events::OnDeviceRunsChanged,
            events::SubtitlesChanged,
            events::AgentShowRequested,
            events::MenuCommandRequested,
            events::FilesOpened
        ])
        .typ::<error::AppError>()
}

/// Where results go: `<Videos>/Jutsu Studio`, and a folder of its own for debug builds.
const OUTPUT_FOLDER: &str = if cfg!(debug_assertions) {
    "Jutsu Studio Desktop Debug"
} else {
    "Jutsu Studio"
};

/// The window was closed while jobs ran: it is hidden, and the app quits once the queue is idle unless the window
/// is opened again.
static HIDDEN_FOR_JOBS: AtomicBool = AtomicBool::new(false);
/// The engine has stopped; the next exit request goes through.
static STOPPED: AtomicBool = AtomicBool::new(false);

/// Clears the hidden-for-jobs mark; `true` when it was set.
pub(crate) fn take_hidden_for_jobs() -> bool {
    HIDDEN_FOR_JOBS.swap(false, Ordering::SeqCst)
}

/// Starts the app and blocks until it quits.
///
/// Debug builds first write the TypeScript bindings to `src/bindings.ts`.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Read before any other thread starts: on Unix the offset can only be read safely while the process is
    // single-threaded. It decides where "today" starts for the spend cap, and stays fixed for this launch.
    let local_offset = UtcOffset::current_local_offset();
    let builder = specta_builder();

    #[cfg(debug_assertions)]
    builder.export(
        specta_typescript::Typescript::default(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts"),
    )?;

    tauri::Builder::default()
        // First, so a second launch hands over to the running app before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, arguments, directory| {
            bring_to_front(app);
            // Files the second launch was asked to open go to Create here.
            system::opened::open_arguments(app, &arguments, std::path::Path::new(&directory));
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[system::panel::LABEL, system::about::LABEL])
                // The main window is restored in `setup`, where a position off every display is caught.
                .skip_initial_state(system::window::MAIN)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args([jutsu_app::system::background::LOGIN_LAUNCH_ARGUMENT])
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            setup(app.handle(), local_offset)?;
            Ok(())
        })
        .build(tauri::generate_context!())?
        .run(on_run_event);
    Ok(())
}

/// Closing the window while jobs run hides it instead; quitting first stops the engine, giving submits already sent
/// up to 5 s to be answered.
fn on_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            if system::window::close_main(app) {
                api.prevent_close();
            }
        }
        RunEvent::ExitRequested { api, code, .. } => {
            if STOPPED.load(Ordering::SeqCst) {
                return;
            }
            let Some(environment) = app
                .try_state::<Arc<AppEnvironment>>()
                .map(|state| state.inner().clone())
            else {
                return;
            };
            api.prevent_exit();
            if let Some(runtime) = app.try_state::<Arc<jutsu_app::agents::runtime::AgentRuntime>>() {
                runtime.stop();
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                environment.stop().await;
                system::updater::install_downloaded(&app);
                STOPPED.store(true, Ordering::SeqCst);
                app.exit(code.unwrap_or(0));
            });
        }
        _ => {}
    }
}

fn setup(
    app: &AppHandle,
    local_offset: Result<UtcOffset, time::error::IndeterminateOffset>,
) -> Result<(), Box<dyn std::error::Error>> {
    let paths = app.path();
    logging::start(&paths.app_log_dir()?);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Jutsu Studio is starting");
    let utc_offset = local_offset.unwrap_or_else(|error| {
        tracing::warn!(%error, "can't read the local time zone; today starts at midnight UTC");
        UtcOffset::UTC
    });

    // Debug builds keep their own data, as they keep their own keychain item.
    let data_dir = if cfg!(debug_assertions) {
        paths.app_data_dir()?.join("Debug")
    } else {
        paths.app_data_dir()?
    };
    let directories = AppDirectories {
        data_dir,
        default_output_dir: paths.video_dir()?.join(OUTPUT_FOLDER),
    };

    let environment = tauri::async_runtime::block_on(AppEnvironment::open(
        directories,
        platform_services(app, utc_offset),
    ))?;
    let media = tauri::async_runtime::block_on(MediaServer::start(&environment.media_roots()))?;
    tracing::info!(origin = %media.origin(), "the media server is listening");

    events::forward(app, &environment, &media);
    app.manage(environment.clone());
    app.manage(media);
    agents::start(app, environment.clone());
    system::start(app, &environment);
    // "Open with" starts the app with the files as its arguments; they wait for Create.
    let arguments: Vec<String> = std::env::args().collect();
    system::opened::open_arguments(app, &arguments, &std::env::current_dir().unwrap_or_default());
    // The vault is read on a blocking task: until then the connection is restoring and the window shows neither
    // onboarding nor the connection. The engine starts after it.
    tauri::async_runtime::spawn(async move { environment.start().await });
    Ok(())
}

fn platform_services(app: &AppHandle, utc_offset: UtcOffset) -> PlatformServices {
    let opener = app.clone();
    let front = app.clone();
    let page = CallbackPage {
        app_url: None,
        icon_png: Some(include_bytes!("../icons/128x128.png").to_vec()),
    };
    PlatformServices {
        vault: Arc::new(KeyringVault::for_this_build()),
        connection: Arc::new(HiggsfieldConnectionServices::new(
            Arc::new(move |url| {
                opener
                    .opener()
                    .open_url(url.as_str(), None::<&str>)
                    .map_err(|error| error.to_string())
            }),
            Arc::new(move || bring_to_front(&front)),
            page,
        )),
        providers: Arc::new(HiggsfieldProviderFactory::default()),
        connectivity: Arc::new(RouteConnectivity::default()),
        media: None,
        utc_offset,
    }
}

/// Shows, restores and focuses the main window: on a second launch, and when the browser hands sign-in back. A
/// window hidden while jobs ran is open again, so the app no longer quits when they finish.
fn bring_to_front(app: &AppHandle) {
    HIDDEN_FOR_JOBS.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    /// Writes `src/bindings.ts` without starting the app: `cargo test -p jutsu-studio -- --ignored`.
    #[test]
    #[ignore = "writes src/bindings.ts"]
    fn export_bindings() {
        super::specta_builder()
            .export(
                specta_typescript::Typescript::default(),
                concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts"),
            )
            .unwrap();
    }

    #[test]
    fn bindings_export_the_commands_events_and_the_error_type() {
        let path = std::env::temp_dir().join(format!("jutsu-bindings-{}.ts", std::process::id()));
        super::specta_builder()
            .export(specta_typescript::Typescript::default(), &path)
            .unwrap();
        let bindings = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        for name in [
            "appVersion",
            "connectionStatus",
            "connectApiKey",
            "startSignIn",
            "signOut",
            "getPreferences",
            "setPreferences",
            "mediaUrl",
            "queueSnapshot",
            "queueJobCommand",
            "queuePauseAll",
            "queueDuplicate",
            "queueProgress",
            "queueChanged",
            "export type QueueSnapshot",
            "generalSettings",
            "outputSettings",
            "recordShortcut",
            "shortcutBindings",
            "subtitleSettings",
            "connectionChanged",
            "preferencesChanged",
            "export type AppError",
            "export type ConnectionSnapshot",
            "createSnapshot",
            "createEdit",
            "createSubmit",
            "createLoadJob",
            "createOpenFollowUp",
            "createChanged",
            "export type CreateSnapshot",
            "librarySnapshot",
            "libraryDetail",
            "onDeviceSheet",
            "onDeviceStart",
            "onDeviceRuns",
            "onDeviceRunsChanged",
            "export type OnDeviceSheet",
            "libraryCull",
            "libraryLoadHistory",
            "libraryChanged",
            "export type LibrarySnapshot",
            "export type ResultDetail",
            "imagesSnapshot",
            "imagesGenerate",
            "imagesChanged",
            "export type ImageSnapshot",
            "templatesSave",
            "templatesApply",
            "promptsHistory",
            "queueCancelAll",
            "createReplacePrompt",
            "enhanceSheet",
            "enhanceQueue",
            "editingExport",
            "deliveryExport",
            "deliveryChange",
            "spendingView",
            "creditHistory",
            "trayPanel",
            "trayOpen",
            "export type TrayPanel",
            "setUpdateSwitch",
            "installUpdate",
        ] {
            assert!(bindings.contains(name), "{name}");
        }
    }
}
