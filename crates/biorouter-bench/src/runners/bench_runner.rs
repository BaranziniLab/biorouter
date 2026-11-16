use crate::bench_config::{BenchModel, BenchRunConfig};
use crate::bench_work_dir::BenchmarkWorkDir;
use crate::eval_suites::EvaluationSuite;
use crate::runners::model_runner::ModelRunner;
use crate::utilities::{await_process_exits, first_failure, parallel_bench_cmd};
use anyhow::Context;
use std::path::PathBuf;

#[derive(Clone)]
pub struct BenchRunner {
    config: BenchRunConfig,
}

impl BenchRunner {
    pub fn new(config_path: PathBuf) -> anyhow::Result<BenchRunner> {
        let config = BenchRunConfig::from(config_path.clone())?;

        let resolved_output_dir = match &config.output_dir {
            Some(path) => {
                if !path.is_absolute() {
                    anyhow::bail!(
                         "Config Error in '{}': 'output_dir' must be an absolute path, but found relative path: {}",
                         config_path.display(),
                         path.display()
                     );
                }
                path.clone()
            }
            None => std::env::current_dir().context(
                "Failed to get current working directory to use as default output directory",
            )?,
        };

        BenchmarkWorkDir::init_experiment(resolved_output_dir)?;

        config.save("config.cfg".to_string());
        Ok(BenchRunner { config })
    }

    pub fn from(config: String) -> anyhow::Result<BenchRunner> {
        let config = BenchRunConfig::from_string(config)?;
        Ok(BenchRunner { config })
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        // split models that must run serial from those that can be run in parallel
        let (parallel_models, serial_models): &(Vec<BenchModel>, Vec<BenchModel>) = &self
            .config
            .models
            .clone()
            .into_iter()
            .partition(|model| model.parallel_safe);

        // exec parallel models
        let mut parallel_models_handle = Vec::new();
        for model in parallel_models {
            self.config.models = vec![model.clone()];
            let cfg = self.config.to_string()?;
            let model_handle = parallel_bench_cmd("eval-model".to_string(), cfg, Vec::new());
            parallel_models_handle.push(model_handle);
        }

        // exec serial models. One that fails does not stop the others, and the
        // parallel ones are waited for either way; the run fails at the end, so
        // `bench run` exits non-zero when any model's evals did not run.
        let mut outcomes = Vec::new();
        for model in serial_models {
            self.config.models = vec![model.clone()];
            let ran = self
                .config
                .to_string()
                .and_then(ModelRunner::from)
                .and_then(|runner| runner.run());
            outcomes.push(
                ran.with_context(|| format!("The model {}/{} failed", model.provider, model.name)),
            );
        }

        outcomes.push(
            await_process_exits(&mut parallel_models_handle, Vec::new())
                .context("A model run in parallel failed"),
        );

        first_failure(outcomes)
    }

    pub fn list_selectors(_config: Option<PathBuf>) -> anyhow::Result<()> {
        let selector_eval_counts = EvaluationSuite::available_selectors();
        let mut keys: Vec<_> = selector_eval_counts.keys().collect();
        keys.sort();
        let max_key_len = keys.iter().map(|k| k.len()).max().unwrap_or(0);
        println!(
            "selector {} => Eval Count",
            " ".repeat(max_key_len - "selector".len())
        );
        println!("{}", "-".repeat(max_key_len + 6));
        for selector in keys {
            println!(
                "{} {} => {}",
                selector,
                " ".repeat(max_key_len - selector.len()),
                selector_eval_counts.get(selector).unwrap()
            );
        }
        Ok(())
    }
}
