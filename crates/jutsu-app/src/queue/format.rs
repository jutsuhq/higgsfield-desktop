//! How prices, times and job titles read in the Queue (`CostText.swift`, `JobRowModel+Formatting.swift`,
//! `FailureReason+Text.swift`, `DomainTitles.swift`, `PostOperations.swift`). English only, written the way the
//! macOS app writes them in `en_US`.

use jutsu_core::{
    Cost, CostUnit, DubLanguage, FailureReason, KlingQuality, PostAction, Rejection, Resolution,
    UpscaleTarget, VideoControl, VideoModel, decimal_text,
};
use rust_decimal::{Decimal, RoundingStrategy};
use time::OffsetDateTime;

/// A job paid from the plan's unlimited allowance instead of credits.
pub const UNLIMITED: &str = "Unlimited";
/// A price no answer has given.
pub const PRICE_UNKNOWN: &str = "Price unknown";
/// Where an unknown price comes from.
pub const CHARGED_ON_COMPLETION: &str = "Charged on completion";
/// An operation Higgsfield prices only once it has run.
pub const PRICE_AFTER_RUN: &str = "Price shown after the run";

/// "$8.85": rounded half up to cents, never grouped.
pub fn dollars(amount: Decimal) -> String {
    let rounded = amount.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    if rounded.is_sign_negative() && !rounded.is_zero() {
        return format!("-${:.2}", rounded.abs());
    }
    format!("${:.2}", rounded.abs())
}

/// "96 cr" or "0.25 cr": up to three decimals, rounded up so a price never looks lower.
pub fn credits(amount: Decimal) -> String {
    format!("{} cr", exact_credits(amount))
}

/// "4", "0.25", "1.125".
pub fn exact_credits(amount: Decimal) -> String {
    decimal_text(amount.round_dp_with_strategy(3, RoundingStrategy::AwayFromZero))
}

/// "$8.85" or "96 credits".
pub fn amount(amount: Decimal, unit: CostUnit) -> String {
    match unit {
        CostUnit::Usd => dollars(amount),
        CostUnit::Credits => format!("{} credits", exact_credits(amount)),
    }
}

/// "Price unknown" for an unknown price, never "$0.00" or "0 cr".
pub fn cost_text(cost: &Cost) -> String {
    if cost.is_unknown_price() {
        return PRICE_UNKNOWN.to_owned();
    }
    match cost.unit() {
        CostUnit::Usd => dollars(cost.amount()),
        CostUnit::Credits => credits(cost.amount()),
    }
}

/// "$8.17" or "16 cr"; zero in the cost's unit when it was not charged. Estimates get a "~".
pub fn format_cost(cost: &Cost, charged: bool) -> String {
    let (amount, prefix) = if charged {
        (cost.amount(), if cost.is_estimate() { "~" } else { "" })
    } else {
        (Decimal::ZERO, "")
    };
    match cost.unit() {
        CostUnit::Usd => format!("{prefix}{}", dollars(amount)),
        CostUnit::Credits => format!("{prefix}{}", credits(amount)),
    }
}

/// The sum, an estimate when any part is; `None` for no costs or for costs in different units.
pub fn total(costs: &[Cost]) -> Option<Cost> {
    let (first, rest) = costs.split_first()?;
    let mut total = first.clone();
    for cost in rest {
        if cost.unit() != total.unit() {
            return None;
        }
        let amount = total.amount() + cost.amount();
        let is_estimate = total.is_estimate() || cost.is_estimate();
        total = match total.unit() {
            CostUnit::Usd => Cost::usd(amount, is_estimate),
            CostUnit::Credits => Cost::credits(amount, is_estimate),
        };
    }
    Some(total)
}

/// "01:06.5": minutes and seconds with tenths, rounded half up to tenths.
pub fn timecode(seconds: f64) -> String {
    let tenths = whole((seconds.max(0.0) * 10.0).round());
    format!("{:02}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// "30 s", "4.5 s": a limit, without a trailing ".0".
pub fn limit(seconds: f64) -> String {
    let rounded = (seconds * 1000.0).round() / 1000.0;
    format!("{rounded} s")
}

/// "42%" for 0.42, rounded half up.
pub fn percent(fraction: f64) -> String {
    format!("{}%", whole((fraction * 100.0).round()))
}

/// "0:42", "12:05", "1:02:09".
pub fn duration(seconds: f64) -> String {
    let total = whole(seconds.max(0.0).trunc());
    let (hours, minutes, rest) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        return format!("{hours}:{minutes:02}:{rest:02}");
    }
    format!("{minutes}:{rest:02}")
}

/// "~40 s", "~2 min", "~1 h 30 min": rounded so it doesn't read as a measurement.
pub fn approximate(seconds: f64) -> String {
    let total = seconds.max(0.0);
    if total < 55.0 {
        return format!("~{} s", (whole((total / 10.0).round()) * 10).max(10));
    }
    let minutes = whole((total / 60.0).round());
    if minutes < 60 {
        return format!("~{minutes} min");
    }
    let rest = minutes % 60;
    if rest == 0 {
        format!("~{} h", minutes / 60)
    } else {
        format!("~{} h {rest} min", minutes / 60)
    }
}

/// "40 s", "1 min 40 s", "1 h 2 min".
pub fn spoken(seconds: f64) -> String {
    let total = whole(seconds.round().max(0.0));
    let (hours, minutes, rest) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        return if minutes == 0 {
            format!("{hours} h")
        } else {
            format!("{hours} h {minutes} min")
        };
    }
    if minutes > 0 {
        return if rest == 0 {
            format!("{minutes} min")
        } else {
            format!("{minutes} min {rest} s")
        };
    }
    format!("{rest} s")
}

/// "just now", "3 min ago", "2 h ago", "4 d ago".
pub fn relative_time(date: OffsetDateTime, now: OffsetDateTime) -> String {
    let minutes = (now - date).as_seconds_f64() / 60.0;
    if minutes < 1.0 {
        "just now".to_owned()
    } else if minutes < 60.0 {
        format!("{} min ago", whole(minutes.round()))
    } else if minutes < 1440.0 {
        format!("{} h ago", whole((minutes / 60.0).round()))
    } else {
        format!("{} d ago", whole((minutes / 1440.0).round()))
    }
}

/// What a failed job's row says; a message as it came, "Failed" when empty.
///
/// The macOS text of `notEnoughDiskSpace` says "on your Mac"; here it says "on this computer".
pub fn failure_text(reason: &FailureReason) -> String {
    match reason {
        FailureReason::Deadline => {
            "Higgsfield didn't finish this job in time. Check the result in your generations on Higgsfield."
        }
        FailureReason::OutcomeUnknown => {
            "Higgsfield may have started this job. Check your generations there before trying again."
        }
        FailureReason::SubmitCutOff => {
            "Jutsu Studio stopped while sending this job, so Higgsfield may have started it. Look for it in your \
             Higgsfield history in Library before you retry: if it started, a retry is charged again."
        }
        FailureReason::NotEnoughDiskSpace => {
            "Not enough disk space to prepare this job. Free up space on this computer, then retry."
        }
        FailureReason::OutputUnavailable => {
            "The result is no longer available on Higgsfield. Results expire about 7 days after they finish."
        }
        FailureReason::InputUnavailable => {
            "An input of this job is no longer available on Higgsfield. Add the files again to run it."
        }
        FailureReason::InsecureInput => {
            "An input of this job doesn't point to a secure Higgsfield address. Add the files again to run it."
        }
        FailureReason::ContentModeration => "Rejected by content moderation.",
        FailureReason::GenerationFailed => "The generation failed.",
        FailureReason::CanceledOnServer => "Canceled on the server.",
        FailureReason::InternalError => "Internal error.",
        FailureReason::Message(text) => {
            let text = text.trim();
            return if text.is_empty() {
                "Failed".to_owned()
            } else {
                text.to_owned()
            };
        }
    }
    .to_owned()
}

/// "Rejected: …", for the queue row.
pub fn rejection_title(rejection: Rejection) -> &'static str {
    match rejection {
        Rejection::ExplicitContent => "Rejected as explicit content",
        Rejection::ProtectedContent => "Rejected: a recognizable person or protected content",
    }
}

/// "Upscale to 4K", "Reframe to 9:16".
pub fn post_action_title(action: &PostAction) -> String {
    match action {
        PostAction::Upscale(options) => format!("Upscale to {}", upscale_target_title(options.target)),
        PostAction::RemoveBackground => "Remove Background".to_owned(),
        PostAction::Reframe(aspect) => format!("Reframe to {}", aspect.raw_value()),
        PostAction::Dub(language) => format!("Dub into {}", dub_language_title(*language)),
        PostAction::ChangeVoice(voice) => format!("Change Voice to {}", voice.name),
    }
}

fn upscale_target_title(target: UpscaleTarget) -> &'static str {
    match target {
        UpscaleTarget::P1080 => "1080p",
        UpscaleTarget::K2 => "2K",
        UpscaleTarget::K4 => "4K",
    }
}

/// The language's English name.
pub fn dub_language_title(language: DubLanguage) -> &'static str {
    match language {
        DubLanguage::English => "English",
        DubLanguage::Chinese => "Chinese",
        DubLanguage::French => "French",
        DubLanguage::Hindi => "Hindi",
        DubLanguage::Italian => "Italian",
        DubLanguage::Japanese => "Japanese",
        DubLanguage::Korean => "Korean",
        DubLanguage::Portuguese => "Portuguese",
        DubLanguage::Russian => "Russian",
        DubLanguage::Turkish => "Turkish",
        DubLanguage::Spanish => "Spanish",
        DubLanguage::German => "German",
        DubLanguage::Arabic => "Arabic",
        DubLanguage::Polish => "Polish",
        DubLanguage::Indonesian => "Indonesian",
        DubLanguage::Filipino => "Filipino",
        DubLanguage::Swedish => "Swedish",
        DubLanguage::Finnish => "Finnish",
    }
}

/// "720p"; on a model with a quality instead (Kling), "Standard" or "Pro".
pub fn resolution_title(resolution: Resolution, model: &VideoModel) -> String {
    if !model.declares(VideoControl::Quality) {
        return resolution.raw_value().to_owned();
    }
    match KlingQuality::from_resolution(resolution) {
        KlingQuality::Standard => "Standard",
        KlingQuality::Pro => "Pro",
    }
    .to_owned()
}

/// A rounded, non-negative float as an integer; saturates far outside any time or amount shown.
#[allow(
    clippy::cast_possible_truncation,
    reason = "callers round first; `as` saturates at the bounds"
)]
fn whole(value: f64) -> i64 {
    value as i64
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn cost_formatting() {
        let usd = |text: &str, estimate| Cost::usd(Decimal::from_str(text).unwrap(), estimate);
        assert_eq!(format_cost(&usd("0.318", false), true), "$0.32");
        assert_eq!(format_cost(&usd("12", true), true), "~$12.00");
        assert_eq!(format_cost(&usd("3.005", false), true), "$3.01");
        assert_eq!(
            format_cost(&Cost::credits(Decimal::from(1200), false), true),
            "1200 cr"
        );
        assert_eq!(format_cost(&usd("8.168", false), false), "$0.00");
        assert_eq!(
            format_cost(&Cost::credits(Decimal::from(96), false), false),
            "0 cr"
        );
        assert_eq!(credits(Decimal::from_str("0.2501").unwrap()), "0.251 cr");
        assert_eq!(amount(Decimal::from(96), CostUnit::Credits), "96 credits");
        assert_eq!(amount(Decimal::from_str("8.5").unwrap(), CostUnit::Usd), "$8.50");
    }

    #[test]
    fn totals_need_one_unit() {
        let costs = [
            Cost::usd(Decimal::from(2), false),
            Cost::usd(Decimal::from(3), true),
        ];
        assert_eq!(total(&costs), Some(Cost::usd(Decimal::from(5), true)));
        assert_eq!(
            total(&[costs[0].clone(), Cost::credits(Decimal::ONE, false)]),
            None
        );
        assert_eq!(total(&[]), None);
    }

    #[test]
    fn timecodes() {
        assert_eq!(timecode(0.0), "00:00.0");
        assert_eq!(timecode(12.04), "00:12.0");
        assert_eq!(timecode(59.96), "01:00.0");
        assert_eq!(timecode(754.5), "12:34.5");
        assert_eq!(limit(30.0), "30 s");
        assert_eq!(limit(4.5), "4.5 s");
    }

    #[test]
    fn duration_formats() {
        assert_eq!(duration(42.0), "0:42");
        assert_eq!(duration(725.0), "12:05");
        assert_eq!(duration(3729.0), "1:02:09");
    }

    #[test]
    fn approximate_durations() {
        for (seconds, text) in [
            (0.0, "~10 s"),
            (24.0, "~20 s"),
            (54.0, "~50 s"),
            (55.0, "~1 min"),
            (100.0, "~2 min"),
            (3540.0, "~59 min"),
            (3600.0, "~1 h"),
            (5400.0, "~1 h 30 min"),
        ] {
            assert_eq!(approximate(seconds), text, "{seconds}");
        }
    }

    #[test]
    fn spoken_durations() {
        for (seconds, text) in [
            (40.0, "40 s"),
            (60.0, "1 min"),
            (100.0, "1 min 40 s"),
            (3600.0, "1 h"),
            (3720.0, "1 h 2 min"),
        ] {
            assert_eq!(spoken(seconds), text, "{seconds}");
        }
    }

    #[test]
    fn relative_times() {
        let now = OffsetDateTime::from_unix_timestamp(1_000_000).unwrap();
        for (minutes, text) in [
            (0.0, "just now"),
            (0.5, "just now"),
            (3.0, "3 min ago"),
            (59.0, "59 min ago"),
            (90.0, "2 h ago"),
            (1439.0, "24 h ago"),
            (2880.0, "2 d ago"),
        ] {
            let date = now - time::Duration::seconds_f64(minutes * 60.0);
            assert_eq!(relative_time(date, now), text, "{minutes}");
        }
    }

    #[test]
    fn failure_texts() {
        assert_eq!(failure_text(&FailureReason::Message(" ".into())), "Failed");
        assert_eq!(failure_text(&FailureReason::Message(" Boom\n".into())), "Boom");
        assert_eq!(
            failure_text(&FailureReason::ContentModeration),
            "Rejected by content moderation."
        );
    }
}
