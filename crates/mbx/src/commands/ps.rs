//! `minibox ps` — list all containers.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Column widths for the table output.
const COL_ID: usize = 14;
const COL_NAME: usize = 16;
const COL_IMAGE: usize = 20;
const COL_COMMAND: usize = 20;
const COL_STATE: usize = 10;
const COL_CREATED: usize = 25;
const COL_PID: usize = 8;

/// Format the table header line.
pub fn format_header() -> String {
    format!(
        "{:<width_id$}  {:<width_name$}  {:<width_image$}  {:<width_cmd$}  \
         {:<width_state$}  {:<width_created$}  {:<width_pid$}",
        "CONTAINER ID",
        "NAME",
        "IMAGE",
        "COMMAND",
        "STATE",
        "CREATED",
        "PID",
        width_id = COL_ID,
        width_name = COL_NAME,
        width_image = COL_IMAGE,
        width_cmd = COL_COMMAND,
        width_state = COL_STATE,
        width_created = COL_CREATED,
        width_pid = COL_PID,
    )
}

/// Format a single container row.
pub fn format_row(c: &minibox_core::protocol::ContainerInfo) -> String {
    let pid_str = c
        .pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    let name_str = c.name.as_deref().unwrap_or("-").to_string();

    let name = truncate(&name_str, COL_NAME);
    let image = truncate(&c.image, COL_IMAGE);
    let command = truncate(&c.command, COL_COMMAND);
    let created = truncate(&c.created_at, COL_CREATED);

    format!(
        "{:<width_id$}  {:<width_name$}  {:<width_image$}  {:<width_cmd$}  \
         {:<width_state$}  {:<width_created$}  {:<width_pid$}",
        c.id,
        name,
        image,
        command,
        c.state,
        created,
        pid_str,
        width_id = COL_ID,
        width_name = COL_NAME,
        width_image = COL_IMAGE,
        width_cmd = COL_COMMAND,
        width_state = COL_STATE,
        width_created = COL_CREATED,
        width_pid = COL_PID,
    )
}

/// Execute the `ps` subcommand.
///
/// Prints a formatted table of all containers known to the daemon.
pub async fn execute(socket_path: &std::path::Path) -> anyhow::Result<()> {
    let request = DaemonRequest::List;

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ContainerList { containers } => {
                // Header
                println!("{}", format_header());

                // Separator
                println!(
                    "{}",
                    "-".repeat(
                        COL_ID
                            + COL_NAME
                            + COL_IMAGE
                            + COL_COMMAND
                            + COL_STATE
                            + COL_CREATED
                            + COL_PID
                            + 12
                    )
                );

                for c in &containers {
                    println!("{}", format_row(c));
                }

                if containers.is_empty() {
                    println!("(no containers)");
                }

                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}

/// Return a string slice of at most `max` characters, appending "…" if
/// truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;
    use minibox_core::protocol::ContainerInfo;

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_prints_empty_list() {
        let (_tmp, socket_path) = setup(DaemonResponse::ContainerList { containers: vec![] }).await;
        let result = execute(&socket_path).await;
        assert!(
            result.is_ok(),
            "execute should succeed with empty list: {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_prints_container_row() {
        let container = ContainerInfo {
            id: "abc123456789".to_string(),
            name: None,
            image: "alpine".to_string(),
            command: "/bin/sh".to_string(),
            state: "running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(42),
        };
        let (_tmp, socket_path) = setup(DaemonResponse::ContainerList {
            containers: vec![container],
        })
        .await;
        let result = execute(&socket_path).await;
        assert!(
            result.is_ok(),
            "execute should succeed with one container: {result:?}"
        );
    }

    /// Verify that the NAME column header is present in ps output.
    /// This test checks the format_header function produces the NAME column.
    #[test]
    fn ps_header_contains_name_column() {
        let header = format_header();
        assert!(
            header.contains("NAME"),
            "ps header should contain NAME column, got: {header}"
        );
    }

    /// Verify that a container with a name shows it in the formatted row.
    #[test]
    fn ps_row_shows_container_name() {
        let info = ContainerInfo {
            id: "abc123".to_string(),
            name: Some("my-web".to_string()),
            image: "nginx".to_string(),
            command: "/bin/nginx".to_string(),
            state: "running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(99),
        };
        let row = format_row(&info);
        assert!(
            row.contains("my-web"),
            "row should contain the container name, got: {row}"
        );
    }

    /// Verify that a container without a name shows '-' in the NAME column.
    #[test]
    fn ps_row_shows_dash_when_no_name() {
        let info = ContainerInfo {
            id: "abc123".to_string(),
            name: None,
            image: "alpine".to_string(),
            command: "/bin/sh".to_string(),
            state: "running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(1),
        };
        let row = format_row(&info);
        // The NAME column should contain '-' for unnamed containers.
        assert!(
            row.contains('-'),
            "row should contain '-' for unnamed container, got: {row}"
        );
    }

    #[test]
    fn truncate_empty_string_unchanged() {
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_one_over_limit_adds_ellipsis() {
        // 6 chars, max 5 → take 4 + "…"
        assert_eq!(truncate("hello!", 5), "hell…");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        assert_eq!(truncate("hello world", 8), "hello w…");
    }

    #[test]
    fn truncate_max_one_produces_single_ellipsis() {
        // max=1, saturating_sub(1)=0 → take 0 chars, push "…"
        assert_eq!(truncate("ab", 1), "…");
    }

    #[test]
    fn truncate_counts_unicode_chars_not_bytes() {
        // "café" is 4 chars but 5 bytes; should be treated as 4 chars
        assert_eq!(truncate("café", 4), "café");
        assert_eq!(truncate("café!", 4), "caf…");
    }

    // ── Property-based tests ──────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        /// Result length never exceeds `max` chars.
        #[test]
        fn prop_truncate_never_exceeds_max(s in ".*", max in 1_usize..=40) {
            let result = truncate(&s, max);
            let char_count = result.chars().count();
            prop_assert!(
                char_count <= max,
                "truncate({s:?}, {max}) produced {char_count} chars: {result:?}"
            );
        }

        /// Input at or below `max` chars is returned unchanged.
        #[test]
        fn prop_truncate_short_input_unchanged(s in "[a-zA-Z0-9 ]{0,20}", max in 20_usize..=40) {
            // s is at most 20 chars, max is at least 20 — so s.len() <= max always.
            let result = truncate(&s, max);
            prop_assert_eq!(&result, &s);
        }

        /// Truncated output always ends with the ellipsis character.
        #[test]
        fn prop_truncate_long_input_ends_with_ellipsis(
            s in "[a-zA-Z]{50,60}",
            max in 1_usize..=20,
        ) {
            // s is at least 50 chars, max at most 20 — always triggers truncation.
            let result = truncate(&s, max);
            prop_assert!(
                result.ends_with('…'),
                "expected ellipsis at end of {result:?}"
            );
        }

        /// `format_row` always contains the container id.
        #[test]
        fn prop_format_row_contains_id(id in "[a-z0-9]{1,14}") {
            let info = ContainerInfo {
                id: id.clone(),
                name: None,
                image: "alpine".to_string(),
                command: "/bin/sh".to_string(),
                state: "running".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                pid: None,
            };
            let row = format_row(&info);
            prop_assert!(row.contains(&id), "row missing id={id:?}: {row:?}");
        }

        /// `format_row` always contains the state.
        #[test]
        fn prop_format_row_contains_state(state in "[a-z]{3,10}") {
            let info = ContainerInfo {
                id: "abc1".to_string(),
                name: None,
                image: "alpine".to_string(),
                command: "/bin/sh".to_string(),
                state: state.clone(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                pid: None,
            };
            let row = format_row(&info);
            prop_assert!(row.contains(&state), "row missing state={state:?}: {row:?}");
        }
    }
}
