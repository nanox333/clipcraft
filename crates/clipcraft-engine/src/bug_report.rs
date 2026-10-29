//! Bounded, local bug-report preparation. No device discovery, filesystem access or network.

use serde::{Deserialize, Serialize};

use crate::{EngineError, RecordingSettings};

/// Maximum UTF-8 bytes in each user-written field.
pub const BUG_REPORT_FIELD_BYTES: usize = 4096;

/// Text supplied by the user. It can contain private information and should be reviewed before sharing.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BugReportInput {
    pub observed: String,
    pub expected: String,
    pub steps: String,
    pub include_settings: bool,
}

/// Builds a reviewable report without paths, device names, media, logs or account data.
pub(crate) fn build(
    input: &BugReportInput,
    settings: &RecordingSettings,
) -> Result<String, EngineError> {
    if input.observed.trim().is_empty()
        || [&input.observed, &input.expected, &input.steps]
            .iter()
            .any(|field| field.len() > BUG_REPORT_FIELD_BYTES)
    {
        return Err(EngineError::BugReportInvalid);
    }
    let mut report = format!(
        "## What happened\n{}\n\n## What I expected\n{}\n\n## Steps to reproduce\n{}\n\n## Environment\nClipCraft {} · {} {}\n",
        input.observed.trim(),
        input.expected.trim(),
        input.steps.trim(),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    if input.include_settings {
        let settings = settings.clone().sanitized();
        report.push_str(&format!(
            "Capture: {} · {}p{} · {} · {}\nReplay: {} s · post-roll: {} s\nSystem audio: {} · microphone: {}\n",
            settings.source.label(), settings.height, settings.frames_per_second,
            settings.video_encoder.label(), settings.codec.label(), settings.replay_seconds,
            settings.post_roll_seconds,
            if settings.capture_game_audio { "on" } else { "off" },
            if settings.capture_microphone { "on" } else { "off" },
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, CommandOutput, execute};

    #[test]
    fn report_only_includes_selected_non_private_settings() {
        let settings = RecordingSettings {
            capture_folder: "/private/alice/clips".into(),
            microphone_device: "private microphone".into(),
            capture_target_name: "private game window".into(),
            ..Default::default()
        };
        let input = BugReportInput {
            observed: "Playback stops".into(),
            include_settings: true,
            ..Default::default()
        };
        let output = execute(Command::PrepareBugReport {
            input: input.clone(),
            settings: Box::new(settings.clone()),
        })
        .unwrap();
        let CommandOutput::BugReport(text) = output else {
            panic!("wrong result")
        };
        assert!(text.contains("Playback stops") && text.contains("Capture:"));
        assert!(
            !text.contains("alice")
                && !text.contains("private microphone")
                && !text.contains("private game window")
        );
        let without = build(
            &BugReportInput {
                include_settings: false,
                ..input
            },
            &settings,
        )
        .unwrap();
        assert!(!without.contains("Capture:"));
    }

    #[test]
    fn blank_oversized_and_unknown_inputs_are_rejected() {
        assert!(build(&BugReportInput::default(), &RecordingSettings::default()).is_err());
        let input = BugReportInput {
            observed: "x".repeat(BUG_REPORT_FIELD_BYTES + 1),
            ..Default::default()
        };
        assert!(build(&input, &RecordingSettings::default()).is_err());
        assert!(
            serde_json::from_str::<BugReportInput>(r#"{"observed":"x","unknown":true}"#).is_err()
        );
    }
}
