use std::collections::HashMap;

use anstream::println;
use biorouter::workflow::{Workflow, BUILT_IN_WORKFLOW_DIR_PARAM};
use console::style;

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
/// per item ([`biorouter::workflow::supervision::summary_lines`]), then what
/// running it does with them. Empty when it declares neither.
fn supervision_lines(workflow: &Workflow) -> Vec<String> {
    let summary = biorouter::workflow::supervision::summary_lines(workflow);
    if summary.is_empty() {
        return summary;
    }
    // The lines name a model by `provider/model` and a Baseten one by its
    // slug; the terminal names it by its alias and where it runs.
    let mut lines: Vec<String> = summary
        .iter()
        .map(|line| format!("   - {}", crate::commands::decide::person_model_names(line)))
        .collect();
    lines.push(format!("   {SUPERVISION_APPLIES}"));
    lines
}

/// The note under the Supervision section: when the blocks apply.
const SUPERVISION_APPLIES: &str = "These apply once, to the new chat this workflow starts, before \
     its first message. A monitor's decision model reads the newest messages of that chat.";

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

    /// Item 8: `biorouter run --workflow X --explain` lists the monitors and
    /// phase gates a workflow declares, so a person sees what will watch the
    /// chat before it starts. A workflow without them prints no section.
    #[test]
    fn explain_lists_monitors_and_gates() {
        let workflow = Workflow::from_content(
            "version: 1.0.0\ntitle: Count\ndescription: probe\nprompt: test\n\
             decision_settings:\n  defaultModel: {provider: baseten_decision, model: joey-1}\n  \
             monitors:\n  - id: reached-ten\n    name: Reached ten\n    question:\n      \
             instructions: Has the agent already echoed 10 or a larger number?\n      \
             kind: {binary: {}}\n    threshold: 0.7\n    action: stop\n    onError: pause\n\
             orchestration_plan:\n  title: Count plan\n  phases:\n  \
             - {id: count, title: Count, kind: research}\n  - id: check\n    title: Check\n    \
             kind: verification\n    gate:\n      questions: {done: Did it stop at ten?}\n      \
             threshold: 0.8\n      max_attempts: 2\n",
        )
        .expect("a valid supervised workflow");

        let lines = supervision_lines(&workflow);
        let text = lines.join("\n");
        for expected in [
            "   - Default decision model: joey-1 · Baseten",
            "   - Monitor \"Reached ten\": model chat default; checked after each step; fires \
             at 0.70 or above; when it fires: stop; on an error: pause",
            "   - Phase plan \"Count plan\": 2 phases",
            "   - Phase \"check\" gate: 1 question, threshold 0.80, 2 attempts; model chat default",
        ] {
            assert!(
                lines.iter().any(|line| line == expected),
                "missing {expected:?} in:\n{text}"
            );
        }
        assert_eq!(
            lines.last().map(String::as_str),
            Some(format!("   {SUPERVISION_APPLIES}").as_str())
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
}
