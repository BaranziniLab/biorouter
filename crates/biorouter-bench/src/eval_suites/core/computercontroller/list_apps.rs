use crate::bench_session::BenchAgent;
use crate::bench_work_dir::BenchmarkWorkDir;
use crate::eval_suites::{
    collect_baseline_metrics, metrics_hashmap_to_vec, EvalMetricValue, Evaluation,
    ExtensionRequirements,
};
use crate::register_evaluation;
use async_trait::async_trait;
use biorouter::conversation::message::MessageContent;
use std::collections::HashSet;

#[derive(Debug)]
pub struct ComputerUseListApps {}

impl ComputerUseListApps {
    pub fn new() -> Self {
        ComputerUseListApps {}
    }
}

#[async_trait]
impl Evaluation for ComputerUseListApps {
    async fn run(
        &self,
        agent: &mut BenchAgent,
        _run_loc: &mut BenchmarkWorkDir,
    ) -> anyhow::Result<Vec<(String, EvalMetricValue)>> {
        let (messages, perf_metrics) = collect_baseline_metrics(
            agent,
            "List the applications currently running on this computer.".to_string(),
        )
        .await;

        let mut metrics = metrics_hashmap_to_vec(perf_metrics);

        let requests: HashSet<&str> = messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|content| {
                let MessageContent::ToolRequest(request) = content else {
                    return None;
                };
                let Ok(call) = &request.tool_call else {
                    return None;
                };
                (call.name == "computercontroller__list_apps"
                    && call
                        .arguments
                        .as_ref()
                        .is_none_or(|arguments| arguments.is_empty()))
                .then_some(request.id.as_str())
            })
            .collect();
        let succeeded = messages
            .iter()
            .flat_map(|message| &message.content)
            .any(|content| {
                let MessageContent::ToolResponse(response) = content else {
                    return false;
                };
                requests.contains(response.id.as_str())
                    && response.tool_result.as_ref().is_ok_and(|result| {
                        result.is_error != Some(true)
                            && result.content.iter().any(|content| {
                                content
                                    .as_text()
                                    .is_some_and(|text| !text.text.trim().is_empty())
                            })
                    })
            });

        metrics.push((
            "Discover native desktop applications".to_string(),
            EvalMetricValue::Boolean(succeeded),
        ));
        Ok(metrics)
    }

    fn name(&self) -> &str {
        "computer_use_list_apps"
    }

    fn required_extensions(&self) -> ExtensionRequirements {
        ExtensionRequirements {
            builtin: vec!["computercontroller".to_string()],
            external: Vec::new(),
            streamable_http: Vec::new(),
        }
    }
}

register_evaluation!(ComputerUseListApps);
