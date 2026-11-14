//! `AudioPlanTests.swift`.

use jutsu_core::*;

struct Plan {
    option: AudioOption,
    range: ClosedRange<f64>,
    source_audio_end: Option<f64>,
    result_duration: f64,
    result_has_audio: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            option: AudioOption::Original,
            range: ClosedRange::new(2.0, 12.0),
            source_audio_end: Some(60.0),
            result_duration: 10.0,
            result_has_audio: false,
        }
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "taken whole, like the Swift helper's labelled arguments"
)]
fn plan(args: Plan) -> AudioPlan {
    AudioPlan::make(
        args.option,
        args.range,
        args.source_audio_end,
        args.result_duration,
        args.result_has_audio,
    )
}

fn add(lower: f64, upper: f64, fade_out: Option<(f64, f64)>) -> AudioPlan {
    AudioPlan::AddSource {
        range: ClosedRange::new(lower, upper),
        fade_out: fade_out.map(|(lower, upper)| ClosedRange::new(lower, upper)),
    }
}

fn option(option: AudioOption) -> Plan {
    Plan {
        option,
        ..Plan::default()
    }
}

/// The cases of every Swift test, one row each: the source's range goes under a silent result, cut to the result and to
/// the source's own sound, faded over its last second when asked; anything else leaves the result as it came.
#[test]
#[allow(clippy::too_many_lines, reason = "one row per case of the Swift tests")]
fn each_result_gets_the_planned_sound() {
    let keep = AudioPlan::Keep;
    let cases = [
        (
            "silent result gets the source range",
            Plan::default(),
            add(2.0, 12.0, None),
        ),
        (
            "result with its own sound is kept",
            Plan {
                result_has_audio: true,
                ..Plan::default()
            },
            keep,
        ),
        (
            "result with its own sound is kept when fading",
            Plan {
                result_has_audio: true,
                ..option(AudioOption::FadeOut)
            },
            keep,
        ),
        // The model was asked for the sound, so even a result that came back silent doesn't get the source's.
        (
            "generated sound is left as it came",
            Plan {
                result_has_audio: true,
                ..option(AudioOption::Generated)
            },
            keep,
        ),
        (
            "generated silence is left as it came",
            option(AudioOption::Generated),
            keep,
        ),
        (
            "silent source leaves the result alone",
            Plan {
                source_audio_end: None,
                ..Plan::default()
            },
            keep,
        ),
        (
            "silent source leaves the result alone when fading",
            Plan {
                source_audio_end: None,
                ..option(AudioOption::FadeOut)
            },
            keep,
        ),
        (
            "mute strips sound",
            Plan {
                result_has_audio: true,
                ..option(AudioOption::Mute)
            },
            AudioPlan::RemoveAudio,
        ),
        (
            "mute leaves a silent result alone",
            option(AudioOption::Mute),
            keep,
        ),
        (
            "mute strips sound from a silent source's result",
            Plan {
                source_audio_end: None,
                result_has_audio: true,
                ..option(AudioOption::Mute)
            },
            AudioPlan::RemoveAudio,
        ),
        (
            "shorter result cuts the audio",
            Plan {
                result_duration: 7.5,
                ..Plan::default()
            },
            add(2.0, 9.5, None),
        ),
        (
            "longer result is not stretched",
            Plan {
                result_duration: 14.0,
                ..Plan::default()
            },
            add(2.0, 12.0, None),
        ),
        (
            "audio track ending early limits the range",
            Plan {
                source_audio_end: Some(8.0),
                ..Plan::default()
            },
            add(2.0, 8.0, None),
        ),
        (
            "audio track ending at the start leaves nothing",
            Plan {
                source_audio_end: Some(2.0),
                ..Plan::default()
            },
            keep,
        ),
        (
            "audio track ending before the start leaves nothing",
            Plan {
                source_audio_end: Some(1.0),
                ..Plan::default()
            },
            keep,
        ),
        (
            "empty result is kept",
            Plan {
                result_duration: 0.0,
                ..Plan::default()
            },
            keep,
        ),
        (
            "fade out covers the last second of the audio",
            option(AudioOption::FadeOut),
            add(2.0, 12.0, Some((9.0, 10.0))),
        ),
        (
            "fade out of a shorter result",
            Plan {
                result_duration: 6.0,
                ..option(AudioOption::FadeOut)
            },
            add(2.0, 8.0, Some((5.0, 6.0))),
        ),
        (
            "fade out of very short audio covers all of it",
            Plan {
                source_audio_end: Some(2.5),
                ..option(AudioOption::FadeOut)
            },
            add(2.0, 2.5, Some((0.0, 0.5))),
        ),
        (
            "negative start is clamped",
            Plan {
                range: ClosedRange::new(-1.0, 5.0),
                result_duration: 10.0,
                ..Plan::default()
            },
            add(0.0, 5.0, None),
        ),
    ];
    for (case, args, expected) in cases {
        assert_eq!(plan(args), expected, "{case}");
    }
}
