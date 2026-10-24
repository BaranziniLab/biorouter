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
}

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
