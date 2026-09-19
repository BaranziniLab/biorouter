use crate::bench_session::BenchAgent;
use crate::bench_work_dir::BenchmarkWorkDir;
use crate::eval_suites::{
    collect_baseline_metrics, metrics_hashmap_to_vec, EvalMetricValue, Evaluation,
    ExtensionRequirements,
};
use crate::register_evaluation;
use async_trait::async_trait;
use biorouter::conversation::message::MessageContent;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug)]
pub struct ComputerUseImage {}

impl ComputerUseImage {
    pub fn new() -> Self {
        ComputerUseImage {}
    }
}

#[async_trait]
impl Evaluation for ComputerUseImage {
    async fn run(
        &self,
        agent: &mut BenchAgent,
        _run_loc: &mut BenchmarkWorkDir,
    ) -> anyhow::Result<Vec<(String, EvalMetricValue)>> {
        let (messages, perf_metrics) = collect_baseline_metrics(
            agent,
            "Take a screenshot of the display 0 and describe what you see.".to_string(),
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
                (call.name == "computercontroller__screen_capture"
                    && call
                        .arguments
                        .as_ref()
                        .and_then(|arguments| arguments.get("display"))
                        .and_then(Value::as_u64)
                        == Some(0))
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
                                content.as_image().is_some_and(|image| {
                                    image.mime_type.starts_with("image/") && !image.data.is_empty()
                                })
                            })
                    })
            });

        metrics.push((
            "Take a screenshot and upload images".to_string(),
            EvalMetricValue::Boolean(succeeded),
        ));
        Ok(metrics)
    }

    fn name(&self) -> &str {
        "computer_use_image"
    }

    fn required_extensions(&self) -> ExtensionRequirements {
        ExtensionRequirements {
            builtin: vec!["computercontroller".to_string()],
            external: Vec::new(),
            streamable_http: Vec::new(),
        }
    }
}

register_evaluation!(ComputerUseImage);
