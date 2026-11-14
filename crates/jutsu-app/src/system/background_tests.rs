//! `BackgroundPolicyTests` and `MenuBarModeTests`.

use super::*;

#[allow(
    clippy::unnecessary_wraps,
    reason = "compared with what `quit_prompt` returns"
)]
fn prompt(message: &str, detail: &str) -> Option<QuitPrompt> {
    Some(QuitPrompt {
        message: message.to_owned(),
        detail: detail.to_owned(),
    })
}

#[test]
fn taskbar_badge() {
    assert_eq!(super::taskbar_badge(0), None);
    assert_eq!(super::taskbar_badge(3).as_deref(), Some("3"));
}

#[test]
fn quit_prompt_counts_the_jobs() {
    assert_eq!(quit_prompt(0, 0, ImageWork::NONE, QuitReason::User), None);
    assert_eq!(
        quit_prompt(1, 0, ImageWork::NONE, QuitReason::User),
        prompt("1 job is still running.", "It resumes on next launch.")
    );
    assert_eq!(
        quit_prompt(4, 0, ImageWork::NONE, QuitReason::User),
        prompt("4 jobs are still running.", "They resume on next launch.")
    );
}

#[test]
fn quit_prompt_says_a_job_is_being_submitted() {
    assert_eq!(
        quit_prompt(1, 1, ImageWork::NONE, QuitReason::User),
        prompt(
            "1 job is still running.",
            "It is being sent to Higgsfield right now, so quitting waits up to 5 seconds for Higgsfield to answer. \
             It resumes on next launch."
        )
    );
    assert_eq!(
        quit_prompt(3, 1, ImageWork::NONE, QuitReason::User).map(|prompt| prompt.detail),
        Some(
            "1 of them is being sent to Higgsfield right now, so quitting waits up to 5 seconds for Higgsfield to \
             answer. They resume on next launch."
                .to_owned()
        )
    );
    assert!(
        quit_prompt(3, 2, ImageWork::NONE, QuitReason::User)
            .is_some_and(|prompt| prompt.detail.starts_with("2 of them are being sent"))
    );
    assert_eq!(quit_prompt(3, 2, ImageWork::NONE, QuitReason::LogOut), None);
}

#[test]
fn image_work_keeps_the_app_running_and_is_asked_about_on_quit() {
    let making = ImageWork {
        running: 1,
        waiting: 0,
    };
    let sending = ImageWork {
        running: 2,
        waiting: 1,
    };
    assert!(keeps_running(0, making));
    assert!(keeps_running(1, ImageWork::NONE));
    assert!(!keeps_running(0, ImageWork::NONE));
    assert_eq!(
        quit_prompt(0, 0, making, QuitReason::User),
        prompt(
            "1 image request is still running.",
            "Images still being made are followed again on next launch."
        )
    );
    assert_eq!(
        quit_prompt(0, 0, sending, QuitReason::User).map(|prompt| prompt.detail),
        Some(
            "Quitting waits up to 5 seconds for the ones being sent or saved. Images still being made are followed \
             again on next launch."
                .to_owned()
        )
    );
    assert_eq!(
        quit_prompt(1, 0, sending, QuitReason::User),
        prompt(
            "1 job is still running.",
            "It resumes on next launch. 2 image requests are running too. Quitting waits up to 5 seconds for the \
             ones being sent or saved. Images still being made are followed again on next launch."
        )
    );
    assert_eq!(quit_prompt(0, 0, sending, QuitReason::LogOut), None);
}

#[test]
fn system_quit_never_asks() {
    for reason in [QuitReason::LogOut, QuitReason::Restart, QuitReason::ShutDown] {
        assert_eq!(quit_prompt(3, 0, ImageWork::NONE, reason), None);
        assert_eq!(quit_prompt(0, 0, ImageWork::NONE, reason), None);
    }
}

/// Tray mode keeps the app with nothing to do; off it, closing the window quits as before unless work runs.
#[test]
fn tray_mode_keeps_the_app_running_with_nothing_to_do() {
    assert!(keeps_running_in_tray(true, 0, ImageWork::NONE));
    assert!(keeps_running_in_tray(true, 2, ImageWork::NONE));
    assert!(!keeps_running_in_tray(false, 0, ImageWork::NONE));
    assert!(keeps_running_in_tray(false, 1, ImageWork::NONE));
    assert!(keeps_running_in_tray(
        false,
        0,
        ImageWork {
            running: 1,
            waiting: 0
        }
    ));
}

#[test]
fn the_helper_always_starts_the_app_hidden_and_login_only_in_tray_mode() {
    for runs_in_tray in [false, true] {
        assert!(starts_hidden(Launch::Helper, runs_in_tray));
        assert!(!starts_hidden(Launch::Person, runs_in_tray));
    }
    assert!(starts_hidden(Launch::Login, true));
    assert!(!starts_hidden(Launch::Login, false));
}

#[test]
fn the_helpers_argument_makes_a_helper_launch_whatever_else_is_passed() {
    let app = "jutsu-studio".to_owned();
    let helper = jutsu_agent::HELPER_LAUNCH_ARGUMENT.to_owned();
    let login = LOGIN_LAUNCH_ARGUMENT.to_owned();
    assert_eq!(launch(&[app.clone(), helper.clone()]), Launch::Helper);
    assert_eq!(launch(&[app.clone(), login.clone(), helper]), Launch::Helper);
    assert_eq!(launch(&[app.clone(), login]), Launch::Login);
    assert_eq!(launch(&[app]), Launch::Person);
}

/// With the tray icon shown the app stays once the work ends; without it the app quits.
#[test]
fn the_app_stays_after_the_work_while_the_tray_icon_shows() {
    assert!(!quits_when_work_ends(true, false));
    assert!(!quits_when_work_ends(false, true));
    assert!(!quits_when_work_ends(true, true));
    assert!(quits_when_work_ends(false, false));
}
