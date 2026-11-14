//! Job notifications (`JobNotifier`): posts what `jutsu_app::system::notifications` decides, with its buttons, and
//! answers a click or a button.

use std::collections::HashMap;

use jutsu_app::system::notifications::{
    JobNotification, NotificationCategory, NotificationEffect, NotificationResponse,
};
use tauri::AppHandle;
use tauri_plugin_notification::{Action, ActionType, NotificationExt as _};
use tauri_plugin_opener::OpenerExt as _;

use super::{environment, window};
use jutsu_app::system::tray::TrayOpen;

/// Registers the notifications' buttons and listens for what the person does with them.
pub(crate) fn register(app: &AppHandle) {
    let notification = app.notification();
    let types = NotificationCategory::ALL
        .into_iter()
        .map(|category| {
            ActionType::builder(category.id())
                .actions(
                    category
                        .actions()
                        .iter()
                        .map(|action| Action::builder(action.id(), action.title()).build())
                        .collect(),
                )
                .build()
        })
        .collect();
    if let Err(error) = notification.register_action_types(types) {
        tracing::error!(%error, "can't register the notifications' buttons");
    }
    let handle = app.clone();
    let listening = notification.on_action(move |performed| {
        let Some(posted) = performed.notification() else {
            return;
        };
        let user_info: HashMap<String, String> = posted
            .extra()
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
            .collect();
        if let Some(response) = NotificationResponse::new(&user_info, performed.action_id()) {
            respond(&handle, &response);
        }
    });
    if let Err(error) = listening {
        tracing::error!(%error, "can't follow clicks on notifications");
    }
}

/// Posts `notification`.
pub(crate) fn post(app: &AppHandle, notification: &JobNotification) {
    let mut builder = app
        .notification()
        .builder()
        .id(notification.notification_id())
        .title(&notification.title)
        .body(notification.display_body());
    if notification.plays_sound {
        builder = builder.sound("default");
    }
    if let Some(category) = notification.category {
        builder = builder.action_type_id(category.id());
    }
    for (key, value) in notification.user_info() {
        builder = builder.extra(key, value);
    }
    // The plugin doesn't replace a notification by its ID on desktop: it closes the job's earlier one first, which
    // works on Linux; on Windows `notify-rust` can't remove a shown notification, so both stay.
    let _ = app
        .notification()
        .remove_active(vec![notification.notification_id()]);
    if let Err(error) = builder.show() {
        tracing::error!(%error, "can't post a notification");
    }
}

/// Runs on the plugin's thread; the window work goes to the main thread.
fn respond(app: &AppHandle, response: &NotificationResponse) {
    match response.effect() {
        NotificationEffect::Open(place) => on_main(app, move |app| window::open(app, &place)),
        NotificationEffect::Reveal(file) => {
            if let Err(error) = app.opener().reveal_item_in_dir(&file) {
                tracing::error!(%error, "can't show the result in the file manager");
            }
        }
        NotificationEffect::Retry(id) => {
            on_main(app, move |app| {
                window::open(app, &TrayOpen::QueueJob { id: id.to_string() });
            });
            let Some(environment) = environment(app) else {
                return;
            };
            if let Some(command) = jutsu_app::system::retry_command(&environment, id) {
                tauri::async_runtime::spawn(async move { environment.queue.perform(id, command).await });
            }
        }
    }
}

fn on_main(app: &AppHandle, work: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || work(&handle)) {
        tracing::error!(%error, "can't answer the notification");
    }
}
