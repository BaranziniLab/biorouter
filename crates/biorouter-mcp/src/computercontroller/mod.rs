use etcetera::{choose_app_strategy, AppStrategy};
use indoc::{formatdoc, indoc};
use reqwest::{Client, Url};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        AnnotateAble, CallToolResult, Content, ErrorCode, ErrorData, Implementation,
        ListResourcesResult, PaginatedRequestParams, RawResource, ReadResourceRequestParams,
        ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo,
    },
    schemars::JsonSchema,
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc, sync::Mutex};
use tokio::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

mod docx_tool;
mod pdf_tool;
mod xlsx_tool;

mod platform;
use platform::{create_system_automation, SystemAutomation};

const MAX_INLINE_WEB_CONTENT_BYTES: usize = 128 * 1024;

/// Build the child process that runs an `automation_script` body.
///
/// `automation_script` executes a script the model wrote, so the child is as
/// agent-controlled as the `developer` shell tool and must not inherit the
/// daemon's own credentials — see
/// [`crate::developer::shell::strip_daemon_private_env`] and issue #57.
/// `computer_controller` is a shipped built-in and does **not** go through
/// `configure_shell_command`, so without the call below the daemon's auth
/// secret reached the script and the fix for #57 was reopened in full: a script
/// could read `BIOROUTER_SERVER__SECRET_KEY` out of its environment and then
/// call `biorouterd` as a fully authenticated client (`GET /sessions` for every
/// session, `POST /sessions/import`).
///
/// The strip is applied **last**, after every other `env` call, so nothing set
/// here can re-admit a daemon credential.
fn automation_script_command(
    language: &ScriptLanguage,
    shell: &str,
    shell_arg: &str,
    command: &str,
) -> Command {
    let mut cmd = match language {
        // For PowerShell, we need to use -File instead of -Command
        ScriptLanguage::Powershell => {
            let mut cmd = Command::new("powershell");
            cmd.arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-File")
                .arg(command);
            cmd
        }
        _ => {
            let mut cmd = Command::new(shell);
            cmd.arg(shell_arg).arg(command);
            cmd
        }
    };
    cmd.env("BIOROUTER_TERMINAL", "1");
    crate::developer::shell::strip_daemon_private_env(&mut cmd);
    cmd
}

/// Refuse a script body that reaches a file the secret guard denies (H1).
///
/// The dispatch boundary already scans the `script` argument, but this server
/// is also spawned on its own (`biorouter mcp computercontroller`), where this
/// is the only check — and a script runs from *this* process's working
/// directory, which is where its relative paths resolve. A shell body is read
/// the way the shell will read it; anything else (Ruby, PowerShell, Batch,
/// AppleScript) has its path literals and its shell-outs (`do shell script
/// "…"`, `system("…")`) judged.
fn refuse_secret_access(script: &str, is_shell: bool) -> Result<(), ErrorData> {
    let cwd = std::env::current_dir().unwrap_or_default();
    refuse_secret_access_in(
        script,
        is_shell,
        &cwd,
        &crate::secret_guard::ShellEnv::from_process(),
    )
}

fn refuse_secret_access_in(
    script: &str,
    is_shell: bool,
    cwd: &std::path::Path,
    env: &crate::secret_guard::ShellEnv,
) -> Result<(), ErrorData> {
    let guard = crate::secret_guard::SecretGuard::cached_for_dir(cwd);
    let denied = if is_shell {
        guard.find_denied_in_command(script, cwd, env)
    } else {
        guard.find_denied_in_code(script, cwd, env)
    };
    match denied {
        Some(denied) => Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            denied.message(),
            None,
        )),
        None => Ok(()),
    }
}

/// `web_scrape` HTTP hardening (issue #25): a browser-compatible UA (the old
/// bare `biorouter/1.0` was bot-flagged into 403s), a request timeout (a hung
/// server used to hang the tool indefinitely), and one retry on transient
/// failures only — connect errors, 429, and 5xx. A timed-out request is NOT
/// retried: the client already waited the full timeout, so retrying a hung
/// server would double worst-case latency.
const WEB_SCRAPE_USER_AGENT: &str = "Mozilla/5.0 (compatible; biorouter/1.0)";
const WEB_SCRAPE_TIMEOUT_SECS: u64 = 30;
const WEB_SCRAPE_RETRY_BACKOFF_MS: u64 = 500;

/// Whether a non-success status is worth one retry: 429 and 5xx are transient;
/// 4xx client errors (403/404/…) are deterministic — an identical retry cannot
/// succeed.
fn web_scrape_status_is_retryable(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

/// Per-status recovery hint appended to the error text. The status code itself
/// stays in the message — the tool-error classifier keys on it ('403' →
/// permission_denied, '404' → not_found, '429' → transient), so it is
/// load-bearing, and the hint tells the model what to do instead of retrying.
fn web_scrape_status_hint(status: reqwest::StatusCode) -> &'static str {
    match status.as_u16() {
        403 => {
            " The site blocks automated clients. Try an alternative source, \
                 or browser automation if available."
        }
        404 => " The URL does not exist. Verify the URL or pick another source.",
        429 => {
            " The site is rate-limiting requests. Wait before retrying, or use \
                 another source."
        }
        code if (500..600).contains(&code) => {
            " The server failed. Retry later or use another source."
        }
        _ => "",
    }
}

fn bounded_web_content(content: &str) -> (&str, bool) {
    if content.len() <= MAX_INLINE_WEB_CONTENT_BYTES {
        return (content, false);
    }

    let mut end = MAX_INLINE_WEB_CONTENT_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    (
        content
            .get(..end)
            .expect("bounded web content must end on a character boundary"),
        true,
    )
}

/// Enum for save_as parameter in web_scrape tool
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Default)]
#[serde(rename_all = "lowercase")]
pub enum SaveAsFormat {
    /// Save as text (for HTML pages)
    #[default]
    Text,
    /// Save as JSON (for API responses)
    Json,
    /// Save as binary (for images and other files)
    Binary,
}

/// Parameters for the web_scrape tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WebScrapeParams {
    /// The URL to fetch content from
    pub url: String,
    /// Format of the response.
    #[serde(default)]
    pub save_as: SaveAsFormat,
}

/// Enum for language parameter in automation_script tool
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ScriptLanguage {
    /// Shell/Bash script
    Shell,
    /// Batch script (Windows)
    Batch,
    /// Ruby script
    Ruby,
    /// PowerShell script
    Powershell,
}

/// Enum for command parameter in cache tool
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "lowercase")]
pub enum CacheCommand {
    /// List all cached files
    List,
    /// View content of a cached file
    View,
    /// Delete a cached file
    Delete,
    /// Clear all cached files
    Clear,
}

/// Parameters for the automation_script tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AutomationScriptParams {
    /// The scripting language to use
    #[serde(rename = "language")]
    pub language: ScriptLanguage,
    /// The script content
    pub script: String,
    /// Whether to save the script output to a file
    #[serde(default)]
    pub save_output: bool,
}

/// Parameters for the computer_control tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ComputerControlParams {
    /// The automation script content (PowerShell for Windows, AppleScript for macOS)
    pub script: String,
    /// Whether to save the script output to a file
    #[serde(default)]
    pub save_output: bool,
}

/// Parameters for the cache tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CacheParams {
    /// The command to perform
    pub command: CacheCommand,
    /// Path to the cached file for view/delete commands
    pub path: Option<String>,
}

/// Parameters for the pdf_tool
/// Enum for operation parameter in pdf_tool
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum PdfOperation {
    /// Extract all text content from the PDF
    ExtractText,
    /// Extract and save embedded images to PNG files
    ExtractImages,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PdfToolParams {
    /// Path to the PDF file
    pub path: String,
    /// Operation to perform on the PDF
    pub operation: PdfOperation,
}

/// Enum for operation parameter in docx_tool
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum DocxOperation {
    /// Extract all text content and structure from the DOCX
    ExtractText,
    /// Create a new DOCX or update existing one with provided content
    UpdateDoc,
}

/// Enum for update mode in docx_tool params
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocxUpdateMode {
    /// Add content to end of document (default)
    #[default]
    Append,
    /// Replace specific text with new content
    Replace,
    /// Add content with specific heading level and styling
    Structured,
    /// Add an image to the document (with optional caption)
    AddImage,
}

/// Enum for text alignment in docx_tool params
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "lowercase")]
pub enum TextAlignment {
    /// Left alignment
    Left,
    /// Center alignment
    Center,
    /// Right alignment
    Right,
    /// Justified alignment
    Justified,
}

/// Styling options for text in docx_tool
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Default)]
pub struct DocxTextStyle {
    /// Make text bold
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// Make text italic
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Make text underlined
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    /// Font size in points
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    /// Text color in hex format (e.g., 'FF0000' for red)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Text alignment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<TextAlignment>,
}

/// Additional parameters for update_doc operation
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Default)]
pub struct DocxUpdateParams {
    /// Update mode (default: append)
    #[serde(default)]
    pub mode: DocxUpdateMode,
    /// Text to replace (required for replace mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    /// Heading level for structured mode (e.g., 'Heading1', 'Heading2')
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Path to the image file (required for add_image mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
    /// Image width in pixels (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Image height in pixels (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Styling options for the text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<DocxTextStyle>,
}

/// Parameters for the docx_tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DocxToolParams {
    /// Path to the DOCX file
    pub path: String,
    /// Operation to perform on the DOCX
    pub operation: DocxOperation,
    /// Content to write (required for update_doc operation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Additional parameters for update_doc operation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<DocxUpdateParams>,
}

/// Parameters for the xlsx_tool
/// Enum for operation parameter in xlsx_tool
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case")]
pub enum XlsxOperation {
    /// List all worksheets in the workbook
    ListWorksheets,
    /// Get column names from a worksheet
    GetColumns,
    /// Get values and formulas from a cell range
    GetRange,
    /// Search for text in a worksheet
    FindText,
    /// Update a single cell's value
    UpdateCell,
    /// Get value and formula from a specific cell
    GetCell,
    /// Save changes back to the file
    Save,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct XlsxToolParams {
    /// Path to the XLSX file
    pub path: String,
    /// Operation to perform on the XLSX file
    pub operation: XlsxOperation,
    /// Worksheet name (if not provided, uses first worksheet)
    pub worksheet: Option<String>,
    /// Cell range in A1 notation (e.g., 'A1:C10') for get_range operation
    pub range: Option<String>,
    /// Text to search for in find_text operation
    pub search_text: Option<String>,
    /// Whether search should be case-sensitive
    #[serde(default)]
    pub case_sensitive: bool,
    /// Row number for update_cell and get_cell operations
    pub row: Option<u64>,
    /// Column number for update_cell and get_cell operations
    pub col: Option<u64>,
    /// New value for update_cell operation
    pub value: Option<String>,
}

/// ComputerController MCP Server using official RMCP SDK
#[derive(Clone)]
pub struct ComputerControllerServer {
    tool_router: ToolRouter<Self>,
    cache_dir: PathBuf,
    active_resources: Arc<Mutex<HashMap<String, ResourceContents>>>,
    http_client: Arc<Mutex<Option<Client>>>,
    web_scrape_timeout: std::time::Duration,
    instructions: String,
    system_automation: Arc<Box<dyn SystemAutomation + Send + Sync>>,
}

static WEB_SCRAPE_CLIENT_BUILD_LOCK: Mutex<()> = Mutex::new(());

impl Default for ComputerControllerServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl ComputerControllerServer {
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Self {
        // choose_app_strategy().cache_dir()
        // - macOS/Linux: ~/.cache/biorouter/computer_controller/
        // - Windows:     ~\AppData\Local\BaranziniLab\Biorouter\cache\computer_controller\
        // keep previous behavior of defaulting to /tmp/
        let cache_dir = choose_app_strategy(crate::APP_STRATEGY.clone())
            .map(|strategy| strategy.in_cache_dir("computer_controller"))
            .unwrap_or_else(|_| create_system_automation().get_temp_path());

        fs::create_dir_all(&cache_dir).unwrap_or_else(|_| {
            println!(
                "Warning: Failed to create cache directory at {:?}",
                cache_dir
            )
        });

        let system_automation: Arc<Box<dyn SystemAutomation + Send + Sync>> =
            Arc::new(create_system_automation());

        let os_specific_instructions = match std::env::consts::OS {
            "windows" => indoc! {r#"
            Here are some extra tools:
            automation_script
              - Create and run PowerShell or Batch scripts
              - PowerShell is recommended for most tasks
              - Scripts can save their output to files
              - Windows-specific features:
                - PowerShell for system automation and UI control
                - Windows Management Instrumentation (WMI)
                - Registry access and system settings
              - If the Developer capability's `screen_capture` is in the effective roster, use it when visual confirmation is needed

            computer_control
              - System automation using PowerShell
              - If the Developer capability's `screen_capture` is listed, use it to inspect the current state before choosing the next control action.
            "#},
            "macos" => indoc! {r#"
            Here are some extra tools:
            automation_script
              - Create and run Shell and Ruby scripts
              - Shell (bash) is recommended for most tasks
              - Scripts can save their output to files
              - macOS-specific features:
                - AppleScript for system and UI control
                - Integration with macOS apps and services
              - If the Developer capability's `screen_capture` is in the effective roster, use it when visual confirmation is needed

            computer_control
              - System automation using AppleScript
              - If the Developer capability's `screen_capture` is listed, use it to inspect the current state before choosing the next control action.

            When you need to interact with websites or web applications, consider using the computer_control tool with AppleScript, which can automate Safari or other browsers to:
              - Open specific URLs
              - Fill in forms
              - Click buttons
              - Extract content
              - Handle web-based workflows
            This is often more reliable than web scraping for modern web applications.
            "#},
            _ => indoc! {r#"
            Here are some extra tools:
            automation_script
              - Create and run Shell scripts
              - Shell (bash) is recommended for most tasks
              - Scripts can save their output to files
              - Linux-specific features:
                - System automation through shell scripting
                - X11/Wayland window management
                - D-Bus system services integration
                - Desktop environment control
              - If the Developer capability's `screen_capture` is in the effective roster, use it when visual confirmation is needed

            computer_control
              - System automation using shell commands and system tools
              - Desktop environment automation (GNOME, KDE, etc.)
              - If the Developer capability's `screen_capture` is listed, use it to inspect the current state before choosing the next control action.

            When you need to interact with websites or web applications, consider using tools like xdotool or wmctrl for:
              - Window management
              - Simulating keyboard/mouse input
              - Automating UI interactions
              - Desktop environment control
            "#},
        };

        let instructions = formatdoc! {r#"
            You are a helpful assistant to a power user who is not a professional developer, but you may use development tools to help assist them.
            The user may not know how to break down tasks, so you will need to ensure that you do, and run things in batches as needed.
            The Computer Control capability helps with web retrieval, data processing,
            and operating-system automation.

            You can use scripting as needed to work with text files of data, such as csvs, json, or text files etc.
            If the Developer capability is enabled and its effective roster includes the needed tool, it may be used for more sophisticated scripting tasks. Do not assume it is loaded.

            Accessing web sites, even apis, may be common (you can use scripting to do this) without troubling them too much (they won't know what limits are).
            Try to do your best to find ways to complete a task without too many questions or offering options unless it is really unclear, find a way if you can.
            You can also guide them steps if they can help out as you go along.

            `screen_capture` belongs to the **Developer** capability, not this one, so it is available only when Developer is enabled and lists it. Treat the effective roster in the system prompt as authoritative; never claim or call it when it is absent.

            ## How to operate the computer (read this before using computer_control)

            These principles apply on every operating system. Follow them to avoid
            wasting time and tokens going back and forth without making progress:

            1. Work PROGRESSIVELY in small, verifiable steps. Do ONE action
               (activate an app, click a control, type text), then confirm its
               effect (take a screenshot or query the UI/app state) BEFORE the
               next action. Do not chain many blind UI actions at once.
            2. NEVER repeat an action that did not visibly change anything. If the
               same step fails or has no effect twice, STOP repeating it. Re-read
               the latest screenshot/output, change your approach, or report what
               you observe and ask the user; looping wastes their tokens.
            3. PERMISSIONS are the most common real failure, not your script. If a
               control script reports a permission/accessibility/automation error
               (e.g. "not allowed", "assistive access", "not authorized"), the OS
               is blocking automation. Tell the user exactly which permission to
               grant (e.g. Accessibility / Screen Recording / Automation for the
               app) and stop. Do NOT retry the same script until they confirm.
            4. Prefer the most RELIABLE method available, in this order: (a) the
               application's own automation/scripting interface or a CLI, (b)
               keyboard navigation and shortcuts, (c) clicking a named UI element,
               and only as a last resort (d) clicking raw screen coordinates.
               Coordinate clicks are brittle and are a frequent cause of getting
               stuck. Before clicking, identify the target element by name/role;
               if you cannot find it, list the available elements/windows rather
               than guessing where it is.
            5. WHEN the Developer capability's `screen_capture` IS LISTED, remember that screenshots are
               per-display. Its result lists connected displays and the primary;
               target the correct display or a window-title substring rather than
               assuming the window is on display 0.
            6. RECOVER FROM POPUPS AND HANGS. If a control action times out or
               hangs, or a search box / dialog / menu / autocomplete popup is open
               and not behaving as you expect, the UI is blocked. Press Escape to
               dismiss the popup (Escape again if needed), take a screenshot to see
               the real state, and only then choose your next action. Never keep
               typing into a popup that isn't responding, and never re-send a script
               that just timed out. Fix the situation first.

            ## Driving messaging & chat apps (Slack, Teams, Discord, Mail, etc.)

            These apps are generally NOT scriptable through their automation
            interface, so you must drive their UI, and you must follow their actual
            UX rather than guessing:
            - To SEND a message: the composer is the text box at the BOTTOM of the
              open conversation. Click into it (it is usually already focused), type
              the message, then press Return/Enter to send. Afterwards take a
              screenshot and confirm the message actually appears in the conversation.
            - To SWITCH channel/DM in Slack: open the quick switcher with Cmd/Ctrl+K,
              type the channel or person name, and press Return to OPEN it. This is
              NOT the same as full-text Search (Cmd/Ctrl+G or the magnifier icon),
              which finds messages and will NOT navigate you to a channel. If you
              opened Search by mistake, press Escape and use the quick switcher.
            - Do NOT assume a channel such as #general exists. If the switcher reports
              "couldn't find anything", press Escape and pick a channel from the left
              sidebar, or just use the conversation that is already open. Do not get
              stuck retyping a channel name that does not exist.
            - Prefer keyboard flow (quick switcher → type → Return → type message →
              Return) over clicking screen coordinates, which is brittle in these apps.

            {os_instructions}

            web_scrape
              - Fetch content from html websites and APIs
              - Save as text, JSON, or binary files
              - Content is cached locally for later use
              - PREFER this as the FIRST tool for fetching any known URL. Do not hand-roll
                HTTP fetches in shell scripts (curl/wget/python urllib). Reserve browser
                automation for JS-heavy or interactive sites.
            cache
              - Manage your cached files
              - List, view, delete files
              - Clear all cached data
            The capability automatically manages:
            - Cache directory: {cache_dir}
            - File organization and cleanup
            "#,
            os_instructions = os_specific_instructions,
            cache_dir = cache_dir.display()
        };

        Self {
            tool_router: Self::tool_router(),
            cache_dir,
            active_resources: Arc::new(Mutex::new(HashMap::new())),
            http_client: Arc::new(Mutex::new(None)),
            web_scrape_timeout: std::time::Duration::from_secs(WEB_SCRAPE_TIMEOUT_SECS),
            instructions,
            system_automation,
        }
    }

    fn http_client(&self) -> Result<Client, ErrorData> {
        let mut slot = self
            .http_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = slot.as_ref() {
            return Ok(client.clone());
        }

        // macOS SystemConfiguration can return -36 while several extension
        // instances read the system proxy settings concurrently.
        let _build_guard = WEB_SCRAPE_CLIENT_BUILD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = Client::builder()
            .user_agent(WEB_SCRAPE_USER_AGENT)
            .timeout(std::time::Duration::from_secs(WEB_SCRAPE_TIMEOUT_SECS))
            .build()
            .map_err(|error| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to initialize the web client: {error}"),
                    None,
                )
            })?;
        *slot = Some(client.clone());
        Ok(client)
    }

    // Helper function to generate a cache file path
    fn get_cache_path(&self, prefix: &str, extension: &str) -> PathBuf {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        self.cache_dir
            .join(format!("{}_{}.{}", prefix, timestamp, extension))
    }

    // Helper function to save content to cache
    async fn save_to_cache(
        &self,
        content: &[u8],
        prefix: &str,
        extension: &str,
    ) -> Result<PathBuf, ErrorData> {
        let cache_path = self.get_cache_path(prefix, extension);
        tokio::fs::write(&cache_path, content).await.map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to write to cache: {}", e),
                None,
            )
        })?;
        Ok(cache_path)
    }

    // Helper function to register a file as a resource
    fn register_as_resource(&self, cache_path: &PathBuf, mime_type: &str) -> Result<(), ErrorData> {
        let uri = Url::from_file_path(cache_path)
            .map_err(|_| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Invalid cache path".to_string(),
                    None,
                )
            })?
            .to_string();

        let resource = ResourceContents::TextResourceContents {
            uri: uri.clone(),
            text: String::new(), // We'll read it when needed
            mime_type: Some(mime_type.to_string()),
            meta: None,
        };

        self.active_resources.lock().unwrap().insert(uri, resource);
        Ok(())
    }

    /// Fetch content from a web page, API, or feed and save a cached copy
    #[tool(
        name = "web_scrape",
        description = "
            Fetch an HTTP(S) URL for simple web research, APIs, RSS/Atom feeds, and web or news search-result URLs.
            Text and JSON content is returned inline so it can be used immediately, and a cached copy is also saved.
            Prefer this over an automation script when the URL is already known. The content can be saved as:
            - text (for HTML pages)
            - json (for API responses)
            - binary (for images and other files)
            Large responses are truncated inline but remain complete in the cached file.
        "
    )]
    pub async fn web_scrape(
        &self,
        params: Parameters<WebScrapeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let url = &params.url;
        let save_as = params.save_as;
        let http_client = self.http_client()?;

        // Fetch the content, with ONE retry on transient failures only —
        // connect errors, 429, and 5xx. Deterministic client errors (403/404/…)
        // fail immediately with a status-preserving message plus a recovery
        // hint (issue #25). Timeouts are deliberately NOT retried: the client
        // already waits up to WEB_SCRAPE_TIMEOUT_SECS, so a retry against a
        // server that is still hung would double worst-case latency to ~60 s
        // for no realistic gain.
        let mut response = None;
        let mut last_error = String::new();
        for attempt in 0..2 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    WEB_SCRAPE_RETRY_BACKOFF_MS,
                ))
                .await;
            }
            match http_client
                .get(url)
                .timeout(self.web_scrape_timeout)
                .header("Accept", "text/markdown, */*")
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        response = Some(resp);
                        break;
                    }
                    last_error = format!(
                        "HTTP request failed with status: {status}.{}",
                        web_scrape_status_hint(status)
                    );
                    if !web_scrape_status_is_retryable(status) {
                        break;
                    }
                }
                Err(e) => {
                    last_error = format!("Failed to fetch URL: {e}");
                    if !e.is_connect() {
                        break;
                    }
                }
            }
        }
        let Some(response) = response else {
            return Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, last_error, None));
        };

        // Process based on save_as parameter
        let (content, extension, mime_type, inline_content) = match save_as {
            SaveAsFormat::Text => {
                let text = response.text().await.map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to get text: {}", e),
                        None,
                    )
                })?;
                (text.as_bytes().to_vec(), "txt", "text/plain", Some(text))
            }
            SaveAsFormat::Json => {
                let text = response.text().await.map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to get text: {}", e),
                        None,
                    )
                })?;
                // Verify it's valid JSON
                serde_json::from_str::<serde_json::Value>(&text).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Invalid JSON response: {}", e),
                        None,
                    )
                })?;
                (
                    text.as_bytes().to_vec(),
                    "json",
                    "application/json",
                    Some(text),
                )
            }
            SaveAsFormat::Binary => {
                let bytes = response.bytes().await.map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to get bytes: {}", e),
                        None,
                    )
                })?;
                (bytes.to_vec(), "bin", "application/octet-stream", None)
            }
        };

        // Save to cache
        let cache_path = self.save_to_cache(&content, "web", extension).await?;

        // Register as a resource
        self.register_as_resource(&cache_path, mime_type)?;

        let mut result = format!("Content saved to: {}", cache_path.display());
        if let Some(inline_content) = inline_content {
            let (inline_content, truncated) = bounded_web_content(&inline_content);
            result.push_str("\n\nFetched content:\n");
            result.push_str(inline_content);
            if truncated {
                result.push_str("\n\n[Inline content truncated; use the cached file for the complete response.]");
            }
        }

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    /// Create and run small scripts for automation tasks
    #[cfg(target_os = "windows")]
    #[tool(
        name = "automation_script",
        description = "
            Create and run small PowerShell or Batch scripts for automation tasks.
            PowerShell is recommended for most tasks.

            This can run network-aware scripts for web, API, RSS, or news searches when no dedicated search tool exists.
            When embedding a multiline script inside execute_code, beware: String.raw`...` ONLY preserves backslashes.
            It does NOT make ${...} literal: every ${...} (PowerShell's ${env:Path} included) is still parsed as a
            JavaScript expression, and any backtick in the script (PowerShell's escape character) terminates the
            template literal early. Escape a literal dollar-brace as ${\"$\"}{ , or pass the script as a plain quoted
            JS string with \\n escapes, or write it to a file with developer/text_editor (write) and run that file.

            The script is saved to a temporary file and executed.
            Some examples:
            - Sort unique lines: Get-Content file.txt | Sort-Object -Unique
            - Extract CSV column: Import-Csv file.csv | Select-Object -ExpandProperty Column2
            - Find text: Select-String -Pattern 'pattern' -Path file.txt
        "
    )]
    pub async fn automation_script(
        &self,
        params: Parameters<AutomationScriptParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.automation_script_impl(params).await
    }

    /// Create and run small scripts for automation tasks
    #[cfg(not(target_os = "windows"))]
    #[tool(
        name = "automation_script",
        description = "
            Create and run small scripts for automation tasks.
            Supports Shell and Ruby (on macOS).

            This can run network-aware scripts for web, API, RSS, or news searches when no dedicated search tool exists.
            When embedding a multiline script inside execute_code, beware: String.raw`...` ONLY preserves backslashes.
            It does NOT make ${...} literal: every ${...} (bash's ${VAR:-default} or ${!v} included) is still parsed
            as a JavaScript expression, and any backtick in the script (command substitution, markdown fences)
            terminates the template literal early. Escape a literal dollar-brace as ${\"$\"}{ , or pass the script as a
            plain quoted JS string with \\n escapes, or write it to a file with developer/text_editor (write) and run
            that file.

            The script is saved to a temporary file and executed.
            Consider using shell script (bash) for most simple tasks first.
            Ruby is useful for text processing or when you need more sophisticated scripting capabilities.
            Some examples of shell:
                - create a sorted list of unique lines: sort file.txt | uniq
                - extract 2nd column in csv: awk -F ',' '{ print $2}'
                - pattern matching: grep pattern file.txt
        "
    )]
    pub async fn automation_script(
        &self,
        params: Parameters<AutomationScriptParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.automation_script_impl(params).await
    }

    #[allow(clippy::too_many_lines)]
    async fn automation_script_impl(
        &self,
        params: Parameters<AutomationScriptParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let language = params.language;
        let script = &params.script;
        let save_output = params.save_output;

        refuse_secret_access(script, matches!(language, ScriptLanguage::Shell))?;

        // Create a temporary directory for the script
        let script_dir = tempfile::tempdir().map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to create temporary directory: {}", e),
                None,
            )
        })?;

        let (shell, shell_arg) = self.system_automation.get_shell_command();

        let command = match language {
            ScriptLanguage::Shell | ScriptLanguage::Batch => {
                let script_path = script_dir.path().join(format!(
                    "script.{}",
                    if cfg!(windows) { "bat" } else { "sh" }
                ));
                fs::write(&script_path, script).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to write script: {}", e),
                        None,
                    )
                })?;

                // Set execute permissions on Unix systems
                #[cfg(unix)]
                {
                    let mut perms = fs::metadata(&script_path)
                        .map_err(|e| {
                            ErrorData::new(
                                ErrorCode::INTERNAL_ERROR,
                                format!("Failed to get file metadata: {}", e),
                                None,
                            )
                        })?
                        .permissions();
                    perms.set_mode(0o755); // rwxr-xr-x
                    fs::set_permissions(&script_path, perms).map_err(|e| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("Failed to set execute permissions: {}", e),
                            None,
                        )
                    })?;
                }

                script_path.display().to_string()
            }
            ScriptLanguage::Ruby => {
                let script_path = script_dir.path().join("script.rb");
                fs::write(&script_path, script).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to write script: {}", e),
                        None,
                    )
                })?;

                format!("ruby {}", script_path.display())
            }
            ScriptLanguage::Powershell => {
                let script_path = script_dir.path().join("script.ps1");
                fs::write(&script_path, script).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to write script: {}", e),
                        None,
                    )
                })?;

                script_path.display().to_string()
            }
        };

        // Run the script
        let output = automation_script_command(&language, shell, shell_arg, &command)
            .output()
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to run script: {}", e),
                    None,
                )
            })?;

        let output_str = String::from_utf8_lossy(&output.stdout).into_owned();
        let error_str = String::from_utf8_lossy(&output.stderr).into_owned();

        let succeeded = output.status.success();
        let mut result = if succeeded {
            format!("Script completed successfully.\n\nOutput:\n{}", output_str)
        } else {
            format!(
                "Script failed with error code {}.\n\nError:\n{}\nOutput:\n{}",
                output.status, error_str, output_str
            )
        };

        // Save output if requested
        if save_output && !output_str.is_empty() {
            let cache_path = self
                .save_to_cache(output_str.as_bytes(), "script_output", "txt")
                .await?;
            result.push_str(&format!("\n\nOutput saved to: {}", cache_path.display()));

            // Register as a resource
            self.register_as_resource(&cache_path, "text")?;
        }

        if succeeded {
            Ok(CallToolResult::success(vec![Content::text(result)]))
        } else {
            Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, result, None))
        }
    }

    /// Control the computer using system automation
    #[cfg(target_os = "windows")]
    #[tool(
        name = "computer_control",
        description = "
            Control the computer using Windows system automation.

            Features available:
            - PowerShell automation for system control
            - UI automation through PowerShell
            - File and system management
            - Windows-specific features and settings

            Can be combined with screenshot tool for visual task assistance.
        "
    )]
    pub async fn computer_control(
        &self,
        params: Parameters<ComputerControlParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.computer_control_impl(params).await
    }

    /// Control the computer using system automation
    #[cfg(target_os = "macos")]
    #[tool(
        name = "computer_control",
        description = "
            Control the computer using AppleScript (macOS only). Automate applications and system features.

            Key capabilities:
            - Control Applications: Launch, quit, manage apps (Mail, Safari, iTunes, etc)
                - Interact with app-specific feature: (e.g, edit documents, process photos)
                - Perform tasks in third-party apps that support AppleScript
            - UI Automation: Simulate user interactions like, clicking buttons, select menus, type text, filling out forms
            - System Control: Manage settings (volume, brightness, wifi), shutdown/restart, monitor events
            - Web & Email: Open URLs, web automation, send/organize emails, handle attachments
            - Media: Manage music libraries, photo collections, playlists
            - File Operations: Organize files/folders
            - Integration: Calendar, reminders, messages
            - Data: Interact with spreadsheets and documents

            Can be combined with screenshot tool for visual task assistance.
        "
    )]
    pub async fn computer_control(
        &self,
        params: Parameters<ComputerControlParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.computer_control_impl(params).await
    }

    /// Control the computer using system automation
    #[cfg(target_os = "linux")]
    #[tool(
        name = "computer_control",
        description = "
            Control the computer using Linux system automation.

            Features available:
            - Shell scripting for system control
            - X11/Wayland window management
            - D-Bus for system services
            - File and system management
            - Desktop environment control (GNOME, KDE, etc.)
            - Process management and monitoring
            - System settings and configurations

            Can be combined with screenshot tool for visual task assistance.
        "
    )]
    pub async fn computer_control(
        &self,
        params: Parameters<ComputerControlParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.computer_control_impl(params).await
    }

    /// Control the computer using system automation (fallback for other OS)
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    #[tool(
        name = "computer_control",
        description = "Control the computer using system automation. Features available depend on your operating system. Can be combined with screenshot tool for visual task assistance."
    )]
    pub async fn computer_control(
        &self,
        params: Parameters<ComputerControlParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.computer_control_impl(params).await
    }

    async fn computer_control_impl(
        &self,
        params: Parameters<ComputerControlParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let script = &params.script;
        let save_output = params.save_output;

        refuse_secret_access(script, false)?;

        // Use platform-specific automation. execute_system_script returns Err
        // (with stderr + exit status) when the underlying osascript/PowerShell
        // actually fails, so a failed UI action surfaces as a tool error the
        // model can react to — instead of a fake "success" that makes it retry
        // blindly and "circle".
        //
        // Guard against HANGS. A UI-automation script can block for a long time
        // when the target UI is busy or stuck behind a modal/overlay/search popup
        // (on macOS this shows up as AppleEvent error -1712 after the ~2-minute
        // default Apple-event timeout). Two minutes per stuck call is a huge time
        // and token sink and a major cause of the agent "circling". Bound it with
        // a watchdog so a hung action fails fast with actionable guidance instead.
        // OS-invariant: runs the blocking backend call under a tokio timeout.
        let timeout_secs = std::env::var("BIOROUTER_COMPUTER_CONTROL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|s| *s > 0)
            .unwrap_or(45);
        let automation = Arc::clone(&self.system_automation);
        let script_owned = script.to_string();
        let run =
            tokio::task::spawn_blocking(move || automation.execute_system_script(&script_owned));
        let output =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), run).await {
                Ok(join) => join
                    .map_err(|e| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("Computer control task failed to run: {e}"),
                            None,
                        )
                    })?
                    .map_err(|e| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("Computer control script failed: {e}"),
                            None,
                        )
                    })?,
                Err(_elapsed) => {
                    return Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!(
                            "Computer control script timed out after {timeout_secs}s and was \
                         abandoned. The UI is almost certainly blocked by a modal dialog, an \
                         open search/autocomplete popup, a menu, or a pending permission prompt. \
                         Do NOT re-run this script. Instead: take a screenshot to see the current \
                         state, press Escape to dismiss any popup, and try a different approach."
                        ),
                        None,
                    ));
                }
            };

        // Many UI-automation scripts succeed without producing stdout (e.g.
        // "activate app", "click button"). Distinguish that from a result with
        // output so the model does not mistake silence for a failure (or vice
        // versa) and re-run the same step.
        let mut result = if output.trim().is_empty() {
            "Script ran with no errors and produced no output (this is normal for UI actions \
             like activating an app or clicking). Verify the effect (e.g. take a screenshot) \
             before assuming it did or did not work, and do not blindly repeat the same step."
                .to_string()
        } else {
            format!("Script completed successfully.\n\nOutput:\n{}", output)
        };

        // Save output if requested
        if save_output && !output.is_empty() {
            let cache_path = self
                .save_to_cache(output.as_bytes(), "automation_output", "txt")
                .await?;
            result.push_str(&format!("\n\nOutput saved to: {}", cache_path.display()));

            // Register as a resource
            self.register_as_resource(&cache_path, "text")?;
        }

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    /// Process Excel (XLSX) files to read and manipulate spreadsheet data
    #[tool(
        name = "xlsx_tool",
        description = "
            Process Excel (XLSX) files to read and manipulate spreadsheet data.
            Supports operations:
            - list_worksheets: List all worksheets in the workbook (returns name, index, column_count, row_count)
            - get_columns: Get column names from a worksheet (returns values from the first row)
            - get_range: Get values and formulas from a cell range (e.g., 'A1:C10') (returns a 2D array organized as [row][column])
            - find_text: Search for text in a worksheet (returns a list of (row, column) coordinates)
            - update_cell: Update a single cell's value (returns confirmation message)
            - get_cell: Get value and formula from a specific cell (returns both value and formula if present)
            - save: Save changes back to the file (returns confirmation message)

            Use this when working with Excel spreadsheets to analyze or modify data.
        "
    )]
    pub async fn xlsx_tool(
        &self,
        params: Parameters<XlsxToolParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let path = &params.path;
        let operation = params.operation;

        match operation {
            XlsxOperation::ListWorksheets => {
                let xlsx = xlsx_tool::XlsxTool::new(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                let worksheets = xlsx
                    .list_worksheets()
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "{:#?}",
                    worksheets
                ))]))
            }
            XlsxOperation::GetColumns => {
                let xlsx = xlsx_tool::XlsxTool::new(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                let worksheet = if let Some(name) = &params.worksheet {
                    xlsx.get_worksheet_by_name(name).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                } else {
                    xlsx.get_worksheet_by_index(0).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                };
                let columns = xlsx
                    .get_column_names(worksheet)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "{:#?}",
                    columns
                ))]))
            }
            XlsxOperation::GetRange => {
                let range = params.range.as_ref().ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'range' parameter".to_string(),
                        None,
                    )
                })?;

                let xlsx = xlsx_tool::XlsxTool::new(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                let worksheet = if let Some(name) = &params.worksheet {
                    xlsx.get_worksheet_by_name(name).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                } else {
                    xlsx.get_worksheet_by_index(0).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                };
                let range_data = xlsx
                    .get_range(worksheet, range)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "{:#?}",
                    range_data
                ))]))
            }
            XlsxOperation::FindText => {
                let search_text = params.search_text.as_ref().ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'search_text' parameter".to_string(),
                        None,
                    )
                })?;

                let case_sensitive = params.case_sensitive;

                let xlsx = xlsx_tool::XlsxTool::new(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                let worksheet = if let Some(name) = &params.worksheet {
                    xlsx.get_worksheet_by_name(name).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                } else {
                    xlsx.get_worksheet_by_index(0).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                };
                let matches = xlsx
                    .find_in_worksheet(worksheet, search_text, case_sensitive)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Found matches at: {:#?}",
                    matches
                ))]))
            }
            XlsxOperation::UpdateCell => {
                let row = params.row.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'row' parameter".to_string(),
                        None,
                    )
                })?;
                let col = params.col.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'col' parameter".to_string(),
                        None,
                    )
                })?;
                let value = params.value.as_ref().ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'value' parameter".to_string(),
                        None,
                    )
                })?;

                let worksheet_name = params.worksheet.as_deref().unwrap_or("Sheet1");

                let mut xlsx = xlsx_tool::XlsxTool::new(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                xlsx.update_cell(worksheet_name, row as u32, col as u32, value)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                xlsx.save(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Updated cell ({}, {}) to '{}' in worksheet '{}'",
                    row, col, value, worksheet_name
                ))]))
            }
            XlsxOperation::Save => {
                let xlsx = xlsx_tool::XlsxTool::new(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                xlsx.save(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    "File saved successfully.",
                )]))
            }
            XlsxOperation::GetCell => {
                let row = params.row.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'row' parameter".to_string(),
                        None,
                    )
                })?;

                let col = params.col.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'col' parameter".to_string(),
                        None,
                    )
                })?;

                let xlsx = xlsx_tool::XlsxTool::new(path)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                let worksheet = if let Some(name) = &params.worksheet {
                    xlsx.get_worksheet_by_name(name).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                } else {
                    xlsx.get_worksheet_by_index(0).map_err(|e| {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                    })?
                };
                let cell_value = xlsx
                    .get_cell_value(worksheet, row as u32, col as u32)
                    .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "{:#?}",
                    cell_value
                ))]))
            }
        }
    }

    /// Process DOCX files to extract text and create/update documents
    #[tool(
        name = "docx_tool",
        description = "
            Process DOCX files to extract text and create/update documents.
            Supports operations:
            - extract_text: Extract all text content and structure (headings, TOC) from the DOCX
            - update_doc: Create a new DOCX or update existing one with provided content
              Modes:
              - append: Add content to end of document (default)
              - replace: Replace specific text with new content
              - structured: Add content with specific heading level and styling
              - add_image: Add an image to the document (with optional caption)

            Use this when there is a .docx file that needs to be processed or created.
        "
    )]
    pub async fn docx_tool(
        &self,
        params: Parameters<DocxToolParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let path = &params.path;
        let operation = params.operation;

        // Convert enum to string for the existing implementation
        let operation_str = match operation {
            DocxOperation::ExtractText => "extract_text",
            DocxOperation::UpdateDoc => "update_doc",
        };

        // Convert typed params back to JSON for the internal docx_tool impl
        let json_params = params
            .params
            .as_ref()
            .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null));

        let result = crate::computercontroller::docx_tool::docx_tool(
            path,
            operation_str,
            params.content.as_deref(),
            json_params.as_ref(),
        )
        .await
        .map_err(|e| ErrorData::new(e.code, e.message, e.data))?;

        Ok(CallToolResult::success(result))
    }

    /// Process PDF files to extract text and images
    #[tool(
        name = "pdf_tool",
        description = "
            Process PDF files to extract text and images.
            Supports operations:
            - extract_text: Extract all text content from the PDF
            - extract_images: Extract and save embedded images to PNG files

            Use this when there is a .pdf file or files that need to be processed.
        "
    )]
    pub async fn pdf_tool(
        &self,
        params: Parameters<PdfToolParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let path = &params.path;
        let operation = params.operation;

        // Convert enum to string for the existing implementation
        let operation_str = match operation {
            PdfOperation::ExtractText => "extract_text",
            PdfOperation::ExtractImages => "extract_images",
        };

        let result =
            crate::computercontroller::pdf_tool::pdf_tool(path, operation_str, &self.cache_dir)
                .await
                .map_err(|e| ErrorData::new(e.code, e.message, e.data))?;

        Ok(CallToolResult::success(result))
    }

    /// Manage cached files and data
    #[tool(
        name = "cache",
        description = "
            Manage cached files and data:
            - list: List all cached files
            - view: View content of a cached file
            - delete: Delete a cached file
            - clear: Clear all cached files
        "
    )]
    pub async fn cache(
        &self,
        params: Parameters<CacheParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let command = params.0.command;
        let path = params.0.path.as_deref();

        // Issue #63 review, finding 2. `view` and `delete` take whatever path
        // the model supplies, so this tool reaches Biorouter's machine-wide
        // memory store as readily as the memory tools do — and the #63 consent
        // gate, which matches tool *names*, never sees it. The store is refused
        // as a place; see `biorouter_mcp::memory::is_in_global_memory_store`.
        if let Some(path) = path {
            let candidate = std::path::Path::new(path);
            if crate::memory::is_in_global_memory_store(candidate) {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    crate::memory::global_memory_store_refusal(candidate),
                    None,
                ));
            }
        }

        match command {
            CacheCommand::List => {
                let mut files = Vec::new();
                for entry in fs::read_dir(&self.cache_dir).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to read cache directory: {}", e),
                        None,
                    )
                })? {
                    let entry = entry.map_err(|e| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("Failed to read directory entry: {}", e),
                            None,
                        )
                    })?;
                    files.push(format!("{}", entry.path().display()));
                }
                files.sort();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Cached files:\n{}",
                    files.join("\n")
                ))]))
            }
            CacheCommand::View => {
                let path = path.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'path' parameter for view".to_string(),
                        None,
                    )
                })?;

                let content = tokio::fs::read_to_string(path).await.map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to read file: {}", e),
                        None,
                    )
                })?;

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Content of {}:\n\n{}",
                    path, content
                ))]))
            }
            CacheCommand::Delete => {
                let path = path.ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "Missing 'path' parameter for delete".to_string(),
                        None,
                    )
                })?;

                fs::remove_file(path).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to delete file: {}", e),
                        None,
                    )
                })?;

                // Remove from active resources if present
                if let Ok(url) = Url::from_file_path(path) {
                    self.active_resources
                        .lock()
                        .unwrap()
                        .remove(&url.to_string());
                }

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Deleted file: {}",
                    path
                ))]))
            }
            CacheCommand::Clear => {
                fs::remove_dir_all(&self.cache_dir).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to clear cache directory: {}", e),
                        None,
                    )
                })?;
                fs::create_dir_all(&self.cache_dir).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to recreate cache directory: {}", e),
                        None,
                    )
                })?;

                // Clear active resources
                self.active_resources.lock().unwrap().clear();

                Ok(CallToolResult::success(vec![Content::text(
                    "Cache cleared successfully.",
                )]))
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ComputerControllerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "biorouter-computercontroller".to_string(),
                // User-facing display name. The internal id/routing key stays
                // "computercontroller" (tools are prefixed `computercontroller__`),
                // but clients that honor the MCP `title` show "Computer Controller".
                title: Some("Computer Controller".to_string()),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            instructions: Some(self.instructions.clone()),
            ..Default::default()
        }
    }

    async fn list_resources(
        &self,
        _pagination: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let active_resources = self.active_resources.lock().unwrap();
        let resources: Vec<Resource> = active_resources
            .keys()
            .map(|uri| {
                RawResource::new(
                    uri.clone(),
                    uri.split('/').next_back().unwrap_or("").to_string(),
                )
                .no_annotation()
            })
            .collect();
        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let active_resources = self.active_resources.lock().unwrap();
        let resource = active_resources.get(&params.uri).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!("Resource not found: {}", params.uri),
                None,
            )
        })?;

        // Clone the resource to return
        Ok(ReadResourceResult {
            contents: vec![resource.clone()],
        })
    }
}

#[cfg(test)]
mod secret_guard_tests {
    use super::*;
    use crate::secret_guard::h1_fixtures::{h1_leaking_spellings, h1_ordinary_commands, FakeHome};

    /// H1, measured through `automation_script` as well as `developer__shell`:
    /// the same rows, against a throwaway HOME, through this server's own check.
    #[test]
    fn h1_a_shell_script_body_is_refused_for_every_spelling() {
        let fake = FakeHome::new();
        let env = fake.env();
        let leaked: Vec<String> = h1_leaking_spellings(&fake.home)
            .into_iter()
            .filter(|(_, script)| {
                refuse_secret_access_in(script, true, &fake.project, &env).is_ok()
            })
            .map(|(name, script)| format!("  {name}: {script}"))
            .collect();
        assert!(
            leaked.is_empty(),
            "{} spelling(s) reached a secret through automation_script:\n{}",
            leaked.len(),
            leaked.join("\n")
        );
        for script in h1_ordinary_commands(&fake.home) {
            assert!(
                refuse_secret_access_in(&script, true, &fake.project, &env).is_ok(),
                "refused an ordinary script: {script}"
            );
        }
    }

    /// Ruby, AppleScript and PowerShell are not shells: their path literals and
    /// the strings they hand to a shell are what can be judged.
    #[test]
    fn h1_a_non_shell_script_is_refused_by_its_literals_and_shell_outs() {
        let fake = FakeHome::new();
        let env = fake.env();
        for script in [
            "puts File.read(File.expand_path('~/.aws/credentials'))",
            "puts `cd ~/.aws && cat credentials`",
            "do shell script \"cd ~/.aws && head credentials\"",
            "Get-Content ~/.ssh/id_ed25519",
            "system(\"cat $HOME/.config/biorouter/secrets.yaml\")",
        ] {
            assert!(
                refuse_secret_access_in(script, false, &fake.project, &env).is_err(),
                "a non-shell script reached a secret: {script}"
            );
        }
        for script in [
            "puts File.read('data.csv')",
            "tell application \"Finder\" to activate",
            "Get-ChildItem ~/Documents",
        ] {
            assert!(
                refuse_secret_access_in(script, false, &fake.project, &env).is_ok(),
                "refused an ordinary script: {script}"
            );
        }
    }
}

#[cfg(test)]
mod web_and_script_tests {
    use super::*;
    use rmcp::model::RawContent;
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                RawContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Child half of the two `automation_script` leak tests below.
    ///
    /// The leak is in the *inherited* environment, so exercising it means
    /// controlling this process's environment — and `set_var` is unsound in a
    /// threaded test binary. So the parent re-invokes this very test binary
    /// with the canary exported and this half spawns a real child through the
    /// real `automation_script_command`, printing the environment that child
    /// actually received. Same shape as the `developer::shell` probe that
    /// guards the shell tool. `#[ignore]` keeps it out of a normal run.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn leak_probe_prints_automation_script_child_env() {
        let mut cmd = automation_script_command(&ScriptLanguage::Shell, "bash", "-c", "printenv");
        let out = cmd.output().await.expect("automation child must spawn");
        let raw = String::from_utf8_lossy(&out.stdout);
        let canary = std::env::var("BR_TEST_CANARY").unwrap_or_default();
        // Report BioRouter's namespace, the injected variables, and any line
        // carrying the canary under *any* key — so a copy of the secret under a
        // different name is still caught. Everything else is dropped: printing
        // the whole environment of whoever runs the suite would be its own leak.
        let report = raw
            .lines()
            .filter(|line| {
                let key = line.split('=').next().unwrap_or("");
                if key == "BR_TEST_CANARY" {
                    return false; // the channel carrying the canary is not the leak
                }
                key.starts_with("BIOROUTER_")
                    || key.starts_with("GOOSE_")
                    || matches!(
                        key,
                        "PATH" | "HOME" | "BR_TEST_USER_VAR" | "SPOKEAGENT_PASSCODE"
                    )
                    || (!canary.is_empty() && line.contains(&canary))
            })
            .collect::<Vec<_>>()
            .join("\n");
        println!("BEGIN_CHILD_ENV");
        println!("{report}");
        println!("END_CHILD_ENV");
    }

    #[cfg(unix)]
    fn run_automation_leak_probe(canary: &str) -> String {
        run_leak_probe_named("leak_probe_prints_automation_script_child_env", canary)
    }

    /// Run the named ignored probe in a fresh copy of this test binary whose
    /// environment is the daemon's: the auth secret, the port, an
    /// extension-declared credential and an ordinary user variable.
    #[cfg(unix)]
    fn run_leak_probe_named(probe_fn: &str, canary: &str) -> String {
        let m = module_path!();
        let without_crate = m.split_once("::").map(|(_, rest)| rest).unwrap_or(m);
        let probe = format!("{without_crate}::{probe_fn}");
        let exe = std::env::current_exe().expect("test binary path");
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                &probe,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BIOROUTER_SERVER__SECRET_KEY", canary)
            .env("BR_TEST_CANARY", canary)
            .env("BIOROUTER_PORT", "54321")
            .env("SPOKEAGENT_PASSCODE", "extension-credential-ok")
            .env("BR_TEST_USER_VAR", "user-env-ok")
            .output()
            .expect("re-invoking the test binary must work");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        stdout
            .split_once("BEGIN_CHILD_ENV\n")
            .and_then(|(_, rest)| rest.split_once("END_CHILD_ENV"))
            .map(|(body, _)| body.to_string())
            .unwrap_or_else(|| {
                panic!(
                    "probe produced no child environment.\nstdout:\n{stdout}\nstderr:\n{}",
                    String::from_utf8_lossy(&out.stderr)
                )
            })
    }

    /// Issue #57. `automation_script` runs a script the model wrote, so its
    /// child is exactly as agent-controlled as the `developer` shell tool — but
    /// it has its own spawn path and never touches `configure_shell_command`.
    /// Until this was fixed, a script could read the daemon's auth secret out
    /// of its own environment and call `biorouterd` as a fully authenticated
    /// client (verified end to end against a live daemon: `GET /sessions`
    /// returned every session on the machine).
    #[cfg(unix)]
    #[test]
    fn daemon_secret_never_reaches_an_automation_script_child() {
        const CANARY: &str = "canary-daemon-secret-cc-57";
        let child_env = run_automation_leak_probe(CANARY);
        assert!(
            !child_env.contains(CANARY),
            "issue #57: the daemon's auth secret reached a computer_controller \
             automation_script, so any session can call biorouterd as an authenticated \
             client.\nchild env:\n{child_env}"
        );
        assert!(
            !child_env.contains("BIOROUTER_SERVER__SECRET_KEY"),
            "the key name itself must be gone, not just the value:\n{child_env}"
        );
    }

    /// Child half of [`daemon_secret_never_reaches_a_computer_control_child`].
    ///
    /// `computer_control` runs a model-authored *system* script through the
    /// per-platform automation backend — `osascript` on macOS, `powershell` on
    /// Windows, a generated `python3` script on Linux. AppleScript's
    /// `do shell script` and Python's `subprocess.run(shell=True)` are both
    /// full shells, so this is a second agent-controlled child in the same
    /// built-in extension, on a different spawn path from `automation_script`.
    /// Unlike `printenv` in a shell, `osascript` returns `do shell script`
    /// output as a single unsplittable blob, so this probe reports **only
    /// booleans** — never environment text. Printing the raw result would dump
    /// the environment of whoever runs the suite (real API keys included) into
    /// the test log, which is its own leak.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore]
    async fn leak_probe_prints_computer_control_child_env() {
        let automation = create_system_automation();
        // Filter inside the child too, so the secret never even crosses back.
        let out = automation
            .execute_system_script(
                "do shell script \"printenv | cut -d= -f1 | sort | tr '\\\\n' ' '\"",
            )
            .unwrap_or_default();
        let keys: Vec<&str> = out.split_whitespace().collect();
        let canary = std::env::var("BR_TEST_CANARY").unwrap_or_default();
        // `BR_TEST_CANARY` is the channel that carries the canary *to* the
        // probe; it is inherited by design and is not the leak under test, so
        // it is excluded before counting — otherwise every run reports a leak.
        let saw_canary_value = automation
            .execute_system_script(&format!(
                "do shell script \"printenv | grep -v '^BR_TEST_CANARY=' | grep -c -- {} || true\"",
                shell_quote(&canary)
            ))
            .unwrap_or_default()
            .trim()
            .parse::<u32>()
            .unwrap_or(0)
            > 0;
        println!("BEGIN_CHILD_ENV");
        println!("CANARY_VALUE_PRESENT={saw_canary_value}");
        for key in [
            "BIOROUTER_SERVER__SECRET_KEY",
            "BIOROUTER_ACP_WS_TOKEN",
            "PATH",
            "HOME",
            "BIOROUTER_PORT",
            "SPOKEAGENT_PASSCODE",
            "BR_TEST_USER_VAR",
        ] {
            println!("HAS_{key}={}", keys.contains(&key));
        }
        println!("END_CHILD_ENV");
    }

    /// Single-quote a value for embedding in the `do shell script` string.
    #[cfg(target_os = "macos")]
    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', r"'\''"))
    }

    /// Issue #57, `computer_control`. Verified end to end against a live daemon
    /// before the fix: an AppleScript `do shell script "printenv"` returned
    /// `BIOROUTER_SERVER__SECRET_KEY`.
    #[cfg(target_os = "macos")]
    #[test]
    fn daemon_secret_never_reaches_a_computer_control_child() {
        const CANARY: &str = "canary-daemon-secret-ctl-57";
        let report = run_leak_probe_named("leak_probe_prints_computer_control_child_env", CANARY);
        let says = |line: &str| report.lines().any(|l| l.trim() == line);

        assert!(
            says("CANARY_VALUE_PRESENT=false"),
            "issue #57: the daemon's auth secret reached a computer_control system script, \
             so any session can call biorouterd as an authenticated client.\nprobe:\n{report}"
        );
        assert!(
            says("HAS_BIOROUTER_SERVER__SECRET_KEY=false"),
            "the key name itself must be gone, not just the value:\n{report}"
        );
        assert!(
            says("HAS_BIOROUTER_ACP_WS_TOKEN=false"),
            "the ACP token is a BioRouter credential too:\n{report}"
        );
        // The other direction: the user's environment must survive.
        for kept in [
            "HAS_PATH=true",
            "HAS_HOME=true",
            "HAS_BIOROUTER_PORT=true",
            "HAS_SPOKEAGENT_PASSCODE=true",
            "HAS_BR_TEST_USER_VAR=true",
        ] {
            assert!(
                says(kept),
                "{kept} is missing: stripping the daemon credential must not censor the user's \
                 environment:\n{report}"
            );
        }
    }

    /// The other direction: stripping the daemon's credential must not take the
    /// user's environment (or a legitimately exported extension credential)
    /// with it. A truncated child environment is its own regression — issue #24
    /// was exactly that, for `PATH`.
    #[cfg(unix)]
    #[test]
    fn automation_script_child_still_receives_the_user_environment() {
        let child_env = run_automation_leak_probe("canary-unused");
        for expected in [
            "PATH=",
            "HOME=",
            "BIOROUTER_PORT=54321",
            "BIOROUTER_TERMINAL=1",
            "SPOKEAGENT_PASSCODE=extension-credential-ok",
            "BR_TEST_USER_VAR=user-env-ok",
        ] {
            assert!(
                child_env.lines().any(|l| l.starts_with(expected)),
                "{expected} must survive the strip, got:\n{child_env}"
            );
        }
    }

    #[test]
    fn inline_web_content_respects_utf8_boundaries() {
        let content = format!("{}é", "a".repeat(MAX_INLINE_WEB_CONTENT_BYTES - 1));
        let (bounded, truncated) = bounded_web_content(&content);

        assert!(truncated);
        assert_eq!(bounded.len(), MAX_INLINE_WEB_CONTENT_BYTES - 1);
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[test]
    fn construction_defers_fallible_http_initialization() {
        let server = ComputerControllerServer::new();
        assert!(server.http_client.lock().unwrap().is_none());
    }

    /// Guidance may only claim tools this capability actually registers.
    ///
    /// ⚠ `screen_capture` is registered by `DeveloperServer`, never by this
    /// router — but every screenshot mention here said "in this capability",
    /// so the routing advice pointed at a tool the model would never find under
    /// Computer Control. A capability that describes another's tool must name
    /// the owner, which is what this asserts.
    ///
    /// The check is over the WHOLE instruction string rather than one line,
    /// because the wrong attribution appeared eight times in three
    /// per-platform blocks and two shared ones — fixing the first and missing
    /// the rest is the likely regression.
    #[test]
    fn guidance_never_claims_a_tool_this_capability_does_not_register() {
        let server = ComputerControllerServer::new();
        let owned: Vec<String> = ComputerControllerServer::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        assert!(
            !owned.iter().any(|name| name == "screen_capture"),
            "this test is only meaningful while screen_capture lives in Developer; \
             it now appears in this router and the guidance should be revisited"
        );

        let instructions = server.instructions.clone();
        assert!(
            instructions.contains("screen_capture"),
            "the screenshot routing advice is worth keeping — just attributed"
        );
        for claim in [
            "`screen_capture` is in this capability",
            "`screen_capture` may be available in this capability",
        ] {
            assert!(
                !instructions.contains(claim),
                "guidance claims a Developer tool as its own: {claim}"
            );
        }
        // Every mention names the owner.
        let mentions = instructions.matches("screen_capture").count();
        let attributed = instructions
            .matches("Developer capability's `screen_capture`")
            .count()
            + instructions
                .matches("`screen_capture` belongs to the **Developer** capability")
                .count();
        assert_eq!(
            mentions, attributed,
            "{mentions} screen_capture mentions but only {attributed} name Developer as the owner"
        );
    }

    #[tokio::test]
    async fn web_scrape_returns_text_inline_and_keeps_cached_copy() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<rss><channel><item><title>Apple Watch update</title></item></channel></rss>",
            ))
            .mount(&mock_server)
            .await;

        let cache = tempfile::tempdir().unwrap();
        let mut server = ComputerControllerServer::new();
        server.cache_dir = cache.path().to_path_buf();
        let result = server
            .web_scrape(Parameters(WebScrapeParams {
                url: mock_server.uri(),
                save_as: SaveAsFormat::Text,
            }))
            .await
            .expect("web fetch should succeed");
        let text = text_of(&result);

        assert!(text.contains("Fetched content:"));
        assert!(text.contains("Apple Watch update"));
        let saved_path = text
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Content saved to: ")
            .unwrap();
        assert!(std::path::Path::new(saved_path).is_file());
    }

    // ---- issue #25: web_scrape hardening -------------------------------------

    /// A 403 is deterministic: the error must keep the status (the tool-error
    /// classifier keys on '403' → permission_denied), carry the bot-block hint,
    /// and must NOT be retried.
    #[tokio::test]
    async fn web_scrape_403_keeps_status_adds_hint_and_does_not_retry() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1) // exactly one request — no retry on 403
            .mount(&mock_server)
            .await;

        let cache = tempfile::tempdir().unwrap();
        let mut server = ComputerControllerServer::new();
        server.cache_dir = cache.path().to_path_buf();
        let error = server
            .web_scrape(Parameters(WebScrapeParams {
                url: mock_server.uri(),
                save_as: SaveAsFormat::Text,
            }))
            .await
            .expect_err("403 must be a tool error");

        let message = error.to_string();
        assert!(message.contains("403"), "status must survive: {message}");
        assert!(
            message.contains("blocks automated clients"),
            "403 must carry the bot-block hint: {message}"
        );
    }

    /// A 404 is deterministic too: no retry, and the hint says to verify the URL.
    #[tokio::test]
    async fn web_scrape_404_keeps_status_adds_hint_and_does_not_retry() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache = tempfile::tempdir().unwrap();
        let mut server = ComputerControllerServer::new();
        server.cache_dir = cache.path().to_path_buf();
        let error = server
            .web_scrape(Parameters(WebScrapeParams {
                url: mock_server.uri(),
                save_as: SaveAsFormat::Text,
            }))
            .await
            .expect_err("404 must be a tool error");

        let message = error.to_string();
        assert!(message.contains("404"), "status must survive: {message}");
        assert!(
            message.contains("Verify the URL"),
            "404 must carry the verify-URL hint: {message}"
        );
    }

    /// A transient 500 gets exactly one retry; the second attempt's 200 wins.
    #[tokio::test]
    async fn web_scrape_retries_once_on_5xx_then_succeeds() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .expect(1)
            .with_priority(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("recovered content"))
            .expect(1)
            .with_priority(2)
            .mount(&mock_server)
            .await;

        let cache = tempfile::tempdir().unwrap();
        let mut server = ComputerControllerServer::new();
        server.cache_dir = cache.path().to_path_buf();
        let result = server
            .web_scrape(Parameters(WebScrapeParams {
                url: mock_server.uri(),
                save_as: SaveAsFormat::Text,
            }))
            .await
            .expect("500-then-200 must succeed after one retry");

        assert!(text_of(&result).contains("recovered content"));
    }

    /// A request timeout is NOT retried (review follow-up on #25): the retry
    /// contract is connect errors, 429, and 5xx only. The client already
    /// waited the full request timeout, so a retry against a still-hung
    /// server would double worst-case latency for no realistic gain. The
    /// mock's `expect(1)` is verified on drop — a retry fails the test.
    #[tokio::test]
    async fn web_scrape_timeout_is_not_retried() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(5))
                    .set_body_string("too late"),
            )
            .expect(1) // exactly one request — a timeout must not be retried
            .mount(&mock_server)
            .await;

        let cache = tempfile::tempdir().unwrap();
        let mut server = ComputerControllerServer::new();
        server.cache_dir = cache.path().to_path_buf();
        server.web_scrape_timeout = std::time::Duration::from_millis(200);

        let error = server
            .web_scrape(Parameters(WebScrapeParams {
                url: mock_server.uri(),
                save_as: SaveAsFormat::Text,
            }))
            .await
            .expect_err("a timed-out fetch must be a tool error");
        assert!(
            error.to_string().contains("Failed to fetch URL"),
            "timeout must surface as a fetch failure, got: {error}"
        );
    }

    /// The request must carry the browser-compatible UA — the old bare
    /// `biorouter/1.0` was bot-flagged into 403s.
    #[tokio::test]
    async fn web_scrape_sends_browser_compatible_user_agent() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wiremock::matchers::header(
                "user-agent",
                WEB_SCRAPE_USER_AGENT,
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string("ua ok"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache = tempfile::tempdir().unwrap();
        let mut server = ComputerControllerServer::new();
        server.cache_dir = cache.path().to_path_buf();
        let result = server
            .web_scrape(Parameters(WebScrapeParams {
                url: mock_server.uri(),
                save_as: SaveAsFormat::Text,
            }))
            .await
            .expect("UA-matched fetch should succeed");
        assert!(text_of(&result).contains("ua ok"));
    }

    /// The extension instructions must agree with the tool description: the
    /// old "don't use this as the first tool" line steered the model into
    /// hand-rolled shell+urllib fetches (the issue's failure mode).
    #[test]
    fn instructions_prefer_web_scrape_as_first_fetcher() {
        let server = ComputerControllerServer::new();
        assert!(
            !server
                .instructions
                .contains("don't use this as the first tool"),
            "the contradictory steer must be gone"
        );
        assert!(
            server
                .instructions
                .contains("FIRST tool for fetching any known URL"),
            "instructions must prefer web_scrape for known URLs, got: {}",
            &server.instructions
        );
    }

    /// Retryability contract: 429/5xx retryable, deterministic 4xx not.
    #[test]
    fn web_scrape_retryable_statuses() {
        use reqwest::StatusCode;
        assert!(web_scrape_status_is_retryable(
            StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(web_scrape_status_is_retryable(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(web_scrape_status_is_retryable(StatusCode::BAD_GATEWAY));
        assert!(!web_scrape_status_is_retryable(StatusCode::FORBIDDEN));
        assert!(!web_scrape_status_is_retryable(StatusCode::NOT_FOUND));
        assert!(!web_scrape_status_is_retryable(StatusCode::BAD_REQUEST));
    }

    /// Collapse every run of whitespace to one space, so a phrase can be looked
    /// for in prose that is hard-wrapped inside a source literal.
    ///
    /// ⚠ This is the fix for a real red build, not tidiness. The two
    /// `automation_script` descriptions below are `#[cfg]` twins whose shared
    /// sentences sit at different column offsets, so the same phrase wraps in
    /// one and not the other: "terminates the template literal" is contiguous
    /// in the unix variant and split by a newline plus twelve spaces in the
    /// windows one, and a `contains` over the raw text passed on macOS and
    /// Linux while failing on windows-latest for a full week. Re-flowing the
    /// literal would have moved the trap to the next edit rather than removing
    /// it: the assertion is about the words, and the line breaks in a `#[tool]`
    /// description are free variables that any rewording disturbs.
    fn unwrapped(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The automation_script description must carry the CORRECTED String.raw
    /// caveats from #23 (String.raw does not neutralise ${...} interpolation
    /// or backticks), not the old unconditional "use String.raw" steer that
    /// produced the very parse failures #23 fixed. Applies to both platform
    /// variants, and the asserted phrases are shared, which is exactly why every
    /// match here goes through [`unwrapped`].
    #[test]
    fn automation_script_description_carries_corrected_string_raw_caveats() {
        let server = ComputerControllerServer::new();
        let tool = server
            .tool_router
            .list_all()
            .into_iter()
            .find(|t| t.name == "automation_script")
            .expect("automation_script is registered");
        let description = unwrapped(tool.description.as_deref().unwrap_or_default());
        assert!(
            !description.contains("so backslashes remain intact"),
            "the old unconditional String.raw steer must be gone: {description}"
        );
        assert!(
            description.contains("ONLY preserves backslashes"),
            "must state String.raw's actual (narrow) effect: {description}"
        );
        assert!(
            description.contains("does NOT make ${...} literal"),
            "must correct the dollar-brace belief: {description}"
        );
        assert!(
            description.contains("terminates the template literal"),
            "must warn that payload backticks end the literal: {description}"
        );
        assert!(
            description.contains(r#"${"$"}{"#),
            "must teach the literal dollar-brace escape: {description}"
        );
    }

    /// The test above can only see the description that this platform
    /// *compiled*, which is the whole reason the break was invisible for a
    /// week: macOS and Linux build the `#[cfg(not(target_os = "windows"))]`
    /// variant, so the windows one is unreachable from every runner except
    /// windows-latest, and a phrase can go missing there with nothing to say
    /// so. This reads both out of the source instead, so either variant losing
    /// a caveat, or re-wrapping one past a `contains`, fails everywhere.
    ///
    /// A source scan is the right instrument and not a workaround: the property
    /// is "both spellings of this description say the same things", and that is
    /// a statement about the file, not about a running server.
    #[test]
    fn both_platform_descriptions_carry_the_same_string_raw_caveats() {
        const SOURCE: &str = include_str!("mod.rs");
        // The body of each `#[tool]` attribute, taken WITHOUT a byte index:
        // `split` on the needle drops it and hands back the tails (the first
        // chunk is everything before the first match, hence `skip(1)`), and
        // `split_once` on the attribute's closing `)]` keeps the part in front
        // of it. Cutting on substrings rather than on offsets is what makes the
        // char-boundary question unaskable — the same move `guardrails::
        // tool_output` made, and the reason the `string_slice` restriction lint
        // is on.
        //
        // The needle is the attribute's own text, so this test's source cannot
        // match itself: its copy of the needle is escaped.
        let blocks: Vec<String> = SOURCE
            .split("name = \"automation_script\",")
            .skip(1)
            .map(|tail| {
                let (block, _) = tail
                    .split_once("\n    )]")
                    .expect("a #[tool(...)] attribute closes with `)]` at the impl indent");
                unwrapped(block)
            })
            .collect();
        assert_eq!(
            blocks.len(),
            2,
            "expected exactly one automation_script description per platform cfg"
        );

        for (i, block) in blocks.iter().enumerate() {
            for phrase in [
                "ONLY preserves backslashes",
                "does NOT make ${...} literal",
                "terminates the template literal",
                // Source spelling, not runtime: the description is a Rust
                // string literal, so the escape the model is taught as
                // `${"$"}{` is written `${\"$\"}{` in the file this scan reads.
                // Both cfg variants escape it identically, which is the
                // property being pinned.
                r#"${\"$\"}{"#,
            ] {
                assert!(
                    block.contains(phrase),
                    "automation_script description #{i} is missing {phrase:?}: {block}"
                );
            }
            assert!(
                !block.contains("so backslashes remain intact"),
                "automation_script description #{i} still carries the old String.raw steer"
            );
        }
    }

    /// The guard on the guard: [`unwrapped`] must join across a line break
    /// rather than merely trimming, or the assertion above quietly goes back to
    /// being a raw `contains` that only one platform's line-wrapping satisfies.
    ///
    /// The middle case is the one with teeth: it is the exact shape the
    /// windows description has, and a `trim()`-only or `replace('\n', "")`
    /// implementation fails it (the first leaves the newline, the second leaves
    /// the twelve-space indent behind).
    #[test]
    fn unwrapped_joins_a_phrase_that_a_source_literal_wrapped() {
        assert_eq!(
            unwrapped("terminates the template literal"),
            "terminates the template literal"
        );
        assert_eq!(
            unwrapped("terminates the\n            template literal early"),
            "terminates the template literal early"
        );
        assert_eq!(unwrapped("\n   a\t\t b \n"), "a b");
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn automation_script_nonzero_exit_is_a_tool_error() {
        let server = ComputerControllerServer::new();
        let error = server
            .automation_script(Parameters(AutomationScriptParams {
                language: ScriptLanguage::Shell,
                script: "printf 'search failed' >&2\nexit 7".to_string(),
                save_output: false,
            }))
            .await
            .expect_err("a nonzero script must not be reported as a successful tool call");

        let message = error.to_string();
        assert!(message.contains("Script failed"));
        assert!(message.contains("search failed"));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod computer_control_tests {
    use super::*;
    use rmcp::model::RawContent;

    fn text_of(result: &rmcp::model::CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn computer_control_reports_failure_instead_of_fake_success() {
        let server = ComputerControllerServer::new();
        let result = server
            .computer_control(Parameters(ComputerControlParams {
                script: "this is not valid applescript @@@".to_string(),
                save_output: false,
            }))
            .await;
        // The script genuinely fails; the tool must return an error (Err here,
        // which the MCP layer turns into is_error=true) rather than the old
        // "Script completed successfully" with empty output.
        assert!(
            result.is_err(),
            "a failing control script must surface as an error, got: {:?}",
            result.ok().map(|r| text_of(&r))
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Computer control script failed"),
            "error should be explicit about the failure, got: {msg}"
        );
    }

    #[tokio::test]
    async fn computer_control_no_output_success_guides_to_verify() {
        let server = ComputerControllerServer::new();
        let result = server
            .computer_control(Parameters(ComputerControlParams {
                script: "set _x to 1\nreturn".to_string(),
                save_output: false,
            }))
            .await
            .expect("a valid no-output script should succeed");
        let text = text_of(&result);
        assert!(
            text.contains("no output") && text.contains("screenshot"),
            "no-output success should tell the model to verify rather than repeat, got: {text}"
        );
    }
}
