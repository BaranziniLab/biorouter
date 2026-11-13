use std::collections::HashMap;

use anstream::println;
use biorouter::agents::orchestration_plan::DEFAULT_GATE_THRESHOLD;
use biorouter::decision::settings::MonitorCriterion;
use biorouter::workflow::supervision::{WorkflowGate, WorkflowPlan};
use biorouter::workflow::{Workflow, BUILT_IN_WORKFLOW_DIR_PARAM};
use console::style;

use crate::commands::{decide, session_monitors};

pub fn print_workflow_explanation(workflow: &Workflow) {
    println!(
        "{} {}",
        style("🔍 Loading workflow:").bold().green(),
        style(&workflow.title).green()
    );
    println!("{}", style("📄 Description:").bold());
    println!("   {}", workflow.description);
    if let Some(params) = &workflow.parameters {
        if !params.is_empty() {
            println!("{}", style("⚙️  Workflow Parameters:").bold());
            for param in params {
                let default_display = match &param.default {
                    Some(val) => format!(" (default: {})", val),
                    None => String::new(),
                };

                println!(
                    "   - {} ({}, {}){}: {}",
                    style(&param.key).cyan(),
                    param.input_type,
                    param.requirement,
                    default_display,
                    param.description
                );
            }
        }
    }
    let supervision = supervision_lines(workflow);
    if !supervision.is_empty() {
        println!("{}", style("🛡️  Supervision:").bold());
        for line in supervision {
            println!("{line}");
        }
    }
}

/// The Supervision section of `--explain`: what the workflow's
/// `decision_settings` and `orchestration_plan` declare, one indented line
/// per item, then when they apply and when their models are checked. Empty
/// when it declares neither.
///
/// Built here rather than from `supervision::summary_lines`, because the
/// terminal says what a monitor does in the words `biorouter session
/// monitors` uses (H-9): when it is checked, what it asks and of which
/// model, when it fires and what it then does. Every model is named by its
/// alias and where it runs, as everywhere else in the terminal.
fn supervision_lines(workflow: &Workflow) -> Vec<String> {
    let mut items = Vec::new();
    // A monitor or gate that names no model asks the chat's default, which
    // is this workflow's when it declares one.
    let default_model = workflow
        .decision_settings
        .as_ref()
        .and_then(|declared| declared.default_model.as_ref())
        .map(decide::model_for_person);
    let fallback = default_model
        .clone()
        .unwrap_or_else(|| "the chat's default decision model".to_string());
    if let Some(declared) = &workflow.decision_settings {
        if let Some(model) = &default_model {
            items.push(format!("Default decision model: {model}"));
        }
        items.extend(
            declared
                .monitors
                .iter()
                .map(|monitor| monitor_line(monitor, &fallback)),
        );
    }
    if let Some(plan) = &workflow.orchestration_plan {
        items.push(plan_line(plan));
        for phase in &plan.phases {
            if let Some(gate) = &phase.gate {
                items.push(gate_line(&phase.id, gate, &fallback));
            }
            let monitors = phase.monitors.as_ref().map_or(0, Vec::len);
            if monitors > 0 {
                items.push(format!(
                    "Phase \"{}\" starts its subagents with {}",
                    decide::clip(&phase.id, 40),
                    plural(monitors, "monitor", "monitors")
                ));
            }
        }
    }
    if items.is_empty() {
        return items;
    }
    let mut lines: Vec<String> = items
        .into_iter()
        .map(|item| format!("   - {item}"))
        .collect();
    lines.push(format!("   {SUPERVISION_APPLIES}"));
    lines.push(format!("   {MODELS_CHECKED_AT_START}"));
    lines
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `Monitor 'Off task': after each iteration, asks joey-1 · Baseten "…";
/// when P(yes) ≥ 0.80, it stops the turn (re-arms below 0.40); an error
/// pauses the turn`. `default_model` names the model it asks when it names
/// none.
fn monitor_line(monitor: &MonitorCriterion, default_model: &str) -> String {
    let model = match &monitor.model {
        Some(model) => decide::model_for_person(model),
        None => default_model.to_string(),
    };
    let in_a_row = if monitor.consecutive > 1 {
        format!(" on {} checks in a row", monitor.consecutive)
    } else {
        String::new()
    };
    let rearm = session_monitors::rearm_text(monitor)
        .map(|rearm| format!(" ({rearm})"))
        .unwrap_or_default();
    let mut line = format!(
        "Monitor '{}'{}: {}, asks {model} \"{}\"; when {}{in_a_row}, it {}{rearm}; {}",
        decide::clip(&monitor.name, 60),
        if monitor.enabled { "" } else { " (off)" },
        session_monitors::timing_text(monitor.timing),
        decide::clip(&monitor.question.instructions, 120),
        session_monitors::condition_text(monitor),
        session_monitors::action_text(monitor.action),
        session_monitors::error_text(monitor.on_error),
    );
    if monitor.apply_to_subagents {
        line.push_str("; also watches the subagents this chat starts");
    }
    line
}

/// `Phase plan "Screen and verify": 2 phases: screen, verify`. Every phase
/// is named, not only the gated ones.
fn plan_line(plan: &WorkflowPlan) -> String {
    let ids: Vec<String> = plan
        .phases
        .iter()
        .map(|phase| decide::clip(&phase.id, 40))
        .collect();
    format!(
        "Phase plan \"{}\": {}: {}",
        decide::clip(&plan.title, 80),
        plural(plan.phases.len(), "phase", "phases"),
        ids.join(", ")
    )
}

/// `Phase "verify" gate: 1 question, each must reach P(yes) ≥ 0.80 before
/// the phase completes, 2 attempts; asked of the chat's default decision
/// model`.
fn gate_line(phase: &str, gate: &WorkflowGate, default_model: &str) -> String {
    let model = match gate.model.as_deref() {
        Some(model) => decide::person_model_names(&decide::terminal_text(model)),
        None => default_model.to_string(),
    };
    format!(
        "Phase \"{}\" gate: {}, each must reach P(yes) ≥ {} before the phase completes, {}{}; \
         asked of {model}",
        decide::clip(phase, 40),
        plural(gate.questions.len(), "question", "questions"),
        decide::format_p(gate.threshold.unwrap_or(DEFAULT_GATE_THRESHOLD)),
        plural(
            gate.max_attempts.unwrap_or(DEFAULT_GATE_ATTEMPTS) as usize,
            "attempt",
            "attempts"
        ),
        if gate.allow_uncalibrated == Some(true) {
            ", accepts uncalibrated answers"
        } else {
            ""
        }
    )
}

/// The attempts a gate gets when the workflow names none: the plan's own
/// default, which a test pins.
const DEFAULT_GATE_ATTEMPTS: u32 = 3;

/// The note under the Supervision section: when the blocks apply.
const SUPERVISION_APPLIES: &str = "These apply once, to the new chat this workflow starts, before \
     its first message. A monitor's decision model reads the newest messages of that chat.";

/// The second note: what `--explain` has not checked (H-3).
const MODELS_CHECKED_AT_START: &str = "Each decision model is checked when the run starts: that \
     it is installed or reachable, and that the chat may use it.";

pub fn print_parameters_with_values(params: HashMap<String, String>) {
    for (key, value) in params {
        println!("{}", parameter_with_value(&key, &value));
    }
}

fn parameter_with_value(key: &str, value: &str) -> String {
    let label = if key == BUILT_IN_WORKFLOW_DIR_PARAM {
        " (built-in)"
    } else {
        ""
    };
    format!("   {}{}: {}", key, label, value)
}

pub fn print_required_parameters_for_template(
    params_for_template: HashMap<String, String>,
    missing_params: Vec<String>,
) {
    if !params_for_template.is_empty() {
        println!(
            "{}",
            style("📥 Parameters used to load this workflow:").bold()
        );
        print_parameters_with_values(params_for_template)
    }
    if !missing_params.is_empty() {
        println!(
            "{}",
            style("🔴 Missing parameters in the command line if you want to run the workflow:")
                .bold()
        );
        for param in missing_params.iter() {
            println!("   - {}", param);
        }
        println!(
            "📩 {}:",
            style("Please provide the following parameters in the command line if you want to run the workflow:").bold()
        );
        println!("  {}", missing_parameters_command_line(missing_params));
    }
}

pub fn missing_parameters_command_line(missing_params: Vec<String>) -> String {
    missing_params
        .iter()
        .map(|key| format!("--params {}=your_value", key))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn print_workflow_info(workflow: &Workflow, params: Vec<(String, String)>) {
    eprintln!(
        "{} {}",
        style("Loading workflow:").green().bold(),
        style(&workflow.title).green()
    );
    eprintln!("{} {}", style("Description:").bold(), &workflow.description);

    if !params.is_empty() {
        eprintln!("{}", style("Parameters used to load this workflow:").bold());
        for (key, value) in params {
            eprintln!("{}", parameter_with_value(&key, &value));
        }
    }
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Item 8 and H-9: `biorouter run --workflow X --explain` lists the
    /// monitors and phase gates a workflow declares, and says what each
    /// monitor does (when it is checked, what it asks of which model, when it
    /// fires and what it does then), naming every model by its alias. A
    /// workflow without them prints no section.
    #[test]
    fn explain_lists_monitors_and_gates() {
        let workflow = Workflow::from_content(
            "version: 1.0.0\ntitle: Count\ndescription: probe\nprompt: test\n\
             decision_settings:\n  defaultModel: {provider: baseten_decision, model: joey-1}\n  \
             monitors:\n  - id: reached-ten\n    name: Reached ten\n    question:\n      \
             instructions: Has the agent already echoed 10 or a larger number?\n      \
             kind: {binary: {}}\n    threshold: 0.7\n    action: stop\n    onError: pause\n\
             \x20 - id: drift\n    name: Drift\n    question:\n      instructions: Is it \
             off task?\n      kind: {binary: {}}\n    model: {provider: ollama_decision, \
             model: \"tev1:0.8b\"}\n    timing: turn_end\n    threshold: 0.3\n    \
             direction: below\n    consecutive: 2\n    action: notify\n    onError: notify\n\
             orchestration_plan:\n  title: Count plan\n  phases:\n  \
             - {id: count, title: Count, kind: research}\n  - id: check\n    title: Check\n    \
             kind: verification\n    gate:\n      questions: {done: Did it stop at ten?}\n      \
             threshold: 0.8\n      max_attempts: 2\n      model: \
             baseten_decision/decision-models/baseten-joey-2b-latest\n",
        )
        .expect("a valid supervised workflow");

        let lines = supervision_lines(&workflow);
        let text = lines.join("\n");
        for expected in [
            "   - Default decision model: joey-1 · Baseten",
            "   - Monitor 'Reached ten': after each iteration, asks joey-1 · Baseten \"Has the \
             agent already echoed 10 or a larger number?\"; when P(yes) ≥ 0.70, it stops the turn \
             (stops again while it holds); an error pauses the turn",
            "   - Monitor 'Drift': as the turn ends, asks tev1:0.8b · this computer \"Is it off \
             task?\"; when P(yes) < 0.30 on 2 checks in a row, it shows a notice; an error shows \
             a notice",
            "   - Phase plan \"Count plan\": 2 phases: count, check",
            "   - Phase \"check\" gate: 1 question, each must reach P(yes) ≥ 0.80 before the \
             phase completes, 2 attempts; asked of joey-1 · Baseten",
        ] {
            assert!(
                lines.iter().any(|line| line == expected),
                "missing {expected:?} in:\n{text}"
            );
        }
        assert!(!text.contains("_decision"), "no provider key: {text}");
        assert_eq!(
            lines[lines.len() - 2..],
            [
                format!("   {SUPERVISION_APPLIES}"),
                format!("   {MODELS_CHECKED_AT_START}")
            ]
        );

        // No default declared: what names no model asks the chat's default.
        let undeclared = Workflow::from_content(
            "version: 1.0.0\ntitle: Plan\ndescription: probe\nprompt: test\n\
             orchestration_plan:\n  title: P\n  phases:\n  - id: check\n    title: Check\n    \
             gate:\n      questions: {done: Done?}\n",
        )
        .unwrap();
        assert_eq!(
            supervision_lines(&undeclared)[1],
            "   - Phase \"check\" gate: 1 question, each must reach P(yes) ≥ 0.80 before the \
             phase completes, 3 attempts; asked of the chat's default decision model"
        );

        // D-2: a Baseten slug reads as its alias, as every terminal line names it.
        let slugged = Workflow::from_content(
            "version: 1.0.0\ntitle: Slug\ndescription: probe\nprompt: test\n\
             decision_settings:\n  defaultModel: {provider: baseten_decision, model: \
             decision-models/baseten-joey-2b-latest}\n",
        )
        .unwrap();
        let lines = supervision_lines(&slugged);
        assert_eq!(lines[0], "   - Default decision model: joey-1 · Baseten");

        let plain = Workflow::from_content(
            "version: 1.0.0\ntitle: Plain\ndescription: probe\nprompt: test\n",
        )
        .unwrap();
        assert!(supervision_lines(&plain).is_empty());
    }

    /// The attempts a gate gets when the workflow names none are the plan's
    /// own default.
    #[test]
    fn the_default_gate_attempts_are_the_plans() {
        use biorouter::agents::orchestration_plan::{self, OrchestratePhaseInput};
        let workflow = Workflow::from_content(
            "version: 1.0.0\ntitle: Plan\ndescription: probe\nprompt: test\n\
             orchestration_plan:\n  title: P\n  phases:\n  - id: check\n    title: Check\n    \
             gate:\n      questions: {done: Done?}\n",
        )
        .unwrap();
        let declared = workflow.orchestration_plan.unwrap();
        let plan = orchestration_plan::apply_workflow_plan(
            &declared.title,
            None,
            declared
                .phases
                .iter()
                .map(OrchestratePhaseInput::from)
                .collect(),
            chrono::Utc::now(),
        )
        .unwrap();
        let record = orchestration_plan::plan_record(&plan);
        assert_eq!(
            record["phases"][0]["gate"]["maxAttempts"],
            serde_json::json!(DEFAULT_GATE_ATTEMPTS),
            "{record}"
        );
    }
}
