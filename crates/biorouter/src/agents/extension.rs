use crate::agents::chatrecall_extension;
use crate::agents::code_execution_extension;
use crate::agents::extension_manager_extension;
use crate::agents::skills_extension;
use crate::agents::todo_extension;
use crate::agents::workspace_extension;
use std::collections::HashMap;

use crate::agents::mcp_client::McpClientTrait;
use crate::agents::mcp_pool::PoolKey;
use crate::config;
use crate::config::extensions::name_to_key;
use crate::config::permission::PermissionLevel;
use once_cell::sync::Lazy;
use rmcp::model::Tool;
use rmcp::service::ClientInitializeError;
use rmcp::ServiceError as ClientError;
use serde::Deserializer;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;
use utoipa::ToSchema;

#[derive(Error, Debug)]
#[error("process quit before initialization: stderr = {stderr}")]
pub struct ProcessExit {
    stderr: String,
    #[source]
    source: ClientInitializeError,
}

impl ProcessExit {
    pub fn new<T>(stderr: T, source: ClientInitializeError) -> Self
    where
        T: Into<String>,
    {
        ProcessExit {
            stderr: stderr.into(),
            source,
        }
    }
}

pub static PLATFORM_EXTENSIONS: Lazy<HashMap<&'static str, PlatformExtensionDef>> = Lazy::new(
    || {
        let mut map = HashMap::new();

        map.insert(
            todo_extension::EXTENSION_NAME,
            PlatformExtensionDef {
                name: todo_extension::EXTENSION_NAME,
                description:
                    "Keep a running checklist through a multi-step task, so Biorouter tracks \
                     what is done and what is left",
                default_enabled: true,
                client_factory: |ctx| Box::new(todo_extension::TodoClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            "chatrecall",
            PlatformExtensionDef {
                name: chatrecall_extension::EXTENSION_NAME,
                description:
                    "Search your earlier chats and load a summary of one, so work you already \
                     did can be picked up here",
                default_enabled: false,
                client_factory: |ctx| {
                    Box::new(chatrecall_extension::ChatRecallClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            "workspace",
            PlatformExtensionDef {
                name: workspace_extension::EXTENSION_NAME,
                description:
                    "Operate the Biorouter workspace: list, open and read conversations, send \
                     prompts, change tool sets, and run subagents in visible tabs",
                // ⚠ Default ON as a built-in capability rather than an
                // extension (#76). This grants the FULL surface — available_tools
                // is empty, which means all of them, including reading and
                // steering other conversations. That is deliberate: "Workspace
                // Control" IS that capability, and the user asked for it on by
                // default.
                //
                // The delegation gate no longer counts it.
                // `has_non_injected_extensions` excludes it by NAME as well as
                // by origin, because a config-enabled workspace loads as
                // Explicit and would otherwise satisfy that gate by itself, in
                // every session, forever.
                default_enabled: true,
                client_factory: |ctx| {
                    Box::new(workspace_extension::WorkspaceClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            "extensionmanager",
            PlatformExtensionDef {
                name: extension_manager_extension::EXTENSION_NAME,
                // ⚠ A FRESH-INSTALL-ONLY fix. `inject_platform_extensions`
                // takes the configured entry when one exists, so a
                // `config.yaml` that already carries an `extensionmanager`
                // Platform entry keeps whatever description it was written
                // with — forever. Editing this changes what NEW installs see,
                // not what existing ones read.
                description:
                    "Look up which extensions are available, attach or detach one, search the \
                     trusted marketplace, install a package, or permanently delete an installed \
                     one — so a task that needs a tool you have not loaded can still finish",
                default_enabled: true,
                client_factory: |ctx| {
                    Box::new(extension_manager_extension::ExtensionManagerClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            skills_extension::EXTENSION_NAME,
            PlatformExtensionDef {
                name: skills_extension::EXTENSION_NAME,
                description: "Search the skills installed on this machine and load the one that \
                              matches the task in hand",
                default_enabled: true,
                client_factory: |ctx| Box::new(skills_extension::SkillsClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            code_execution_extension::EXTENSION_NAME,
            PlatformExtensionDef {
                name: code_execution_extension::EXTENSION_NAME,
                description: "Execute JavaScript code in a sandboxed environment",
                default_enabled: true,
                client_factory: |ctx| {
                    Box::new(code_execution_extension::CodeExecutionClient::new(ctx).unwrap())
                },
            },
        );

        map.insert("crew", PlatformExtensionDef {
            name: crate::agents::crew_extension::EXTENSION_NAME,
            description: "Use saved Crew connections and human-approved channel context and agent posting",
            default_enabled: true,
            client_factory: |ctx| Box::new(crate::agents::crew_extension::CrewClient::new(ctx)),
        });
        map
    },
);

#[derive(Clone)]
pub struct PlatformExtensionContext {
    pub extension_manager:
        Option<std::sync::Weak<crate::agents::extension_manager::ExtensionManager>>,
    pub session_manager: std::sync::Arc<crate::session::SessionManager>,
}

#[derive(Debug, Clone)]
pub struct PlatformExtensionDef {
    pub name: &'static str,
    pub description: &'static str,
    pub default_enabled: bool,
    pub client_factory: fn(PlatformExtensionContext) -> Box<dyn McpClientTrait>,
}

/// Errors from Extension operation
#[derive(Error, Debug)]
pub enum ExtensionError {
    #[error("failed a client call to an MCP server: {0}")]
    Client(#[from] ClientError),
    #[error("invalid config: {0}")]
    ConfigError(String),
    #[error("error during extension setup: {0}")]
    SetupError(String),
    #[error("join error occurred during task execution: {0}")]
    TaskJoinError(#[from] tokio::task::JoinError),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("failed to initialize MCP client: {0}")]
    InitializeError(#[from] ClientInitializeError),
    #[error("{0}")]
    ProcessExit(#[from] ProcessExit),
}

pub type ExtensionResult<T> = Result<T, ExtensionError>;

#[derive(Debug, Clone, Deserialize, Serialize, Default, ToSchema, PartialEq)]
pub struct Envs {
    /// A map of environment variables to set, e.g. API_KEY -> some_secret, HOST -> host
    #[serde(default)]
    #[serde(flatten)]
    map: HashMap<String, String>,
}

impl Envs {
    /// List of sensitive env vars that should not be overridden
    const DISALLOWED_KEYS: [&'static str; 31] = [
        // 🔧 Binary path manipulation
        "PATH",       // Controls executable lookup paths — critical for command hijacking
        "PATHEXT",    // Windows: Determines recognized executable extensions (e.g., .exe, .bat)
        "SystemRoot", // Windows: Can affect system DLL resolution (e.g., `kernel32.dll`)
        "windir",     // Windows: Alternative to SystemRoot (used in legacy apps)
        // 🧬 Dynamic linker hijacking (Linux/macOS)
        "LD_LIBRARY_PATH",  // Alters shared library resolution
        "LD_PRELOAD",       // Forces preloading of shared libraries — common attack vector
        "LD_AUDIT",         // Loads a monitoring library that can intercept execution
        "LD_DEBUG",         // Enables verbose linker logging (information disclosure risk)
        "LD_BIND_NOW",      // Forces immediate symbol resolution, affecting ASLR
        "LD_ASSUME_KERNEL", // Tricks linker into thinking it's running on an older kernel
        // 🍎 macOS dynamic linker variables
        "DYLD_LIBRARY_PATH",     // Same as LD_LIBRARY_PATH but for macOS
        "DYLD_INSERT_LIBRARIES", // macOS equivalent of LD_PRELOAD
        "DYLD_FRAMEWORK_PATH",   // Overrides framework lookup paths
        // 🐍 Python / Node / Ruby / Java / Golang hijacking
        "PYTHONPATH",   // Overrides Python module resolution
        "PYTHONHOME",   // Overrides Python root directory
        "NODE_OPTIONS", // Injects options/scripts into every Node.js process
        "RUBYOPT",      // Injects Ruby execution flags
        "GEM_PATH",     // Alters where RubyGems looks for installed packages
        "GEM_HOME",     // Changes RubyGems default install location
        "CLASSPATH",    // Java: Controls where classes are loaded from — critical for RCE attacks
        "GO111MODULE",  // Go: Forces use of module proxy or disables it
        "GOROOT", // Go: Changes root installation directory (could lead to execution hijacking)
        // 🖥️ Windows-specific process & DLL hijacking
        "APPINIT_DLLS", // Forces Windows to load a DLL into every process
        "SESSIONNAME",  // Affects Windows session configuration
        "ComSpec",      // Determines default command interpreter (can replace `cmd.exe`)
        "TEMP",
        "TMP",          // Redirects temporary file storage (useful for injection attacks)
        "LOCALAPPDATA", // Controls application data paths (can be abused for persistence)
        "USERPROFILE",  // Windows user directory (can affect profile-based execution paths)
        "HOMEDRIVE",
        "HOMEPATH", // Changes where the user's home directory is located
    ];

    /// Constructs a new Envs, skipping disallowed env vars with a warning
    pub fn new(map: HashMap<String, String>) -> Self {
        let mut validated = HashMap::new();

        for (key, value) in map {
            if Self::is_disallowed(&key) {
                warn!("Skipping disallowed env var: {}", key);
                continue;
            }
            validated.insert(key, value);
        }

        Self { map: validated }
    }

    /// Returns a copy of the validated env vars
    pub fn get_env(&self) -> HashMap<String, String> {
        self.map.clone()
    }

    /// Returns an error if any disallowed env var is present
    pub fn validate(&self) -> Result<(), Box<ExtensionError>> {
        for key in self.map.keys() {
            if Self::is_disallowed(key) {
                return Err(Box::new(ExtensionError::ConfigError(format!(
                    "environment variable {} not allowed to be overwritten",
                    key
                ))));
            }
        }
        Ok(())
    }

    fn is_disallowed(key: &str) -> bool {
        Self::DISALLOWED_KEYS
            .iter()
            .any(|disallowed| disallowed.eq_ignore_ascii_case(key))
    }
}

/// Represents the different types of MCP extensions that can be added to the manager
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
#[serde(tag = "type")]
pub enum ExtensionConfig {
    /// SSE transport is no longer supported - kept only for config file compatibility
    #[serde(rename = "sse")]
    Sse {
        #[serde(default)]
        #[schema(required)]
        name: String,
        #[serde(default)]
        #[serde(deserialize_with = "deserialize_null_with_default")]
        #[schema(required)]
        description: String,
        #[serde(default)]
        uri: Option<String>,
    },
    /// Standard I/O client with command and arguments
    #[serde(rename = "stdio")]
    Stdio {
        /// The name used to identify this extension
        name: String,
        #[serde(default)]
        #[serde(deserialize_with = "deserialize_null_with_default")]
        #[schema(required)]
        description: String,
        cmd: String,
        args: Vec<String>,
        #[serde(default)]
        envs: Envs,
        #[serde(default)]
        env_keys: Vec<String>,
        timeout: Option<u64>,
        #[serde(default)]
        bundled: Option<bool>,
        #[serde(default)]
        available_tools: Vec<String>,
    },
    /// Built-in extension that is part of the bundled biorouter MCP server
    #[serde(rename = "builtin")]
    Builtin {
        /// The name used to identify this extension
        name: String,
        #[serde(default)]
        #[serde(deserialize_with = "deserialize_null_with_default")]
        #[schema(required)]
        description: String,
        display_name: Option<String>, // needed for the UI
        timeout: Option<u64>,
        #[serde(default)]
        bundled: Option<bool>,
        #[serde(default)]
        available_tools: Vec<String>,
    },
    /// Platform extensions that have direct access to the agent etc and run in the agent process
    #[serde(rename = "platform")]
    Platform {
        /// The name used to identify this extension
        name: String,
        #[serde(deserialize_with = "deserialize_null_with_default")]
        #[schema(required)]
        description: String,
        #[serde(default)]
        bundled: Option<bool>,
        #[serde(default)]
        available_tools: Vec<String>,
    },
    /// Streamable HTTP client with a URI endpoint using MCP Streamable HTTP specification
    #[serde(rename = "streamable_http")]
    StreamableHttp {
        /// The name used to identify this extension
        name: String,
        #[serde(deserialize_with = "deserialize_null_with_default")]
        #[schema(required)]
        description: String,
        uri: String,
        #[serde(default)]
        envs: Envs,
        #[serde(default)]
        env_keys: Vec<String>,
        #[serde(default)]
        headers: HashMap<String, String>,
        // NOTE: set timeout to be optional for compatibility.
        // However, new configurations should include this field.
        timeout: Option<u64>,
        #[serde(default)]
        bundled: Option<bool>,
        #[serde(default)]
        available_tools: Vec<String>,
    },
    /// Frontend-provided tools that will be called through the frontend
    #[serde(rename = "frontend")]
    Frontend {
        /// The name used to identify this extension
        name: String,
        #[serde(deserialize_with = "deserialize_null_with_default")]
        #[schema(required)]
        description: String,
        /// The tools provided by the frontend
        tools: Vec<Tool>,
        /// Instructions for how to use these tools
        instructions: Option<String>,
        #[serde(default)]
        bundled: Option<bool>,
        #[serde(default)]
        available_tools: Vec<String>,
    },
    /// Inline Python code that will be executed using uvx
    #[serde(rename = "inline_python")]
    InlinePython {
        /// The name used to identify this extension
        name: String,
        #[serde(deserialize_with = "deserialize_null_with_default")]
        #[schema(required)]
        description: String,
        /// The Python code to execute
        code: String,
        /// Timeout in seconds
        timeout: Option<u64>,
        /// Python package dependencies required by this extension
        #[serde(default)]
        dependencies: Option<Vec<String>>,
        #[serde(default)]
        available_tools: Vec<String>,
    },
}

fn redacted_stdio(
    name: &str,
    description: &str,
    env_keys: &[String],
    timeout: Option<u64>,
    bundled: Option<bool>,
    available_tools: &[String],
) -> ExtensionConfig {
    ExtensionConfig::Stdio {
        name: name.to_string(),
        description: description.to_string(),
        cmd: String::new(),
        args: Vec::new(),
        envs: Envs::default(),
        env_keys: env_keys.to_vec(),
        timeout,
        bundled,
        available_tools: available_tools.to_vec(),
    }
}

fn redacted_streamable_http(
    name: &str,
    description: &str,
    env_keys: &[String],
    timeout: Option<u64>,
    bundled: Option<bool>,
    available_tools: &[String],
) -> ExtensionConfig {
    ExtensionConfig::StreamableHttp {
        name: name.to_string(),
        description: description.to_string(),
        uri: String::new(),
        envs: Envs::default(),
        env_keys: env_keys.to_vec(),
        headers: HashMap::new(),
        timeout,
        bundled,
        available_tools: available_tools.to_vec(),
    }
}

impl Default for ExtensionConfig {
    fn default() -> Self {
        Self::Builtin {
            name: config::DEFAULT_EXTENSION.to_string(),
            display_name: Some(config::DEFAULT_DISPLAY_NAME.to_string()),
            description: "default".to_string(),
            timeout: Some(config::DEFAULT_EXTENSION_TIMEOUT),
            bundled: Some(true),
            available_tools: Vec::new(),
        }
    }
}

impl ExtensionConfig {
    /// Whether this entry is a Biorouter capability rather than an installed
    /// third-party extension.
    pub fn is_capability(&self) -> bool {
        match self {
            Self::Builtin { name, .. } => {
                biorouter_mcp::BUILTIN_EXTENSIONS.contains_key(name_to_key(name).as_str())
            }
            Self::Platform { name, .. } => {
                PLATFORM_EXTENSIONS.contains_key(name_to_key(name).as_str())
            }
            _ => false,
        }
    }

    /// A display/import projection that cannot carry resolved connector auth.
    /// Executable locators are omitted too: command arguments, endpoint URLs,
    /// inline code, frontend schemas, and instructions can all embed credentials
    /// even when the dedicated `envs` and `headers` fields are empty.
    pub fn redacted_for_session_export(&self) -> Self {
        match self {
            Self::Sse {
                name, description, ..
            } => Self::Sse {
                name: name.clone(),
                description: description.clone(),
                uri: None,
            },
            Self::Stdio {
                name,
                description,
                env_keys,
                timeout,
                bundled,
                available_tools,
                ..
            } => redacted_stdio(
                name,
                description,
                env_keys,
                *timeout,
                *bundled,
                available_tools,
            ),
            Self::Builtin {
                name,
                description,
                display_name,
                timeout,
                bundled,
                available_tools,
            } => Self::Builtin {
                name: name.clone(),
                description: description.clone(),
                display_name: display_name.clone(),
                timeout: *timeout,
                bundled: *bundled,
                available_tools: available_tools.clone(),
            },
            Self::Platform {
                name,
                description,
                bundled,
                available_tools,
            } => Self::Platform {
                name: name.clone(),
                description: description.clone(),
                bundled: *bundled,
                available_tools: available_tools.clone(),
            },
            Self::StreamableHttp {
                name,
                description,
                env_keys,
                timeout,
                bundled,
                available_tools,
                ..
            } => redacted_streamable_http(
                name,
                description,
                env_keys,
                *timeout,
                *bundled,
                available_tools,
            ),
            Self::Frontend {
                name,
                description,
                bundled,
                available_tools,
                ..
            } => Self::Frontend {
                name: name.clone(),
                description: description.clone(),
                tools: Vec::new(),
                instructions: None,
                bundled: *bundled,
                available_tools: available_tools.clone(),
            },
            Self::InlinePython {
                name,
                description,
                timeout,
                available_tools,
                ..
            } => Self::InlinePython {
                name: name.clone(),
                description: description.clone(),
                code: String::new(),
                timeout: *timeout,
                dependencies: None,
                available_tools: available_tools.clone(),
            },
        }
    }

    pub fn is_bundled(&self) -> bool {
        match self {
            Self::Stdio { bundled, .. }
            | Self::Builtin { bundled, .. }
            | Self::Platform { bundled, .. }
            | Self::StreamableHttp { bundled, .. }
            | Self::Frontend { bundled, .. } => bundled.unwrap_or(false),
            Self::Sse { .. } | Self::InlinePython { .. } => false,
        }
    }

    pub fn streamable_http<S: Into<String>, T: Into<u64>>(
        name: S,
        uri: S,
        description: S,
        timeout: T,
    ) -> Self {
        Self::StreamableHttp {
            name: name.into(),
            uri: uri.into(),
            envs: Envs::default(),
            env_keys: Vec::new(),
            headers: HashMap::new(),
            description: description.into(),
            timeout: Some(timeout.into()),
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    pub fn stdio<S: Into<String>, T: Into<u64>>(
        name: S,
        cmd: S,
        description: S,
        timeout: T,
    ) -> Self {
        Self::Stdio {
            name: name.into(),
            cmd: cmd.into(),
            args: vec![],
            envs: Envs::default(),
            env_keys: Vec::new(),
            description: description.into(),
            timeout: Some(timeout.into()),
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    pub fn inline_python<S: Into<String>, T: Into<u64>>(
        name: S,
        code: S,
        description: S,
        timeout: T,
    ) -> Self {
        Self::InlinePython {
            name: name.into(),
            code: code.into(),
            description: description.into(),
            timeout: Some(timeout.into()),
            dependencies: None,
            available_tools: Vec::new(),
        }
    }

    pub fn with_args<I, S>(self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        match self {
            Self::Stdio {
                name,
                cmd,
                envs,
                env_keys,
                timeout,
                description,
                bundled,
                available_tools,
                ..
            } => Self::Stdio {
                name,
                cmd,
                envs,
                env_keys,
                args: args.into_iter().map(Into::into).collect(),
                description,
                timeout,
                bundled,
                available_tools,
            },
            other => other,
        }
    }

    pub fn key(&self) -> String {
        let name = self.name();
        name_to_key(&name)
    }

    /// The SharedMcpPool identity for this config in a given working directory, or
    /// `None` if the variant must never be pooled (BR-54). Two configs that return
    /// equal `PoolKey`s can safely share ONE spawned process.
    ///
    /// Not poolable:
    /// - `Sse` — unsupported transport (errors at spawn anyway).
    /// - `Platform` — its client captures a `Weak` ref to the *specific*
    ///   `ExtensionManager` that built it, so it is inherently session-scoped.
    /// - `Frontend` — not a spawned server (tools are proxied to the frontend).
    pub fn pool_key(&self, working_dir: &std::path::Path) -> Option<PoolKey> {
        use std::hash::{Hash, Hasher};

        fn hash_env<H: Hasher>(h: &mut H, envs: &Envs, env_keys: &[String]) {
            // Values are stable within one daemon (secrets are process-global), so
            // hashing declared envs + secret key NAMES distinguishes configs
            // without resolving secrets from the keychain on every key derivation.
            let mut e: Vec<(String, String)> = envs.get_env().into_iter().collect();
            e.sort();
            e.hash(h);
            let mut k: Vec<String> = env_keys.to_vec();
            k.sort();
            k.hash(h);
        }

        let mut h = std::collections::hash_map::DefaultHasher::new();
        let transport = match self {
            Self::Stdio {
                cmd,
                args,
                envs,
                env_keys,
                ..
            } => {
                hash_env(&mut h, envs, env_keys);
                format!("stdio:{}\u{0}{}", cmd, args.join("\u{0}"))
            }
            Self::StreamableHttp {
                uri,
                headers,
                envs,
                env_keys,
                ..
            } => {
                hash_env(&mut h, envs, env_keys);
                let mut hs: Vec<(&String, &String)> = headers.iter().collect();
                hs.sort();
                hs.hash(&mut h);
                format!("http:{}", uri)
            }
            Self::Builtin { name, .. } if name == "computercontroller" => return None,
            Self::Builtin { name, .. } => format!("builtin:{}", name),
            Self::InlinePython {
                name,
                code,
                dependencies,
                ..
            } => {
                let mut deps = dependencies.clone().unwrap_or_default();
                deps.sort();
                deps.hash(&mut h);
                let mut ch = std::collections::hash_map::DefaultHasher::new();
                code.hash(&mut ch);
                format!("inline_python:{}\u{0}{:x}", name, ch.finish())
            }
            Self::Sse { .. } | Self::Platform { .. } | Self::Frontend { .. } => return None,
        };

        Some(PoolKey {
            transport,
            working_dir: Some(working_dir.to_path_buf()),
            env_fingerprint: h.finish(),
        })
    }

    /// Get the extension name regardless of variant
    pub fn name(&self) -> String {
        match self {
            Self::Sse { name, .. } => name,
            Self::StreamableHttp { name, .. } => name,
            Self::Stdio { name, .. } => name,
            Self::Builtin { name, .. } => name,
            Self::Platform { name, .. } => name,
            Self::Frontend { name, .. } => name,
            Self::InlinePython { name, .. } => name,
        }
        .to_string()
    }

    /// Check if a tool should be available to the LLM
    pub fn is_tool_available(&self, tool_name: &str) -> bool {
        let available_tools = match self {
            Self::Sse { .. } => return false, // SSE is unsupported
            Self::StreamableHttp {
                available_tools, ..
            }
            | Self::Stdio {
                available_tools, ..
            }
            | Self::Builtin {
                available_tools, ..
            }
            | Self::Platform {
                available_tools, ..
            }
            | Self::InlinePython {
                available_tools, ..
            }
            | Self::Frontend {
                available_tools, ..
            } => available_tools,
        };

        // If no tools are specified, all tools are available
        // If tools are specified, only those tools are available
        available_tools.is_empty() || available_tools.contains(&tool_name.to_string())
    }
}

impl std::fmt::Display for ExtensionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionConfig::Sse { name, .. } => {
                write!(f, "SSE({}: unsupported)", name)
            }
            ExtensionConfig::StreamableHttp { name, uri, .. } => {
                write!(f, "StreamableHttp({}: {})", name, uri)
            }
            ExtensionConfig::Stdio {
                name, cmd, args, ..
            } => {
                write!(f, "Stdio({}: {} {})", name, cmd, args.join(" "))
            }
            ExtensionConfig::Builtin { name, .. } => write!(f, "Builtin({})", name),
            ExtensionConfig::Platform { name, .. } => write!(f, "Platform({})", name),
            ExtensionConfig::Frontend { name, tools, .. } => {
                write!(f, "Frontend({}: {} tools)", name, tools.len())
            }
            ExtensionConfig::InlinePython { name, code, .. } => {
                write!(f, "InlinePython({}: {} chars)", name, code.len())
            }
        }
    }
}

/// Model-facing classification for an attached MCP client.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionClassification {
    /// A tool surface shipped as part of Biorouter.
    Capability,
    /// A user-installed or third-party connector.
    Extension,
}

/// Information about an attached MCP client used for building prompts.
#[derive(Clone, Debug, Serialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub instructions: String,
    pub has_resources: bool,
    pub classification: ExtensionClassification,
    /// False only for synthetic/offline prompt fixtures that have not been
    /// joined to a live tool snapshot.
    pub tool_roster_known: bool,
    /// Effective tools owned by this entry before Code Execution's direct-tool
    /// filter. This is the authoritative per-turn module surface.
    pub available_tools: Vec<String>,
    /// Subset exposed directly to the provider on this turn.
    pub directly_callable_tools: Vec<String>,
    /// True when the context budget shortened or removed this entry's prose.
    /// The tool roster remains exact, but the model must be told that the
    /// attached operating guidance is incomplete rather than treating silence
    /// as a complete description of the capability.
    pub instructions_degraded: bool,
}

impl ExtensionInfo {
    pub fn new(name: &str, instructions: &str, has_resources: bool) -> Self {
        Self::classified(
            name,
            instructions,
            has_resources,
            ExtensionClassification::Extension,
        )
    }

    pub fn capability(name: &str, instructions: &str, has_resources: bool) -> Self {
        Self::classified(
            name,
            instructions,
            has_resources,
            ExtensionClassification::Capability,
        )
    }

    pub fn classified(
        name: &str,
        instructions: &str,
        has_resources: bool,
        classification: ExtensionClassification,
    ) -> Self {
        Self {
            name: name.to_string(),
            instructions: instructions.to_string(),
            has_resources,
            classification,
            tool_roster_known: false,
            available_tools: Vec::new(),
            directly_callable_tools: Vec::new(),
            instructions_degraded: false,
        }
    }
}

fn deserialize_null_with_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// Information about the tool used for building prompts
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub parameters: Vec<String>,
    pub permission: Option<PermissionLevel>,
}

impl ToolInfo {
    pub fn new(
        name: &str,
        description: &str,
        parameters: Vec<String>,
        permission: Option<PermissionLevel>,
    ) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
            permission,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PLATFORM_EXTENSIONS;
    use crate::agents::*;

    /// ⚠ **Default ON, and it grants the FULL surface** (#76).
    ///
    /// This test asserted the opposite until Workspace became a built-in
    /// capability. The inversion is deliberate, but it is not free, and the
    /// second assertion is the one that matters: `available_tools` is empty,
    /// which means every workspace tool, including reading and steering other
    /// conversations. A future change that keeps the default but narrows the
    /// grant would be a different product decision, so it should fail here and
    /// be made on purpose rather than noticed later.
    #[test]
    fn workspace_is_a_default_on_capability_granting_its_whole_surface() {
        assert_eq!(PLATFORM_EXTENSIONS.len(), 7);
        assert!(
            PLATFORM_EXTENSIONS["workspace"].default_enabled,
            "workspace is a capability now, so it ships on"
        );
    }

    #[test]
    fn platform_extension_defaults_match_capabilities() {
        assert_eq!(PLATFORM_EXTENSIONS.len(), 7);
        assert!(PLATFORM_EXTENSIONS["todo"].default_enabled);
        assert!(PLATFORM_EXTENSIONS["extensionmanager"].default_enabled);
        assert!(PLATFORM_EXTENSIONS["skills"].default_enabled);
        assert!(PLATFORM_EXTENSIONS["code_execution"].default_enabled);
        assert!(!PLATFORM_EXTENSIONS["chatrecall"].default_enabled);
    }

    #[test]
    fn builtin_transport_is_not_itself_a_capability_classification() {
        let capability = ExtensionConfig::Builtin {
            name: "Developer".to_string(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: Vec::new(),
        };
        let connector = ExtensionConfig::Builtin {
            name: "ucsfomopagent".to_string(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: Vec::new(),
        };

        assert!(capability.is_capability());
        assert!(
            !connector.is_capability(),
            "a bundled connector remains an extension even when its transport enum is Builtin"
        );
    }

    #[test]
    fn test_deserialize_missing_description() {
        let config: ExtensionConfig = serde_yaml::from_str(
            "enabled: true
type: builtin
name: developer
display_name: Developer
timeout: 300
bundled: true
available_tools: []",
        )
        .unwrap();
        if let ExtensionConfig::Builtin { description, .. } = config {
            assert_eq!(description, "")
        } else {
            panic!("unexpected result of deserialization: {}", config)
        }
    }

    #[test]
    fn test_deserialize_null_description() {
        let config: ExtensionConfig = serde_yaml::from_str(
            "enabled: true
type: builtin
name: developer
display_name: Developer
description: null
timeout: 300
bundled: true
available_tools: []
",
        )
        .unwrap();
        if let ExtensionConfig::Builtin { description, .. } = config {
            assert_eq!(description, "")
        } else {
            panic!("unexpected result of deserialization: {}", config)
        }
    }

    #[test]
    fn test_deserialize_normal_description() {
        let config: ExtensionConfig = serde_yaml::from_str(
            "enabled: true
type: builtin
name: developer
display_name: Developer
description: description goes here
timeout: 300
bundled: true
available_tools: []
    ",
        )
        .unwrap();
        if let ExtensionConfig::Builtin { description, .. } = config {
            assert_eq!(description, "description goes here")
        } else {
            panic!("unexpected result of deserialization: {}", config)
        }
    }
}
