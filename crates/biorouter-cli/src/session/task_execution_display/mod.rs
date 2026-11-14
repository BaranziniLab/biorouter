use biorouter::agents::subagent_execution_tool::lib::TaskStatus;
use biorouter::agents::subagent_execution_tool::notification_events::{
    TaskExecutionNotificationEvent, TaskInfo,
};
use biorouter::utils::safe_truncate;
use console::{style, Color, Term};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
mod tests;

const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";
const MOVE_TO_PROGRESS_LINE: &str = "\x1b[4;1H";
const CLEAR_TO_EOL: &str = "\x1b[K";
const CLEAR_BELOW: &str = "\x1b[J";
pub const TASK_EXECUTION_NOTIFICATION_TYPE: &str = "task_execution";

/// Brand warm tan-brown accent (xterm-256 137 ≈ #af875f), Biorouter's light cream palette
const ACCENT: Color = Color::Color256(137);

static INITIAL_SHOWN: AtomicBool = AtomicBool::new(false);

/// A glyph + colored status word for a task, e.g. a green `●` + `completed`.
/// The literal status word is preserved (un-styled in non-tty contexts) so it
/// stays greppable in logs and tests.
fn status_badge(status: &TaskStatus) -> String {
    let (glyph, word, color) = match status {
        TaskStatus::Pending => ("○", "pending", Color::Color256(244)),
        TaskStatus::Running => ("◐", "running", Color::Yellow),
        TaskStatus::Completed => ("●", "completed", Color::Green),
        // Red is rare (design.md 9.14): a failed task keeps its ✗ and its word, in
        // the terminal's own ink and bold, so it reads as failed without an alarm.
        TaskStatus::Failed => return format!("{} {}", style("✗").bold(), style("failed").bold()),
    };
    format!("{} {}", style(glyph).fg(color), style(word).fg(color))
}

/// A dim horizontal rule sized to the terminal width (capped for readability).
fn rule() -> String {
    let width = Term::stdout()
        .size_checked()
        .map(|(_h, w)| w as usize)
        .unwrap_or(80)
        .clamp(24, 100);
    style("─".repeat(width)).dim().to_string()
}

fn format_result_data_for_display(result_data: &Value) -> String {
    match result_data {
        Value::String(s) => s.to_string(),
        Value::Object(obj) => {
            if let Some(partial_output) = obj.get("partial_output").and_then(|v| v.as_str()) {
                format!("Partial output: {}", partial_output)
            } else {
                serde_json::to_string_pretty(obj).unwrap_or_default()
            }
        }
        Value::Array(arr) => serde_json::to_string_pretty(arr).unwrap_or_default(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "null".to_string(),
    }
}

fn process_output_for_display(output: &str) -> String {
    const MAX_OUTPUT_LINES: usize = 2;
    const OUTPUT_PREVIEW_LENGTH: usize = 100;

    let lines: Vec<&str> = output.lines().collect();
    let recent_lines = if lines.len() > MAX_OUTPUT_LINES {
        &lines[lines.len() - MAX_OUTPUT_LINES..]
    } else {
        &lines
    };

    let clean_output = recent_lines.join(" ... ");
    safe_truncate(&clean_output, OUTPUT_PREVIEW_LENGTH)
}

pub fn format_task_execution_notification(
    data: &Value,
) -> Option<(String, Option<String>, Option<String>)> {
    if let Ok(event) = serde_json::from_value::<TaskExecutionNotificationEvent>(data.clone()) {
        return Some(match event {
            TaskExecutionNotificationEvent::LineOutput { output, .. } => (
                format!("{}\n", output),
                None,
                Some(TASK_EXECUTION_NOTIFICATION_TYPE.to_string()),
            ),
            TaskExecutionNotificationEvent::TasksUpdate { .. } => {
                let formatted_display = format_tasks_update_from_event(&event);
                (
                    formatted_display,
                    None,
                    Some(TASK_EXECUTION_NOTIFICATION_TYPE.to_string()),
                )
            }
            TaskExecutionNotificationEvent::TasksComplete { .. } => {
                let formatted_summary = format_tasks_complete_from_event(&event);
                (
                    formatted_summary,
                    None,
                    Some(TASK_EXECUTION_NOTIFICATION_TYPE.to_string()),
                )
            }
        });
    }
    None
}

fn format_tasks_update_from_event(event: &TaskExecutionNotificationEvent) -> String {
    if let TaskExecutionNotificationEvent::TasksUpdate { stats, tasks } = event {
        let mut display = String::new();

        // Header occupies exactly three lines so `MOVE_TO_PROGRESS_LINE` (row 4)
        // keeps landing on the progress line across in-place redraws.
        if !INITIAL_SHOWN.swap(true, Ordering::SeqCst) {
            display.push_str(CLEAR_SCREEN);
            display.push_str(&format!(
                "{} {}\n",
                style("▌").fg(ACCENT),
                style("Task execution").bold()
            ));
            display.push_str(&format!("{}\n\n", rule()));
        } else {
            display.push_str(MOVE_TO_PROGRESS_LINE);
        }

        display.push_str(&format!(
            "Progress: {} total {} {} pending {} {} running {} {} completed {} {} failed",
            stats.total,
            style("·").dim(),
            stats.pending,
            style("·").dim(),
            style(stats.running).yellow(),
            style("·").dim(),
            style(stats.completed).green(),
            style("·").dim(),
            style(stats.failed).red(),
        ));
        display.push_str(&format!("{}\n\n", CLEAR_TO_EOL));

        let mut sorted_tasks = tasks.clone();
        sorted_tasks.sort_by(|a, b| a.id.cmp(&b.id));

        for task in sorted_tasks {
            display.push_str(&format_task_display(&task));
        }

        display.push_str(CLEAR_BELOW);
        display
    } else {
        String::new()
    }
}

fn format_tasks_complete_from_event(event: &TaskExecutionNotificationEvent) -> String {
    if let TaskExecutionNotificationEvent::TasksComplete {
        stats,
        failed_tasks,
    } = event
    {
        let mut summary = String::new();
        summary.push_str(&format!(
            "{} {}\n",
            style("▌").fg(ACCENT),
            style("Execution complete").bold()
        ));
        summary.push_str(&format!("{}\n", rule()));

        summary.push_str(&format!("Total Tasks: {}\n", stats.total));
        summary.push_str(&format!("Completed: {}\n", style(stats.completed).green()));
        summary.push_str(&format!("Failed: {}\n", style(stats.failed).red()));
        summary.push_str(&format!("Success Rate: {:.1}%\n", stats.success_rate));

        if !failed_tasks.is_empty() {
            summary.push_str(&format!("\n{}\n", style("Failed Tasks:").red()));
            for task in failed_tasks {
                summary.push_str(&format!("   {} {}\n", style("✗").red(), task.name));
                if let Some(error) = &task.error {
                    summary.push_str(&format!("     {} {}\n", style("Error:").dim(), error));
                }
            }
        }

        summary.push_str(&format!("\n{}\n", style("Generating summary...").dim()));
        summary
    } else {
        String::new()
    }
}

/// Render a representative block of task rows (one per status) for the TUI
/// preview harness. Not used in normal operation.
pub fn preview_block() -> String {
    use serde_json::json;
    let samples = vec![
        TaskInfo {
            id: "1".into(),
            status: TaskStatus::Completed,
            duration_secs: Some(2.3),
            current_output: String::new(),
            task_type: "sub_workflow".into(),
            task_name: "ingest-pubmed".into(),
            task_metadata: String::new(),
            error: None,
            result_data: Some(json!({"partial_output": "412 records indexed"})),
        },
        TaskInfo {
            id: "2".into(),
            status: TaskStatus::Running,
            duration_secs: Some(1.5),
            current_output: "scoring candidates...\nalmost done".into(),
            task_type: "text_instruction".into(),
            task_name: "rank-targets".into(),
            task_metadata: "top_k=20".into(),
            error: None,
            result_data: None,
        },
        TaskInfo {
            id: "3".into(),
            status: TaskStatus::Pending,
            duration_secs: None,
            current_output: String::new(),
            task_type: "sub_workflow".into(),
            task_name: "summarize".into(),
            task_metadata: String::new(),
            error: None,
            result_data: None,
        },
        TaskInfo {
            id: "4".into(),
            status: TaskStatus::Failed,
            duration_secs: None,
            current_output: String::new(),
            task_type: "sub_workflow".into(),
            task_name: "fetch-uniprot".into(),
            task_metadata: String::new(),
            error: Some("connection timed out after 3 retries".into()),
            result_data: None,
        },
    ];

    let mut out = format!(
        "{} {}\n{}\n\n",
        style("▌").fg(ACCENT),
        style("Task execution").bold(),
        rule()
    );
    out.push_str(&format!(
        "Progress: 4 total {} 1 pending {} {} running {} {} completed {} {} failed\n\n",
        style("·").dim(),
        style("·").dim(),
        style(1).yellow(),
        style("·").dim(),
        style(1).green(),
        style("·").dim(),
        style(1).red(),
    ));
    for task in &samples {
        out.push_str(&format_task_display(task).replace(CLEAR_TO_EOL, ""));
    }
    out
}

fn format_task_display(task: &TaskInfo) -> String {
    let mut task_display = String::new();

    // Header:  ● completed  task-name  (task_type)
    task_display.push_str(&format!(
        "{}  {} {}{}\n",
        status_badge(&task.status),
        style(&task.task_name).bold(),
        style(format!("({})", task.task_type)).dim(),
        CLEAR_TO_EOL
    ));

    // A dim, indented sub-line:  "      label  value"
    let mut sub = |label: &str, value: String| {
        task_display.push_str(&format!(
            "      {} {}{}\n",
            style(label).dim(),
            value,
            CLEAR_TO_EOL
        ));
    };

    if !task.task_metadata.is_empty() {
        sub("Parameters:", task.task_metadata.clone());
    }

    if let Some(duration_secs) = task.duration_secs {
        sub("Duration:", format!("{:.1}s", duration_secs));
    }

    if matches!(task.status, TaskStatus::Running) && !task.current_output.trim().is_empty() {
        let processed_output = process_output_for_display(&task.current_output);
        if !processed_output.is_empty() {
            sub("Output:", processed_output);
        }
    }

    if matches!(task.status, TaskStatus::Completed) {
        if let Some(result_data) = &task.result_data {
            let result_preview = format_result_data_for_display(result_data);
            if !result_preview.is_empty() {
                sub("Result:", result_preview);
            }
        }
    }

    if matches!(task.status, TaskStatus::Failed) {
        if let Some(error) = &task.error {
            let error_preview = safe_truncate(error, 80);
            sub("Error:", error_preview.replace('\n', " "));
        }
    }

    task_display.push_str(&format!("{}\n", CLEAR_TO_EOL));
    task_display
}
