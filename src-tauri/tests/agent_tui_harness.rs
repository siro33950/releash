#[path = "support/agent_tui_fixture.rs"]
mod agent_tui_fixture;

use agent_tui_fixture::{
    parsed_signals, run_fixture, signal_events, CapturedLifecyclePayload, FixtureLifecycleEmission,
    FixtureLifecyclePayload, FixtureLifecycleSignal, FixturePlan, FixtureRunOptions,
    FIXTURE_ATTEMPT_KEY, FIXTURE_SESSION_KEY, FIXTURE_TRANSCRIPT_REF,
};
use portable_pty::PtySize;
use std::time::Duration;

#[test]
fn atui_000_real_pty_io_and_lifecycle_use_independent_channels() {
    let run = run_fixture(
        FixturePlan::new(
            "arbitrary-visible-provider-wording",
            vec![
                FixtureLifecycleEmission::signal("submit", 1),
                FixtureLifecycleEmission::signal("stop", 2),
            ],
        ),
        FixtureRunOptions::default(),
    );

    assert_eq!(run.exit_code, 0);
    assert!(run.terminal_output.contains("\x1b[?1049h"));
    assert!(run.terminal_output.contains("\x1b[1;32m"));
    assert!(run.terminal_output.contains("日本語🙂"));
    assert!(run.terminal_output.contains("operator-input"));
    assert!(!run.terminal_output.contains("\"event\":\"stop\""));
    assert_eq!(signal_events(&run), vec!["submit", "stop"]);
    assert!(parsed_signals(&run).iter().all(|signal| {
        signal.session_key == FIXTURE_SESSION_KEY
            && signal.attempt_key == FIXTURE_ATTEMPT_KEY
            && signal.transcript_ref.as_deref() == Some(FIXTURE_TRANSCRIPT_REF)
    }));
}

#[test]
fn atui_000_backend_capture_continues_without_surface_consumer() {
    let run = run_fixture(
        FixturePlan {
            input_lines: 2,
            ..FixturePlan::new("output-without-surface-consumer", vec![])
        },
        FixtureRunOptions {
            input_lines: vec!["first-input".to_string(), "second-input".to_string()],
            ..FixtureRunOptions::default()
        },
    );

    assert_eq!(run.exit_code, 0);
    assert!(run
        .terminal_output
        .contains("output-without-surface-consumer"));
    assert!(run.terminal_output.contains("received-0:first-input"));
    assert!(run.terminal_output.contains("received-1:second-input"));
}

#[cfg(unix)]
#[test]
fn atui_000_real_pty_resize_is_visible_to_provider() {
    let run = run_fixture(
        FixturePlan {
            report_terminal_size: true,
            ..FixturePlan::new("resize-provider", vec![])
        },
        FixtureRunOptions {
            resize_to: Some(PtySize {
                rows: 37,
                cols: 111,
                pixel_width: 0,
                pixel_height: 0,
            }),
            ..FixtureRunOptions::default()
        },
    );

    assert!(run.terminal_output.contains("terminal-size:37x111"));
}

#[test]
fn atui_000_provider_process_exit_is_not_a_stop_signal() {
    let run = run_fixture(
        FixturePlan {
            exit_code: 23,
            ..FixturePlan::new("provider-exits-without-stop", vec![])
        },
        FixtureRunOptions::default(),
    );

    assert_eq!(run.exit_code, 23);
    assert!(run.lifecycle.is_empty());
}

#[test]
fn atui_000_submit_and_stop_order_can_be_reversed() {
    let submit_then_stop = run_fixture(
        FixturePlan::new(
            "same-visible-output",
            vec![
                FixtureLifecycleEmission::signal("submit", 1),
                FixtureLifecycleEmission::signal("stop", 2),
            ],
        ),
        FixtureRunOptions::default(),
    );
    let stop_then_submit = run_fixture(
        FixturePlan::new(
            "same-visible-output",
            vec![
                FixtureLifecycleEmission::signal("stop", 1),
                FixtureLifecycleEmission::signal("submit", 2),
            ],
        ),
        FixtureRunOptions::default(),
    );

    assert_eq!(signal_events(&submit_then_stop), vec!["submit", "stop"]);
    assert_eq!(signal_events(&stop_then_submit), vec!["stop", "submit"]);
}

#[test]
fn atui_000_each_lifecycle_signal_can_be_missing() {
    let submit_only = run_fixture(
        FixturePlan::new(
            "same-visible-output",
            vec![FixtureLifecycleEmission::signal("submit", 1)],
        ),
        FixtureRunOptions::default(),
    );
    let stop_only = run_fixture(
        FixturePlan::new(
            "same-visible-output",
            vec![FixtureLifecycleEmission::signal("stop", 1)],
        ),
        FixtureRunOptions::default(),
    );
    let neither = run_fixture(
        FixturePlan::new("same-visible-output", vec![]),
        FixtureRunOptions::default(),
    );

    assert_eq!(signal_events(&submit_only), vec!["submit"]);
    assert_eq!(signal_events(&stop_only), vec!["stop"]);
    assert!(neither.lifecycle.is_empty());
}

#[test]
fn atui_000_each_lifecycle_signal_can_be_duplicated() {
    let run = run_fixture(
        FixturePlan::new(
            "duplicate-independent-output",
            vec![
                FixtureLifecycleEmission::signal("submit", 1),
                FixtureLifecycleEmission::signal("submit", 2),
                FixtureLifecycleEmission::signal("stop", 3),
                FixtureLifecycleEmission::signal("stop", 4),
            ],
        ),
        FixtureRunOptions::default(),
    );

    assert_eq!(
        signal_events(&run),
        vec!["submit", "submit", "stop", "stop"]
    );
}

#[test]
fn atui_000_lifecycle_signal_can_be_delayed() {
    let run = run_fixture(
        FixturePlan::new(
            "delayed-signal-output",
            vec![
                FixtureLifecycleEmission::signal("submit", 1),
                FixtureLifecycleEmission::delayed_signal("stop", 2, 100),
            ],
        ),
        FixtureRunOptions::default(),
    );

    assert_eq!(signal_events(&run), vec!["submit", "stop"]);
    let gap = run.lifecycle[1]
        .received_after
        .saturating_sub(run.lifecycle[0].received_after);
    assert!(gap >= Duration::from_millis(75), "observed gap: {gap:?}");
}

#[test]
fn atui_000_lifecycle_scope_can_reference_another_session_or_attempt() {
    let mut wrong_session = FixtureLifecycleSignal::new("stop", 1);
    wrong_session.session_key = "other-session".to_string();
    let mut wrong_attempt = FixtureLifecycleSignal::new("stop", 2);
    wrong_attempt.attempt_key = "other-attempt".to_string();
    let run = run_fixture(
        FixturePlan::new(
            "cross-scope-signal-output",
            vec![
                FixtureLifecycleEmission {
                    delay_before_ms: 0,
                    payload: FixtureLifecyclePayload::Signal {
                        signal: wrong_session,
                    },
                },
                FixtureLifecycleEmission {
                    delay_before_ms: 0,
                    payload: FixtureLifecyclePayload::Signal {
                        signal: wrong_attempt,
                    },
                },
            ],
        ),
        FixtureRunOptions::default(),
    );

    let signals = parsed_signals(&run);
    assert_eq!(signals[0].session_key, "other-session");
    assert_eq!(signals[1].attempt_key, "other-attempt");
}

#[test]
fn atui_000_lifecycle_sequence_gap_and_reversal_are_preserved() {
    let run = run_fixture(
        FixturePlan::new(
            "sequence-fault-output",
            vec![
                FixtureLifecycleEmission::signal("submit", 3),
                FixtureLifecycleEmission::signal("stop", 1),
            ],
        ),
        FixtureRunOptions::default(),
    );

    let sequences: Vec<_> = parsed_signals(&run)
        .into_iter()
        .map(|signal| signal.sequence)
        .collect();
    assert_eq!(sequences, vec![3, 1]);
}

#[test]
fn atui_000_malformed_lifecycle_payload_is_observable_without_terminal_parsing() {
    let run = run_fixture(
        FixturePlan::new(
            "malformed-signal-output",
            vec![FixtureLifecycleEmission::raw("{not-valid-json")],
        ),
        FixtureRunOptions::default(),
    );

    assert_eq!(run.lifecycle.len(), 1);
    match &run.lifecycle[0].payload {
        CapturedLifecyclePayload::Invalid(payload) => assert_eq!(payload, "{not-valid-json"),
        CapturedLifecyclePayload::Signal(signal) => {
            panic!("expected invalid payload, got {signal:?}")
        }
    }
    assert!(!run.terminal_output.contains("not-valid-json"));
}

#[test]
fn harness_contract_lists_every_milestone_scenario_once() {
    const CONTRACT: &str =
        include_str!("../../specs/milestone-87-agent-tui-cutover/acceptance-contract.md");
    const SCENARIOS: &[&str] = &[
        "ATUI-000", "ATUI-010", "ATUI-011", "ATUI-012", "ATUI-020", "ATUI-021", "ATUI-030",
        "ATUI-040", "ATUI-041", "ATUI-042", "ATUI-050",
    ];

    for scenario in SCENARIOS {
        assert_eq!(
            CONTRACT
                .lines()
                .filter(|line| line.starts_with(&format!("| {scenario} |")))
                .count(),
            1,
            "{scenario} must have exactly one canonical contract row"
        );
    }
}

#[test]
fn integration_branch_runs_ci_without_becoming_a_release_source() {
    const CI: &str = include_str!("../../.github/workflows/ci.yml");
    const CODEQL: &str = include_str!("../../.github/workflows/codeql.yml");
    const AUTO_TAG: &str = include_str!("../../.github/workflows/auto-tag.yml");
    const RELEASE: &str = include_str!("../../.github/workflows/release.yml");

    assert_eq!(
        CI.matches("branches: [main, feature/milestone/87]").count(),
        2
    );
    assert_eq!(
        CODEQL
            .matches("branches: [main, feature/milestone/87]")
            .count(),
        2
    );
    assert!(CI.contains("cargo test --locked --test agent_tui_harness"));
    assert!(AUTO_TAG.contains("branches: [main]"));
    assert!(!AUTO_TAG.contains("feature/milestone/87"));
    assert!(RELEASE.contains("tags: ['v*']"));
    assert!(!RELEASE.contains("feature/milestone/87"));
}
