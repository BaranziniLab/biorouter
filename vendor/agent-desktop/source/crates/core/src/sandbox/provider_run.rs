//! One provider script call: the request it builds, and the reply data commands return.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::sandbox::provider_protocol::next_step;
use crate::sandbox::provider_reply::ProviderReply;
use crate::sandbox::provider_script_output::ProviderScriptOutput;
use crate::sandbox::provider_script_request::ProviderScriptRequest;
use crate::sandbox::provider_spec::ProviderSpec;
use crate::{AdapterError, AppError, DeliverySemantics, ErrorCode, PlatformAdapter};

const SCRIPT_FILE: &str = "provider.sh";

/// One verb call against a provider, for a sandbox it owns or for the provider itself.
pub(crate) struct ProviderCall<'a> {
    pub(crate) spec: &'a ProviderSpec,
    pub(crate) provider_dir: &'a Path,
    pub(crate) sandboxes_root: &'a Path,
    pub(crate) sandbox_name: Option<&'a str>,
    pub(crate) settings: &'a [(String, String)],
}

impl<'a> ProviderCall<'a> {
    pub(crate) fn new(
        spec: &'a ProviderSpec,
        provider_dir: &'a Path,
        sandboxes_root: &'a Path,
        sandbox_name: Option<&'a str>,
        settings: &'a [(String, String)],
    ) -> Self {
        Self {
            spec,
            provider_dir,
            sandboxes_root,
            sandbox_name,
            settings,
        }
    }

    pub(crate) fn sandbox_dir(&self) -> Option<PathBuf> {
        self.sandbox_name.map(|name| self.sandboxes_root.join(name))
    }

    /// Runs one verb and returns its raw output, with a deadline failure mapped to a plain error.
    pub(crate) fn run(
        &self,
        adapter: &dyn PlatformAdapter,
        argv: Vec<String>,
        timeout: Duration,
    ) -> Result<ProviderScriptOutput, AppError> {
        let request = self.request(argv, timeout)?;
        match adapter.run_provider_script(&request) {
            Ok(output) => Ok(output),
            Err(error) => Err(AppError::Adapter(self.deadline_error(error, &request.args))),
        }
    }

    fn request(
        &self,
        argv: Vec<String>,
        timeout: Duration,
    ) -> Result<ProviderScriptRequest, AppError> {
        let working_dir = self
            .sandbox_dir()
            .unwrap_or_else(|| self.provider_dir.to_path_buf());
        Ok(ProviderScriptRequest {
            script: self.provider_dir.join(SCRIPT_FILE),
            args: argv,
            env: self.env()?,
            working_dir,
            timeout,
        })
    }

    fn env(&self) -> Result<Vec<(String, String)>, AppError> {
        let mut env = vec![
            (
                "AD_PROVIDER_DIR".to_string(),
                self.provider_dir.display().to_string(),
            ),
            (
                "AD_VERSION".to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ),
            ("AD_BINARY".to_string(), binary_path()?),
        ];
        if let (Some(name), Some(dir)) = (self.sandbox_name, self.sandbox_dir()) {
            env.push(("AD_SANDBOX_NAME".to_string(), name.to_string()));
            env.push(("AD_SANDBOX_DIR".to_string(), dir.display().to_string()));
        }
        if let Ok(cwd) = std::env::current_dir()
            && let Some(cwd) = cwd.to_str()
        {
            env.push(("AD_CALLER_CWD".to_string(), cwd.to_string()));
        }
        env.extend(self.settings.iter().cloned());
        Ok(env)
    }

    fn deadline_error(&self, error: AdapterError, argv: &[String]) -> AdapterError {
        if error.code != ErrorCode::Timeout {
            return error;
        }
        let verb = argv.first().map(String::as_str).unwrap_or("check");
        let provider = self.spec.name;
        let suggestion = next_step(self.sandbox_name);
        let timed_out = AdapterError::new(
            ErrorCode::Timeout,
            format!(
                "The {provider} provider did not answer `{verb}` in time, so agent-desktop stopped waiting."
            ),
        )
        .with_suggestion(suggestion)
        .with_disposition(DeliverySemantics::unknown());
        match error.platform_detail {
            Some(detail) => timed_out.with_platform_detail(detail),
            None => timed_out,
        }
    }
}

/// One provider reply as the command's data value.
pub(crate) fn success_value(name: &str, provider: Option<&str>, reply: ProviderReply) -> Value {
    let ProviderReply {
        state,
        message,
        next,
        data,
    } = reply;
    let mut value = json!({ "name": name, "state": state, "message": message });
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    if let Some(provider) = provider {
        object.insert("provider".to_string(), Value::String(provider.to_string()));
    }
    if let Some(next) = next {
        object.insert("next".to_string(), Value::String(next));
    }
    if let Some(Value::Object(data)) = data {
        merge(object, data);
    }
    value
}

fn merge(object: &mut Map<String, Value>, data: Map<String, Value>) {
    for (key, value) in data {
        object.entry(key).or_insert(value);
    }
}

fn binary_path() -> Result<String, AppError> {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .map_err(|_| {
            AppError::Adapter(
                AdapterError::new(
                    ErrorCode::Internal,
                    "agent-desktop could not find its own program path.",
                )
                .with_suggestion("Run the command from an installed agent-desktop program.")
                .with_disposition(DeliverySemantics::not_delivered()),
            )
        })
}
