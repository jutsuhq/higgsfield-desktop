//! The tray panel (`MenuBarContent`): a small undecorated window by the tray icon, showing the `/tray` route. It is
//! made on the first click and hidden, not closed, when it loses focus.

use std::sync::{Arc, Mutex, PoisonError};

use jutsu_app::AppEnvironment;
use jutsu_app::agents::preferences::AgentPreferences;
use jutsu_app::agents::runtime::AgentRuntime;
use jutsu_app::queue::JobCommand;
use jutsu_app::system::icons::{self, ScreenRect};
use jutsu_app::system::tray::{AgentAsk, AgentsControl, AgentsInput, PanelAction, TrayPanel};
use tauri::{AppHandle, Manager, Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt as _;
use time::OffsetDateTime;

use super::window;

/// The panel window's label.
pub(crate) const LABEL: &str = "tray";
/// The panel's width in logical pixels (`MenuBarContent`'s), and its tallest: past it, the content scrolls.
const WIDTH: f64 = 330.0;
const MAX_HEIGHT: f64 = 560.0;
/// The shortest the panel gets, so a page still loading doesn't make it a sliver.
const MIN_HEIGHT: f64 = 120.0;

/// The panel's height as the page measured it, and the icon it was last shown by.
static FIT: Mutex<(f64, Option<Rect>)> = Mutex::new((MAX_HEIGHT, None));

/// When the panel last hid on losing focus. A click on the icon takes the focus first, so a click within
/// `CLICK_HIDES` of it closed the panel and must not open it again.
static BLURRED_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
const CLICK_HIDES: std::time::Duration = std::time::Duration::from_millis(300);

fn fit_state() -> std::sync::MutexGuard<'static, (f64, Option<Rect>)> {
    FIT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The agents' side of the panel: whether they're allowed, which clients sent requests lately, their open asks and
/// today's budget.
async fn agents(app: &AppHandle, environment: &AppEnvironment) -> AgentsInput {
    let is_allowed = AgentPreferences::new(environment.preferences.clone()).is_allowed();
    let Some(runtime) = app
        .try_state::<Arc<AgentRuntime>>()
        .map(|state| state.inner().clone())
    else {
        return AgentsInput {
            is_allowed,
            ..AgentsInput::default()
        };
    };
    let now = OffsetDateTime::now_utc();
    let activity = runtime.service.activity();
    let budget = if is_allowed {
        runtime.service.budget_today().await.unwrap_or_else(|error| {
            tracing::warn!(%error, "can't read today's agent budget");
            None
        })
    } else {
        None
    };
    AgentsInput {
        is_allowed,
        connected_clients: activity.connected_clients(now),
        asks: activity
            .open_asks(now)
            .into_iter()
            .map(|ask| AgentAsk {
                item: ask.item,
                sheet: ask.sheet,
                title: ask.title,
            })
            .collect(),
        budget,
    }
}

/// What the panel shows now.
pub(crate) async fn model(app: &AppHandle, environment: &AppEnvironment) -> TrayPanel {
    let agents = agents(app, environment).await;
    jutsu_app::system::tray_panel(environment, agents, OffsetDateTime::now_utc()).await
}

/// Runs what a panel row or the menu that stands for the panel asks.
pub(crate) async fn perform(app: &AppHandle, environment: &AppEnvironment, action: PanelAction) {
    match action {
        PanelAction::Open(place) => window::open(app, &place),
        PanelAction::Cancel(id) => {
            if let Ok(job) = uuid::Uuid::parse_str(&id) {
                if environment.on_device.cancel_run(job) {
                    return;
                }
                environment.queue.perform(job, JobCommand::Cancel).await;
            } else {
                tracing::warn!(id, "the panel canceled something that isn't a job");
            }
        }
        PanelAction::Reveal(file) => {
            if let Err(error) = app.opener().reveal_item_in_dir(&file) {
                tracing::error!(%error, "can't show the result in the file manager");
            }
        }
        PanelAction::Agents(AgentsControl::Pause) => environment.engine.pause_agent_jobs().await,
        PanelAction::Agents(AgentsControl::Resume) => environment.engine.resume_agent_jobs().await,
    }
}

fn existing(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

fn make(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let panel = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("tray".into()))
        .title("Jutsu Studio")
        .inner_size(WIDTH, fit_state().0)
        .resizable(false)
        .decorations(false)
        .skip_taskbar(true)
        // A shadow, and on Windows 11 the rounded corners of the system's own flyouts.
        .shadow(true)
        .always_on_top(true)
        .visible(false)
        .background_color(tauri::window::Color(6, 6, 7, 255))
        .build()?;
    let hidden = panel.clone();
    panel.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            *BLURRED_AT.lock().unwrap_or_else(PoisonError::into_inner) = Some(std::time::Instant::now());
            let _ = hidden.hide();
        }
    });
    Ok(panel)
}

/// A click on the tray icon: shows the panel by the icon, or hides it when it shows.
pub(crate) fn toggle(app: &AppHandle, icon: Rect) {
    if let Some(panel) = existing(app)
        && panel.is_visible().unwrap_or(false)
    {
        let _ = panel.hide();
        return;
    }
    if BLURRED_AT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .is_some_and(|at| at.elapsed() < CLICK_HIDES)
    {
        return;
    }
    let panel = match existing(app).map_or_else(|| make(app), Ok) {
        Ok(panel) => panel,
        Err(error) => {
            tracing::error!(%error, "can't make the tray panel");
            return;
        }
    };
    fit_state().1 = Some(icon);
    place(app, &panel, icon);
    let _ = panel.show();
    let _ = panel.set_focus();
}

/// Moves the panel next to the icon, inside the work area of the icon's screen.
fn place(app: &AppHandle, panel: &WebviewWindow, icon: Rect) {
    let scale = panel.scale_factor().unwrap_or(1.0);
    let position = icon.position.to_physical::<i32>(scale);
    let size = icon.size.to_physical::<i32>(scale);
    // The monitor whose physical bounds hold the icon. `monitor_from_point` is not used: on macOS it takes points,
    // everywhere else pixels, while the tray reports pixels.
    let centre = (position.x + size.width / 2, position.y + size.height / 2);
    let Some(monitor) = app
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .find(|monitor| {
            let (origin, extent) = (monitor.position(), monitor.size());
            (origin.x..origin.x + i32::try_from(extent.width).unwrap_or(i32::MAX)).contains(&centre.0)
                && (origin.y..origin.y + i32::try_from(extent.height).unwrap_or(i32::MAX)).contains(&centre.1)
        })
    else {
        return;
    };
    let area = monitor.work_area();
    let monitor_scale = monitor.scale_factor();
    #[allow(clippy::cast_possible_truncation, reason = "a window's size in pixels")]
    let panel_size = (
        (WIDTH * monitor_scale).round() as i32,
        (fit_state().0 * monitor_scale).round() as i32,
    );
    let (x, y) = icons::panel_origin(
        ScreenRect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
        panel_size,
        ScreenRect {
            x: area.position.x,
            y: area.position.y,
            width: i32::try_from(area.size.width).unwrap_or(i32::MAX),
            height: i32::try_from(area.size.height).unwrap_or(i32::MAX),
        },
    );
    let _ = panel.set_position(tauri::PhysicalPosition::new(x, y));
}

/// The page measured its content: the panel takes that height, up to its most, and stays by the icon.
pub(crate) fn fit(app: &AppHandle, height: f64) {
    let height = height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    let icon = {
        let mut state = fit_state();
        if (state.0 - height).abs() < 0.5 {
            return;
        }
        state.0 = height;
        state.1
    };
    let Some(panel) = existing(app) else {
        return;
    };
    let _ = panel.set_size(tauri::LogicalSize::new(WIDTH, height));
    if let Some(icon) = icon {
        place(app, &panel, icon);
    }
}

/// Hides the panel, when it was made.
pub(crate) fn hide(app: &AppHandle) {
    if let Some(panel) = existing(app) {
        let _ = panel.hide();
    }
}
