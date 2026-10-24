use anstream::println;
use bat::WrappingMode;
use biorouter::config::Config;
use biorouter::conversation::message::{
    ActionRequiredData, Message, MessageContent, ToolRequest, ToolResponse,
};
use biorouter::utils::{safe_truncate, sanitize_untrusted_label};
use console::{measure_text_width, style, Color, Term};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rmcp::model::{CallToolRequestParams, JsonObject, PromptArgument, ResourceContents};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{Error, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Biorouter warm tan-brown accent (xterm-256 137 ≈ #af875f), Biorouter's light cream palette
/// the closest 256-color match to the desktop brand coral `#cf6d47`).
///
/// Per the Biorouter design language the accent is used *sparingly* — the input
/// prompt, section rules, and active indicators — never for general decoration.
pub const ACCENT: Color = Color::Color256(137);

/// Width of the terminal in columns, falling back to 80 when it can't be probed.
fn term_width() -> usize {
    Term::stdout()
        .size_checked()
        .map(|(_h, w)| w as usize)
        .unwrap_or(80)
}

/// Format a token count compactly for status lines: `1234 → "1.2k"`, `2_000_000 → "2.0M"`.
fn human_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Print a left-aligned section rule that fills the terminal width:
///
/// ```text
/// ── name  namespace ──────────────────────────────
/// ```
///
/// `name` is rendered bold and `namespace` (when present) dim. The leading
/// `── ` ticks carry the brand accent. Width is clamped so it stays tidy on
/// both narrow and ultra-wide terminals.
fn print_section_rule(name: &str, namespace: &str) {
    let width = term_width().clamp(24, 100);
    let prefix = "── ";
    // Track the visible (un-styled) width so the trailing dashes land flush.
    let mut visible = measure_text_width(prefix) + measure_text_width(name) + 1;
    let ns_segment = if namespace.is_empty() {
        String::new()
    } else {
        visible += measure_text_width(namespace) + 1;
        format!("{} ", style(namespace).dim())
    };
    let fill = width.saturating_sub(visible).max(3);
    println!();
    println!(
        "{}{} {}{}",
        style(prefix).fg(ACCENT),
        style(name).bold(),
        ns_segment,
        style("─".repeat(fill)).dim(),
    );
}

// Re-export theme for use in main
#[derive(Clone, Copy)]
pub enum Theme {
    Light,
    Dark,
    Ansi,
}

impl Theme {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Theme::Light => "GitHub",
            Theme::Dark => "zenburn",
            Theme::Ansi => "base16",
        }
    }

    fn from_config_str(val: &str) -> Self {
        if val.eq_ignore_ascii_case("light") {
            Theme::Light
        } else if val.eq_ignore_ascii_case("ansi") {
            Theme::Ansi
        } else {
            Theme::Dark
        }
    }

    fn as_config_string(&self) -> String {
        match self {
            Theme::Light => "light".to_string(),
            Theme::Dark => "dark".to_string(),
            Theme::Ansi => "ansi".to_string(),
        }
    }
}

thread_local! {
    static CURRENT_THEME: RefCell<Theme> = RefCell::new(
        std::env::var("BIOROUTER_CLI_THEME").ok()
            .map(|val| Theme::from_config_str(&val))
            .unwrap_or_else(||
                Config::global().get_param::<String>("BIOROUTER_CLI_THEME").ok()
                    .map(|val| Theme::from_config_str(&val))
                    .unwrap_or(Theme::Ansi)
            )
    );
    static SHOW_FULL_TOOL_OUTPUT: RefCell<bool> = const { RefCell::new(false) };
}

pub fn set_theme(theme: Theme) {
    let config = Config::global();
    config
        .set_param("BIOROUTER_CLI_THEME", theme.as_config_string())
        .expect("Failed to set theme");
    CURRENT_THEME.with(|t| *t.borrow_mut() = theme);

    let config = Config::global();
    let theme_str = match theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::Ansi => "ansi",
    };

    if let Err(e) = config.set_param("BIOROUTER_CLI_THEME", theme_str) {
        eprintln!("Failed to save theme setting to config: {}", e);
    }
}

pub fn get_theme() -> Theme {
    CURRENT_THEME.with(|t| *t.borrow())
}

pub fn toggle_full_tool_output() -> bool {
    SHOW_FULL_TOOL_OUTPUT.with(|s| {
        let mut val = s.borrow_mut();
        *val = !*val;
        *val
    })
}

pub fn get_show_full_tool_output() -> bool {
    SHOW_FULL_TOOL_OUTPUT.with(|s| *s.borrow())
}

// Simple wrapper around spinner to manage its state
#[derive(Default)]
pub struct ThinkingIndicator {
    spinner: Option<cliclack::ProgressBar>,
}

impl ThinkingIndicator {
    pub fn show(&mut self) {
        let spinner = cliclack::spinner();
        if Config::global()
            .get_param("RANDOM_THINKING_MESSAGES")
            .unwrap_or(true)
        {
            spinner.start(format!(
                "{}...",
                super::thinking::get_random_thinking_message()
            ));
        } else {
            spinner.start("Thinking...");
        }
        self.spinner = Some(spinner);
    }

    pub fn hide(&mut self) {
        if let Some(spinner) = self.spinner.take() {
            spinner.stop("");
        }
    }

    pub fn is_shown(&self) -> bool {
        self.spinner.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct PromptInfo {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Option<Vec<PromptArgument>>,
    pub extension: Option<String>,
}

// Global thinking indicator
thread_local! {
    static THINKING: RefCell<ThinkingIndicator> = RefCell::new(ThinkingIndicator::default());
}

pub fn show_thinking() {
    if std::io::stdout().is_terminal() {
        THINKING.with(|t| t.borrow_mut().show());
    }
}

pub fn hide_thinking() {
    if std::io::stdout().is_terminal() {
        THINKING.with(|t| t.borrow_mut().hide());
    }
}

pub fn is_showing_thinking() -> bool {
    THINKING.with(|t| t.borrow().is_shown())
}

pub fn set_thinking_message(s: &String) {
    if std::io::stdout().is_terminal() {
        THINKING.with(|t| {
            if let Some(spinner) = t.borrow_mut().spinner.as_mut() {
                spinner.set_message(s);
            }
        });
    }
}

pub fn render_message(message: &Message, debug: bool) {
    let theme = get_theme();

    for content in &message.content {
        match content {
            MessageContent::ActionRequired(action) => match &action.data {
                ActionRequiredData::ToolConfirmation { tool_name, .. } => {
                    println!("action_required(tool_confirmation): {}", tool_name)
                }
                ActionRequiredData::Elicitation { message, .. } => {
                    println!("action_required(elicitation): {}", message)
                }
                ActionRequiredData::ElicitationResponse { id, .. } => {
                    println!("action_required(elicitation_response): {}", id)
                }
                // #107: key names only. A credential card carries no value —
                // the trusted surface writes them straight to the keyring — and
                // this line reaches a terminal that may be piped or logged.
                ActionRequiredData::SecretRequest { prompt, keys, .. } => {
                    println!(
                        "action_required(secret_request): {prompt} [{}]",
                        keys.iter()
                            .map(|k| k.key.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            },
            MessageContent::Text(text) => print_markdown(&text.text, theme),
            MessageContent::ToolRequest(req) => render_tool_request(req, theme, debug),
            MessageContent::ToolResponse(resp) => render_tool_response(resp, theme, debug),
            MessageContent::Image(image) => {
                // Show a compact placeholder, not the full base64 blob (which can
                // be megabytes and floods the terminal). `data` is base64, so the
                // decoded size is ~3/4 of its length.
                let approx_bytes = image.data.len() * 3 / 4;
                println!(
                    "{}",
                    style(format!(
                        "[Image: {}, ~{} bytes]",
                        image.mime_type, approx_bytes
                    ))
                    .dim()
                );
            }
            MessageContent::Thinking(thinking) => {
                if std::env::var("BIOROUTER_CLI_SHOW_THINKING").is_ok()
                    && std::io::stdout().is_terminal()
                {
                    println!("\n{}", style("Thinking:").dim().italic());
                    print_markdown(&thinking.thinking, theme);
                }
            }
            MessageContent::RedactedThinking(_) => {
                // For redacted thinking, print thinking was redacted
                println!("\n{}", style("Thinking:").dim().italic());
                print_markdown("Thinking was redacted", theme);
            }
            MessageContent::SystemNotification(notification) => {
                use biorouter::conversation::message::SystemNotificationType;

                match notification.notification_type {
                    SystemNotificationType::ThinkingMessage => {
                        show_thinking();
                        set_thinking_message(&notification.msg);
                    }
                    SystemNotificationType::InlineMessage => {
                        println!("\n{}", style(&notification.msg).yellow());
                    }
                }
            }
            _ => {
                println!("WARNING: Message content type could not be rendered");
            }
        }
    }

    let _ = std::io::stdout().flush();
}

pub fn render_text(text: &str, color: Option<Color>, dim: bool) {
    render_text_no_newlines(format!("\n{}\n\n", text).as_str(), color, dim);
}

pub fn render_text_no_newlines(text: &str, color: Option<Color>, dim: bool) {
    if !std::io::stdout().is_terminal() {
        println!("{}", text);
        return;
    }
    let mut styled_text = style(text);
    if dim {
        styled_text = styled_text.dim();
    }
    if let Some(color) = color {
        styled_text = styled_text.fg(color);
    }
    print!("{}", styled_text);
}

pub fn render_enter_plan_mode() {
    println!(
        "\n{} {}\n",
        style("Entering plan mode.").bold(),
        style("You can provide instructions to create a plan and then act on it. To exit early, type /endplan")

            .dim()
    );
}

pub fn render_act_on_plan() {
    println!(
        "\n{}\n",
        style("Exiting plan mode and acting on the above plan").bold(),
    );
}

pub fn render_exit_plan_mode() {
    println!("\n{}\n", style("Exiting plan mode.").bold());
}

pub fn biorouter_mode_message(text: &str) {
    println!("\n{}", style(text).yellow(),);
}

fn render_tool_request(req: &ToolRequest, theme: Theme, debug: bool) {
    match &req.tool_call {
        Ok(call) => match call.name.to_string().as_str() {
            "developer__text_editor" => render_text_editor_request(call, debug),
            "developer__shell" => render_shell_request(call, debug),
            "code_execution__execute_code" => render_execute_code_request(call, debug),
            // BR-71: the spawn tool is advertised by the workspace extension
            // now, so the name that reaches the transcript is normally the
            // PREFIXED one. The bare form still arrives from prefix-stripping
            // models (dispatch tolerates both — `is_spawn_tool_call`), so both
            // must render as a delegation rather than falling through to the
            // generic "tool with arguments" card.
            "subagent" | "workspace__subagent" => render_subagent_request(call, debug),
            "todo__write" => render_todo_request(call, debug),
            _ => render_default_request(call, debug),
        },
        Err(e) => print_markdown_source(&e.to_string(), theme),
    }
}

/// Default for `BIOROUTER_CLI_MIN_PRIORITY`, the floor a tool result's
/// annotated priority must reach to be printed.
///
/// ⚠ 0.0, and it has to be, because higher means quieter and nothing in this
/// repo emits much. The only production values are 0.2 (`text_editor`) and 0.0
/// (`developer`, `analyze`), so any floor above 0.2 silences every tool result
/// there is and the classic renderer prints nothing at all.
///
/// It read 0.5 from the initial commit until it was measured, which is exactly
/// that: a renderer that showed no tool output in its default configuration.
/// Both `docs/configuration/environment-variables.md` and
/// `docs/configuration/config-file-reference.md` documented 0.0 the whole time,
/// so the code was the side that disagreed with everything else.
/// `the_documented_default_is_the_one_the_code_uses` keeps the three in step.
const DEFAULT_CLI_MIN_PRIORITY: f32 = 0.0;

/// Whether a tool-result content item is printed.
///
/// Pulled out of [`render_tool_response`] so the rule can be tested. The
/// renderer writes through `bat` to the real stdout and returns nothing, so
/// while the decision lived inline there was no way to assert it, which is how
/// a floor that hid every tool result went unnoticed.
///
/// Two clauses, and they answer different questions. An annotated priority
/// below the floor is the user asking for less. A MISSING priority is not a
/// quiet result, it is an unannotated one, and those stay hidden outside debug
/// because that is where raw protocol chatter arrives.
fn content_clears_priority_gate(priority: Option<f32>, min_priority: f32, debug: bool) -> bool {
    match priority {
        Some(priority) => priority >= min_priority,
        None => debug,
    }
}

fn render_tool_response(resp: &ToolResponse, theme: Theme, debug: bool) {
    let config = Config::global();

    match &resp.tool_result {
        Ok(result) => {
            for content in &result.content {
                // A terminal can't render inline artifact HTML, so surface a
                // browser-safe target before the normal text priority filter.
                if let Some(note) = artifact_note_from_content(content) {
                    render_artifact_note(&note);
                    continue;
                }

                if let Some(audience) = content.audience() {
                    if !audience.contains(&rmcp::model::Role::User) {
                        continue;
                    }
                }

                let min_priority = config
                    .get_param::<f32>("BIOROUTER_CLI_MIN_PRIORITY")
                    .ok()
                    .unwrap_or(DEFAULT_CLI_MIN_PRIORITY);

                if !content_clears_priority_gate(content.priority(), min_priority, debug) {
                    continue;
                }

                if debug {
                    println!("{:#?}", content);
                } else if let Some(text) = content.as_text() {
                    // Every successful tool result reaches here wrapped in the
                    // guardrail's `<tool-output untrusted="true" …>` frame. That
                    // frame is addressed to the model, and printing it at a
                    // terminal shows the user machinery instead of their answer.
                    //
                    // ⚠ Display only. The frame is the prompt-injection control,
                    // so it must still be intact everywhere the content heads for
                    // a model. The `{:#?}` branch above is a structural dump for
                    // debugging and is deliberately left raw.
                    //
                    // A `[BIOROUTER GUARDRAIL]` warning sits ABOVE the opening
                    // tag and survives this, which is the point: the frame is
                    // provenance and routine, the warning is a real finding the
                    // user should see.
                    print_markdown_source(
                        &biorouter::guardrails::tool_output_display::unframe_tool_output(
                            &text.text,
                        ),
                        theme,
                    );
                }
            }
            for note in app_launch_notes(result) {
                render_artifact_note(&note);
            }
        }
        Err(e) => print_markdown_source(&e.to_string(), theme),
    }
}

/// A user-facing summary of an artifact resource or browser link emitted by a tool.
pub struct ArtifactNote {
    /// Human title derived from the `ui://` URI (e.g. "Volcano Plot").
    pub title: String,
    /// Where a standalone HTML copy was saved, if the artifact was HTML.
    pub saved_path: Option<std::path::PathBuf>,
    /// A browser-safe URL the terminal can expose as a clickable target.
    pub browser_url: Option<String>,
}

/// Extract an [`ArtifactNote`] from a `ui://` resource or browser-safe resource
/// link. Shared by the classic renderer and TUI so both surface artifacts
/// identically.
pub fn artifact_note_from_content(content: &rmcp::model::Content) -> Option<ArtifactNote> {
    use rmcp::model::RawContent;

    if content
        .audience()
        .is_some_and(|audience| !audience.contains(&rmcp::model::Role::User))
    {
        return None;
    }

    if let RawContent::ResourceLink(link) = &content.raw {
        let browser_url = external_browser_url(&link.uri)?;
        let title = link
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| link.name.clone());
        return Some(ArtifactNote {
            title: sanitize_artifact_title(&title),
            saved_path: None,
            browser_url: Some(browser_url),
        });
    }

    let resource = content.as_resource()?;
    let linked_url = uri_list_browser_url(&resource.resource);
    let (uri, html) = match &resource.resource {
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } => {
            let html = if mime_type.as_deref().is_some_and(is_html_mime)
                && blob.len() <= MAX_ENCODED_ARTIFACT_BYTES
            {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(blob)
                    .ok()
                    .filter(|bytes| bytes.len() <= MAX_ARTIFACT_HTML_BYTES)
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            } else {
                None
            };
            (uri.clone(), html)
        }
        ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            ..
        } => {
            let html = (mime_type.as_deref().is_some_and(is_html_mime)
                && text.len() <= MAX_ARTIFACT_HTML_BYTES)
                .then(|| text.clone());
            (uri.clone(), html)
        }
    };

    if let Some(browser_url) = external_browser_url(&uri) {
        let title = sanitize_artifact_title(&title_from_external_url(&browser_url));
        return Some(ArtifactNote {
            title,
            saved_path: None,
            browser_url: Some(browser_url),
        });
    }
    if !uri.starts_with("ui://") {
        return None;
    }
    if uri.len() > MAX_BROWSER_URL_BYTES {
        return None;
    }

    let title =
        sanitize_artifact_title(&title_from_ui_uri(&uri).unwrap_or_else(|| "Artifact".to_string()));
    let saved_path = html.and_then(|h| save_artifact_html(&uri, &h));
    let browser_url = saved_path
        .as_ref()
        .and_then(|path| url::Url::from_file_path(path).ok())
        .map(|url| url.to_string());
    Some(ArtifactNote {
        title,
        saved_path,
        browser_url: linked_url.or(browser_url),
    })
}

const MAX_ARTIFACT_HTML_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENCODED_ARTIFACT_BYTES: usize = (MAX_ARTIFACT_HTML_BYTES * 4 / 3) + 4;
const MAX_BROWSER_URL_BYTES: usize = 8 * 1024;
const MAX_URI_LIST_BYTES: usize = 64 * 1024;
const MAX_ENCODED_URI_LIST_BYTES: usize = (MAX_URI_LIST_BYTES * 4 / 3) + 4;
const MAX_APP_LAUNCH_NOTES: usize = 64;
const MAX_ARTIFACT_TITLE_CHARS: usize = 256;

fn is_html_mime(mime_type: &str) -> bool {
    matches!(
        mime_type.split(';').next().map(str::trim),
        Some(value)
            if value.eq_ignore_ascii_case("text/html")
                || value.eq_ignore_ascii_case("application/xhtml+xml")
    )
}

fn is_uri_list_mime(mime_type: &str) -> bool {
    mime_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/uri-list"))
}

fn uri_list_browser_url(resource: &ResourceContents) -> Option<String> {
    let text = match resource {
        ResourceContents::TextResourceContents {
            mime_type, text, ..
        } if mime_type.as_deref().is_some_and(is_uri_list_mime)
            && text.len() <= MAX_URI_LIST_BYTES =>
        {
            text.clone()
        }
        ResourceContents::BlobResourceContents {
            mime_type, blob, ..
        } if mime_type.as_deref().is_some_and(is_uri_list_mime)
            && blob.len() <= MAX_ENCODED_URI_LIST_BYTES =>
        {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(blob)
                .ok()?;
            if bytes.len() > MAX_URI_LIST_BYTES {
                return None;
            }
            String::from_utf8(bytes).ok()?
        }
        _ => return None,
    };
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(url) = external_browser_url(line) {
            return Some(url);
        }
    }
    None
}

fn external_browser_url(uri: &str) -> Option<String> {
    if uri.len() > MAX_BROWSER_URL_BYTES {
        return None;
    }
    let url = url::Url::parse(uri).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

/// A terminal-safe artifact title.
///
/// The drop set itself lives in [`sanitize_untrusted_label`] — the workspace's
/// single definition — because the preview panel needs the identical treatment
/// for the titles a *website* chooses, and two copies of a filter like this
/// drift until only one of them still covers the class that matters.
fn sanitize_artifact_title(title: &str) -> String {
    let sanitized = sanitize_untrusted_label(title, MAX_ARTIFACT_TITLE_CHARS);
    if sanitized.is_empty() {
        "Artifact".to_string()
    } else {
        sanitized
    }
}

fn title_from_external_url(uri: &str) -> String {
    url::Url::parse(uri)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .rfind(|segment| !segment.is_empty())
                .map(str::to_string)
                .or_else(|| url.host_str().map(str::to_string))
        })
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Artifact link".to_string())
}

/// Extract live Agent Drafter launch targets from result metadata. A normal
/// `launch_app` result uses one path; `execute_code` can aggregate several.
pub fn app_launch_notes(result: &rmcp::model::CallToolResult) -> Vec<ArtifactNote> {
    let Some(meta) = &result.meta else {
        return Vec::new();
    };
    let mut paths = meta
        .0
        .get("biorouter/app-paths")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .take(MAX_APP_LAUNCH_NOTES)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(path) = meta
        .0
        .get("biorouter/app-path")
        .and_then(serde_json::Value::as_str)
    {
        paths.push(path.to_string());
    }
    paths.sort();
    paths.dedup();
    paths.truncate(MAX_APP_LAUNCH_NOTES);
    let linked_urls = result
        .content
        .iter()
        .filter(|content| {
            content
                .audience()
                .is_none_or(|audience| audience.contains(&rmcp::model::Role::User))
        })
        .filter_map(|content| match &content.raw {
            rmcp::model::RawContent::ResourceLink(link) => external_browser_url(&link.uri),
            _ => None,
        })
        .collect::<HashSet<_>>();

    paths
        .into_iter()
        .filter_map(|path| {
            let id = path.strip_prefix("/apps/")?.strip_suffix('/')?;
            biorouter_mcp::agent_drafter::store::validate_artifact_id(id).ok()?;
            let browser_url = app_browser_url(&path);
            if linked_urls.contains(&browser_url) {
                return None;
            }
            Some(ArtifactNote {
                title: format!("App: {id}"),
                saved_path: None,
                browser_url: Some(browser_url),
            })
        })
        .collect()
}

fn app_browser_url(path: &str) -> String {
    if let Some(base) = std::env::var("BIOROUTER_APP_BASE_URL")
        .ok()
        .and_then(|base| url::Url::parse(base.trim()).ok())
        .filter(|base| {
            matches!(base.scheme(), "http" | "https")
                && base.host_str().is_some()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none()
        })
    {
        return format!("{}{}", base.as_str().trim_end_matches('/'), path);
    }
    let port = std::env::var("BIOROUTER_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3000);
    format!("http://127.0.0.1:{port}{path}")
}

/// Derive a human title from a `ui://host/path` URI, mirroring the desktop's
/// `titleFromResourceUri`: title-case the path segments (e.g. `ui://volcano/plot`
/// → "Volcano Plot", `ui://dashboard/omics-summary` → "Dashboard Omics Summary").
fn title_from_ui_uri(uri: &str) -> Option<String> {
    if uri.len() > MAX_BROWSER_URL_BYTES {
        return None;
    }
    let rest = uri.strip_prefix("ui://")?;
    // Drop any query string / fragment before splitting the path.
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let parts: Vec<String> = rest
        .split('/')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.replace(['-', '_'], " "))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let title = parts
        .iter()
        .flat_map(|p| p.split_whitespace())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (!title.is_empty()).then_some(title)
}

/// Save a standalone HTML artifact to a temp dir so a terminal user can open it
/// in a browser. Best-effort: returns None on any IO error.
fn save_artifact_html(_uri: &str, html: &str) -> Option<std::path::PathBuf> {
    use std::io::Write;

    if html.len() > MAX_ARTIFACT_HTML_BYTES {
        return None;
    }
    let mut file = tempfile::Builder::new()
        .prefix("biorouter-artifact-")
        .suffix(".html")
        .tempfile()
        .ok()?;
    let html = wrap_artifact_for_browser(html);
    file.write_all(html.as_bytes()).ok()?;
    let (_file, path) = file.keep().ok()?;
    Some(path)
}

fn inject_artifact_browser_csp(html: &str) -> String {
    const META: &str = concat!(
        r#"<meta http-equiv="Content-Security-Policy" content="#,
        "default-src 'none'; ",
        "script-src 'unsafe-inline' 'unsafe-eval' blob:; ",
        "style-src 'unsafe-inline'; ",
        "img-src data: blob:; ",
        "connect-src 'none'; ",
        "font-src data:; frame-src 'none'; worker-src blob:; ",
        "media-src data: blob:; navigate-to 'none'; ",
        "form-action 'none'; base-uri 'none'; object-src 'none'",
        r#"">"#
    );

    let lower = html.to_ascii_lowercase();
    if let Some(html_start) = find_start_tag(&lower, "html") {
        let Some(prefix) = lower.get(..html_start) else {
            return format!("<head>{META}</head>{html}");
        };
        if !is_document_preamble(prefix) {
            return format!("<head>{META}</head>{html}");
        }
        if let Some(relative_end) = lower.get(html_start..).and_then(|tail| tail.find('>')) {
            let insert_at = html_start + relative_end + 1;
            let Some(remainder) = lower.get(insert_at..) else {
                return format!("<head>{META}</head>{html}");
            };
            let whitespace = remainder.len() - remainder.trim_start().len();
            let head_start = insert_at + whitespace;
            if lower
                .get(head_start..)
                .is_some_and(|tail| find_start_tag(tail, "head") == Some(0))
            {
                if let Some(relative_end) = lower.get(head_start..).and_then(|tail| tail.find('>'))
                {
                    let head_end = head_start + relative_end + 1;
                    if let Some(secured) = insert_html_at(html, head_end, META) {
                        return secured;
                    }
                }
            }
            if let Some(secured) = insert_html_at(html, insert_at, &format!("<head>{META}</head>"))
            {
                return secured;
            }
        }
    }
    if let Some(head_start) = find_start_tag(&lower, "head") {
        if lower
            .get(..head_start)
            .is_some_and(|prefix| prefix.trim().is_empty())
        {
            if let Some(relative_end) = lower.get(head_start..).and_then(|tail| tail.find('>')) {
                let insert_at = head_start + relative_end + 1;
                if let Some(secured) = insert_html_at(html, insert_at, META) {
                    return secured;
                }
            }
        }
    }
    format!("<head>{META}</head>{html}")
}

fn insert_html_at(html: &str, index: usize, fragment: &str) -> Option<String> {
    let prefix = html.get(..index)?;
    let suffix = html.get(index..)?;
    let mut result = String::with_capacity(html.len() + fragment.len());
    result.push_str(prefix);
    result.push_str(fragment);
    result.push_str(suffix);
    Some(result)
}

fn wrap_artifact_for_browser(html: &str) -> String {
    let secured = inject_artifact_browser_csp(html);
    let srcdoc = secured
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\0', "\u{fffd}");
    format!(
        concat!(
            "<!doctype html><html><head><meta charset=\"utf-8\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"",
            "default-src 'none'; style-src 'unsafe-inline'; frame-src 'self'\">",
            "<style>html,body,iframe{{width:100%;height:100%;margin:0;border:0;overflow:hidden}}",
            "body{{background:#fff}}</style></head><body>",
            "<iframe name=\"biorouter-artifact-preview\" title=\"Biorouter artifact preview\" ",
            "credentialless referrerpolicy=\"no-referrer\" ",
            "sandbox=\"allow-scripts allow-downloads\" ",
            "srcdoc=\"{}\"></iframe></body></html>"
        ),
        srcdoc
    )
}

fn is_document_preamble(prefix: &str) -> bool {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return true;
    }
    prefix
        .strip_prefix("<!doctype")
        .and_then(|doctype| doctype.find('>').and_then(|end| doctype.get(end + 1..)))
        .is_some_and(|remainder| remainder.trim().is_empty())
}

fn find_start_tag(html: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{tag}");
    html.match_indices(&needle).find_map(|(offset, _)| {
        html.as_bytes()
            .get(offset + needle.len())
            .is_some_and(|byte| *byte == b'>' || byte.is_ascii_whitespace())
            .then_some(offset)
    })
}

/// Print the titled artifact signal (classic path).
fn render_artifact_note(note: &ArtifactNote) {
    println!(
        "\n  {} {}",
        style("◆").fg(ACCENT),
        style(format!("Artifact: {}", note.title)).bold()
    );
    match &note.browser_url {
        Some(url) => println!(
            "    {} {}",
            style("open in a browser:").dim(),
            style(url).fg(ACCENT)
        ),
        None => println!(
            "    {}",
            style("(no browser-safe preview URL was provided)").dim()
        ),
    }
}

pub fn render_error(message: &str) {
    // stderr, not stdout: error prose printed to stdout corrupts
    // `--output-format json`, whose document is the ONLY thing stdout may
    // carry (#31/#41 — the reporter's JSON file contained this prose).
    eprintln!("\n  {} {}\n", style("error:").red().bold(), message);
}

pub fn render_prompts(prompts: &HashMap<String, Vec<String>>) {
    println!();
    for (extension, prompts) in prompts {
        println!(" {}", style(extension));
        for prompt in prompts {
            println!("  - {}", style(prompt).cyan());
        }
    }
    println!();
}

pub fn render_prompt_info(info: &PromptInfo) {
    println!();
    if let Some(ext) = &info.extension {
        println!(" {}: {}", style("Extension"), ext);
    }
    println!(" Prompt: {}", style(&info.name).cyan().bold());
    if let Some(desc) = &info.description {
        println!("\n {}", desc);
    }
    render_arguments(info);
    println!();
}

fn render_arguments(info: &PromptInfo) {
    if let Some(args) = &info.arguments {
        println!("\n Arguments:");
        for arg in args {
            let required = arg.required.unwrap_or(false);
            let req_str = if required {
                style("(required)").red()
            } else {
                style("(optional)").dim()
            };

            println!(
                "  {} {} {}",
                style(&arg.name).yellow(),
                req_str,
                arg.description.as_deref().unwrap_or("")
            );
        }
    }
}

pub fn render_extension_success(name: &str) {
    println!();
    println!("  {} extension `{}`", style("added"), style(name).cyan(),);
    println!();
}

pub fn render_extension_error(name: &str, error: &str) {
    println!();
    println!(
        "  {} to add extension {}",
        style("failed").red(),
        style(name).red()
    );
    println!();
    println!("{}", style(error).dim());
    println!();
}

pub fn render_diverge_success(new_session_id: &str) {
    println!();
    println!(
        "  {} chat into a new window (session ID {})",
        style("diverged"),
        style(new_session_id).cyan()
    );
    println!(
        "  {}",
        style("the original chat is unchanged, keep chatting here").dim()
    );
    println!();
}

pub fn render_diverge_open_failed(new_session_id: &str, url: &str, error: &str) {
    println!();
    println!(
        "  {} the diverged chat (session ID {}) but couldn't open a window",
        style("created").yellow(),
        style(new_session_id).cyan()
    );
    println!("  {} {}", style("reason:").dim(), style(error).dim());
    println!(
        "  {} open Biorouter and run this link manually:",
        style("tip:").dim()
    );
    println!("  {}", style(url).cyan());
    println!();
}

pub fn render_builtin_success(names: &str) {
    println!();
    println!(
        "  {} builtin{}: {}",
        style("added"),
        if names.contains(',') { "s" } else { "" },
        style(names).cyan()
    );
    println!();
}

pub fn render_builtin_error(names: &str, error: &str) {
    println!();
    println!(
        "  {} to add builtin{}: {}",
        style("failed").red(),
        if names.contains(',') { "s" } else { "" },
        style(names).red()
    );
    println!();
    println!("{}", style(error).dim());
    println!();
}

fn render_text_editor_request(call: &CallToolRequestParams, debug: bool) {
    print_tool_header(call);

    // Print path first with special formatting
    if let Some(args) = &call.arguments {
        if let Some(Value::String(path)) = args.get("path") {
            println!(
                "{}: {}",
                style("path").dim(),
                style(shorten_path(path, debug))
            );
        }

        // Print other arguments normally, excluding path
        if let Some(args) = &call.arguments {
            let mut other_args = serde_json::Map::new();
            for (k, v) in args {
                if k != "path" {
                    other_args.insert(k.clone(), v.clone());
                }
            }
            if !other_args.is_empty() {
                print_params(&Some(other_args), 0, debug);
            }
        }
    }
    println!();
}

fn render_shell_request(call: &CallToolRequestParams, debug: bool) {
    print_tool_header(call);
    print_params(&call.arguments, 0, debug);
    println!();
}

fn render_execute_code_request(call: &CallToolRequestParams, debug: bool) {
    let tool_graph = call
        .arguments
        .as_ref()
        .and_then(|args| args.get("tool_graph"))
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty());

    let Some(tool_graph) = tool_graph else {
        return render_default_request(call, debug);
    };

    let count = tool_graph.len();
    let plural = if count == 1 { "" } else { "s" };
    print_section_rule(&format!("{} tool call{}", count, plural), "execute_code");

    for (i, node) in tool_graph.iter().filter_map(Value::as_object).enumerate() {
        let tool = node
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let desc = node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let deps: Vec<_> = node
            .get("depends_on")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|d| (d + 1).to_string())
            .collect();
        let deps_str = if deps.is_empty() {
            String::new()
        } else {
            format!(" (uses {})", deps.join(", "))
        };
        println!(
            "  {}. {}: {}{}",
            style(i + 1).dim(),
            style(tool).cyan(),
            style(desc),
            style(deps_str).dim()
        );
    }
    println!();
}

fn render_subagent_request(call: &CallToolRequestParams, debug: bool) {
    print_tool_header(call);

    if let Some(args) = &call.arguments {
        if let Some(Value::String(subworkflow)) = args.get("subworkflow") {
            println!(
                "{}: {}",
                style("subworkflow").dim(),
                style(subworkflow).cyan()
            );
        }

        if let Some(Value::String(instructions)) = args.get("instructions") {
            let display = if instructions.len() > 100 && !debug {
                safe_truncate(instructions, 100)
            } else {
                instructions.clone()
            };
            println!("{}: {}", style("instructions").dim(), style(display));
        }

        if let Some(Value::Object(params)) = args.get("parameters") {
            println!("{}:", style("parameters").dim());
            print_params(&Some(params.clone()), 1, debug);
        }

        let skip_keys = ["subworkflow", "instructions", "parameters"];
        let mut other_args = serde_json::Map::new();
        for (k, v) in args {
            if !skip_keys.contains(&k.as_str()) {
                other_args.insert(k.clone(), v.clone());
            }
        }
        if !other_args.is_empty() {
            print_params(&Some(other_args), 0, debug);
        }
    }

    println!();
}

fn render_todo_request(call: &CallToolRequestParams, _debug: bool) {
    print_tool_header(call);

    if let Some(args) = &call.arguments {
        if let Some(Value::String(content)) = args.get("content") {
            println!("{}: {}", style("content").dim(), style(content));
        }
    }
    println!();
}

fn render_default_request(call: &CallToolRequestParams, debug: bool) {
    print_tool_header(call);
    print_params(&call.arguments, 0, debug);
    println!();
}

// Helper functions

fn print_tool_header(call: &CallToolRequestParams) {
    let parts: Vec<_> = call.name.rsplit("__").collect();
    let name = parts.first().copied().unwrap_or("unknown");
    let namespace = parts
        .split_first()
        .map(|(_, s)| s.iter().rev().copied().collect::<Vec<_>>().join("__"))
        .unwrap_or_default();
    // A clear, accented "tool call" badge so it's unmistakable that the model is
    // invoking a tool — simple but distinctly indicative.
    println!();
    let ns = if namespace.is_empty() {
        String::new()
    } else {
        format!("  {}", style(format!("· {}", namespace)).dim())
    };
    println!(
        "{} {}{}",
        style(" ▸ tool call ").black().on_color256(137).bold(),
        style(name).fg(ACCENT).bold(),
        ns,
    );
}

// Respect NO_COLOR, as https://crates.io/crates/console already does
pub fn env_no_color() -> bool {
    // if NO_COLOR is defined at all disable colors
    std::env::var_os("NO_COLOR").is_none()
}

/// Render assistant markdown into styled terminal output: headings, bold,
/// tables, blockquotes, lists, links, and syntax-highlighted code blocks
/// (see `session::markdown`). Falls back to raw text when not on a TTY.
fn print_markdown(content: &str, theme: Theme) {
    if std::io::stdout().is_terminal() {
        let width = term_width().clamp(40, 100);
        print!(
            "{}",
            super::markdown::render_markdown(content, theme, width)
        );
    } else {
        print!("{}", content);
    }
}

/// Print text as syntax-highlighted *markdown source* via bat. Used for tool
/// output, which is data — re-flowing or re-styling it the way assistant
/// prose is rendered could distort file contents, paths, or logs.
fn print_markdown_source(content: &str, theme: Theme) {
    if std::io::stdout().is_terminal() {
        bat::PrettyPrinter::new()
            .input(bat::Input::from_bytes(content.as_bytes()))
            .theme(theme.as_str())
            .colored_output(env_no_color())
            .language("Markdown")
            .wrapping_mode(WrappingMode::NoWrapping(true))
            .print()
            .unwrap();
    } else {
        print!("{}", content);
    }
}

const INDENT: &str = "    ";

fn print_value_with_prefix(prefix: &String, value: &Value, debug: bool) {
    let prefix_width = measure_text_width(prefix.as_str());
    print!("{}", prefix);
    print_value(value, debug, prefix_width)
}

fn print_value(value: &Value, debug: bool, reserve_width: usize) {
    let max_width = Term::stdout()
        .size_checked()
        .map(|(_h, w)| (w as usize).saturating_sub(reserve_width));
    let show_full = get_show_full_tool_output();
    let formatted = match value {
        Value::String(s) => match (max_width, debug || show_full) {
            (Some(w), false) if s.len() > w => style(safe_truncate(s, w)),
            _ => style(s.to_string()),
        },
        Value::Number(n) => style(n.to_string()).yellow(),
        Value::Bool(b) => style(b.to_string()).yellow(),
        Value::Null => style("null".to_string()).dim(),
        _ => unreachable!(),
    };
    println!("{}", formatted);
}

fn print_params(value: &Option<JsonObject>, depth: usize, debug: bool) {
    let indent = INDENT.repeat(depth);

    if let Some(json_object) = value {
        for (key, val) in json_object.iter() {
            match val {
                Value::Object(obj) => {
                    println!("{}{}:", indent, style(key).dim());
                    print_params(&Some(obj.clone()), depth + 1, debug);
                }
                Value::Array(arr) => {
                    // Check if all items are simple values (not objects or arrays)
                    let all_simple = arr.iter().all(|item| {
                        matches!(
                            item,
                            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                        )
                    });

                    if all_simple {
                        // Render inline for simple arrays, truncation will be handled by print_value if needed
                        let values: Vec<String> = arr
                            .iter()
                            .map(|item| match item {
                                Value::String(s) => s.clone(),
                                Value::Number(n) => n.to_string(),
                                Value::Bool(b) => b.to_string(),
                                Value::Null => "null".to_string(),
                                _ => unreachable!(),
                            })
                            .collect();
                        let joined_values = values.join(", ");
                        print_value_with_prefix(
                            &format!("{}{}: ", indent, style(key).dim()),
                            &Value::String(joined_values),
                            debug,
                        );
                    } else {
                        // Use the original multi-line format for complex arrays
                        println!("{}{}:", indent, style(key).dim());
                        for item in arr.iter() {
                            if let Value::Object(obj) = item {
                                println!("{}{}- ", indent, INDENT);
                                print_params(&Some(obj.clone()), depth + 2, debug);
                            } else {
                                println!("{}{}- {}", indent, INDENT, item);
                            }
                        }
                    }
                }
                _ => {
                    print_value_with_prefix(
                        &format!("{}{}: ", indent, style(key).dim()),
                        val,
                        debug,
                    );
                }
            }
        }
    }
}

fn shorten_path(path: &str, debug: bool) -> String {
    let home = etcetera::home_dir().ok();
    shorten_path_with_home(path, debug, home.as_deref())
}

fn shorten_path_with_home(path: &str, debug: bool, home: Option<&Path>) -> String {
    // In debug mode, return the full path
    if debug {
        return path.to_string();
    }

    let path = Path::new(path);

    // First try to convert to ~ if it's in home directory
    let path_str = if let Some(home) = home {
        if let Ok(stripped) = path.strip_prefix(home) {
            format!("~/{}", stripped.display())
        } else {
            path.display().to_string()
        }
    } else {
        path.display().to_string()
    };

    // If path is already short enough, return as is
    if path_str.len() <= 60 {
        return path_str;
    }

    let parts: Vec<_> = path_str.split('/').collect();

    // Keep the leading component plus the last few components in FULL, collapsing
    // only the middle into a single ellipsis. This preserves the readable
    // in-project path (…/project/src/module/file.rs) instead of abbreviating each
    // directory to a single letter (…/p/s/m/file.rs), which made it hard to tell
    // which file was being touched.
    const TAIL: usize = 4;
    if parts.len() <= TAIL + 2 {
        return path_str;
    }

    let mut shortened = vec![parts[0].to_string(), "…".to_string()];
    shortened.extend(parts[parts.len() - TAIL..].iter().map(|s| s.to_string()));
    shortened.join("/")
}

// Session display functions
pub fn display_session_info(
    resume: bool,
    provider: &str,
    model: &str,
    session_id: &Option<String>,
    provider_instance: Option<&Arc<dyn biorouter::providers::base::Provider>>,
    // Issue #56 Task 31 / R10. The tier is a property of the CHAT, not of the
    // provider row above it: a public model bound to a private chat is exactly
    // the state the banner has to make visible, and reading it off
    // `provider_instance` would report the model's tier instead and say
    // "Private" for the one case that matters.
    privacy: biorouter::privacy::SessionClassification,
) {
    let headline = if resume {
        "resuming chat"
    } else if session_id.is_none() {
        "running without session"
    } else {
        "starting chat"
    };

    // Helper to print one aligned `label   value` row under the headline.
    let row = |label: &str, value: String| {
        println!(
            "    {:<9} {}",
            style(label).dim(),
            style(value).cyan().dim()
        );
    };

    println!("  {} {}", style("▌").fg(ACCENT), style(headline).bold());

    // Lead/worker mode shows both model tiers; otherwise a single model row.
    match provider_instance.and_then(|p| p.as_lead_worker()) {
        Some(lead_worker) => {
            let (lead_model, worker_model) = lead_worker.get_model_info();
            row("provider", provider.to_string());
            row("lead", lead_model);
            row("worker", worker_model);
        }
        None => {
            row("provider", provider.to_string());
            row("model", model.to_string());
        }
    }

    if let Some(id) = session_id {
        row("session", id.clone());
    }

    row(
        "privacy",
        format!("{} (at start)", super::privacy::tier_row(privacy)),
    );

    row(
        "workdir",
        std::env::current_dir().unwrap().display().to_string(),
    );

    // Surface the primary knowledge base — the base a `--kb`-less write lands
    // in — so chat-side knowledge tools have visible context.
    if let Ok(svc) = biorouter::knowledge::service::KnowledgeService::new_default() {
        if let Some(kb) = primary_kb_row(&svc, session_id) {
            row("knowledge", kb);
        }
    }
}

/// The `knowledge` row of the session banner: the base a `--kb`-less write
/// lands in **for this session**.
///
/// This used to pass `None` — the machine-wide pointer — even though the
/// session id was right there. A session can pin its own primary, and the
/// agent's `kb_*` tools resolve `primary_for_session(session_id)`, so the
/// banner announced one base while the very next KB-less write went to
/// another. `primary_for_session` is the only correct resolver: it knows the
/// difference between "this session never chose" (inherit the machine
/// pointer) and "this session explicitly has none" (show nothing).
fn primary_kb_row(
    svc: &biorouter::knowledge::service::KnowledgeService,
    session_id: &Option<String>,
) -> Option<String> {
    svc.primary_for_session(session_id.as_deref())
        .ok()
        .flatten()
}

/// Small, simple "Biorouter" wordmark banner (3 lines), rendered in the brand
/// coral at the top of the greeting.
pub(crate) const BIOROUTER_ASCII: &[&str] = &[
    "█▀▄ █ █▀█ █▀▄ █▀█ █ █ ▀█▀ █▀▀ █▀▄",
    "█▀▄ █ █ █ █▀▄ █ █ █ █  █  █▀▀ █▀▄",
    "▀▀  ▀ ▀▀▀ ▀ ▀ ▀▀▀ ▀▀▀  ▀  ▀▀▀ ▀ ▀",
];

pub fn display_greeting() {
    if !std::io::stdout().is_terminal() {
        println!(
            "\nBiorouter is running! Enter your instructions, or try asking what Biorouter can do.\n"
        );
        return;
    }

    println!();
    for line in BIOROUTER_ASCII {
        println!("  {}", style(line).fg(ACCENT).bold());
    }
    println!();
    println!(
        "  {}",
        style("Biorouter: integrated biomedical research environment").bold()
    );
    println!(
        "  {}",
        style("Enter your instructions, or type /help for commands").dim()
    );
    println!();
}

/// Display context window usage with both current and session totals
pub fn display_context_usage(total_tokens: usize, context_limit: usize) {
    use console::style;

    if context_limit == 0 {
        println!("Context: Error - context limit is zero");
        return;
    }

    // Calculate percentage used with bounds checking
    let percentage =
        (((total_tokens as f64 / context_limit as f64) * 100.0).round() as usize).min(100);

    // Render a contiguous meter bar:  ████████░░░░░░░░░░░░
    let bar_width = 20;
    let filled_cells =
        (((percentage as f64 / 100.0) * bar_width as f64).round() as usize).min(bar_width);
    let empty_cells = bar_width - filled_cells;

    // The filled portion carries a load-based color (a usage gauge, so green→red
    // semantics rather than the brand accent); the track stays dim.
    let filled = "█".repeat(filled_cells);
    let colored_fill = if percentage < 50 {
        style(filled)
    } else if percentage < 85 {
        style(filled).yellow()
    } else {
        style(filled).red()
    };
    let bar = format!(
        "{}{}{}{}",
        style("▕").dim(),
        colored_fill,
        style("░".repeat(empty_cells)).dim(),
        style("▏").dim(),
    );

    // Print a sparse, dim status line:  Context ▕████████░░░░░░░░░░░░▏ 37%  ·  74.2k / 200k
    println!(
        "{}  {} {}  {}  {}",
        style("Context").dim(),
        bar,
        style(format!("{}%", percentage)).dim(),
        style("·").dim(),
        style(format!(
            "{} / {}",
            human_count(total_tokens),
            human_count(context_limit)
        ))
        .dim(),
    );
}

/// Display cost information, if price data is available.
pub fn display_cost_usage(provider: &str, model: &str, input_tokens: usize, output_tokens: usize) {
    // Priced by the shared estimator (provider overrides, then the canonical
    // catalog), so the CLI's cost line, the server's `/config/pricing` and the
    // BR-35 per-reply dollar budget can never disagree about what a turn cost.
    if let Some(cost) = biorouter::providers::pricing::estimate_cost_usd(
        provider,
        model,
        input_tokens as u64,
        output_tokens as u64,
    ) {
        use console::style;
        eprintln!(
            "Cost: {} USD ({} tokens: in {}, out {})",
            style(format!("${:.4}", cost)).cyan(),
            input_tokens + output_tokens,
            input_tokens,
            output_tokens
        );
    }
}

pub struct McpSpinners {
    bars: HashMap<String, ProgressBar>,
    log_spinner: Option<ProgressBar>,

    multi_bar: MultiProgress,
}

impl Default for McpSpinners {
    fn default() -> Self {
        Self::new()
    }
}

impl McpSpinners {
    pub fn new() -> Self {
        McpSpinners {
            bars: HashMap::new(),
            log_spinner: None,
            multi_bar: MultiProgress::new(),
        }
    }

    pub fn log(&mut self, message: &str) {
        let spinner = self.log_spinner.get_or_insert_with(|| {
            let bar = self.multi_bar.add(
                ProgressBar::new_spinner()
                    .with_style(
                        ProgressStyle::with_template("{spinner:.green} {msg}")
                            .unwrap()
                            .tick_chars("⠋⠙⠚⠛⠓⠒⠊⠉"),
                    )
                    .with_message(message.to_string()),
            );
            bar.enable_steady_tick(Duration::from_millis(100));
            bar
        });

        spinner.set_message(message.to_string());
    }

    pub fn update(&mut self, token: &str, value: f64, total: Option<f64>, message: Option<&str>) {
        let bar = self.bars.entry(token.to_string()).or_insert_with(|| {
            if let Some(total) = total {
                self.multi_bar.add(
                    ProgressBar::new((total * 100_f64) as u64).with_style(
                        ProgressStyle::with_template("[{elapsed}] {bar:40} {pos:>3}/{len:3} {msg}")
                            .unwrap(),
                    ),
                )
            } else {
                self.multi_bar.add(ProgressBar::new_spinner())
            }
        });
        bar.set_position((value * 100_f64) as u64);
        if let Some(msg) = message {
            bar.set_message(msg.to_string());
        }
    }

    pub fn hide(&mut self) -> Result<(), Error> {
        self.bars.iter_mut().for_each(|(_, bar)| {
            bar.disable_steady_tick();
        });
        if let Some(spinner) = self.log_spinner.as_mut() {
            spinner.disable_steady_tick();
        }
        self.multi_bar.clear()
    }
}

/// Render a full sample of the CLI's visual surfaces to stdout. Used by the
/// `preview_tui` example to eyeball the styling without a live session.
pub fn preview() {
    display_greeting();

    // Session info (single-model variant; no provider instance needed).
    let sid = Some("ab12cd34-ef56".to_string());
    display_session_info(
        false,
        "versa_azure",
        "gpt-5.2",
        &sid,
        None,
        biorouter::privacy::SessionClassification::Private,
    );

    // Section rules for a few representative tools.
    print_section_rule("text_editor", "developer");
    println!(
        "{}: {}",
        style("path").dim(),
        style("~/Desktop/biorouter/crates/biorouter-cli/src/session/output.rs")
    );
    print_section_rule("shell", "developer");
    println!(
        "{}: {}",
        style("command").dim(),
        style("cargo test -p biorouter-cli")
    );
    print_section_rule("3 tool calls", "execute_code");

    // Context meter at a few load levels.
    println!();
    display_context_usage(41_000, 200_000);
    display_context_usage(128_000, 200_000);
    display_context_usage(188_000, 200_000);

    // Task execution block.
    println!();
    println!("{}", super::task_execution_display::preview_block());

    // Status messages.
    render_extension_success("knowledge");
    render_error("could not reach provider endpoint (timeout)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use rmcp::model::{CallToolResult, Content, Meta, RawResource, ResourceContents};

    /// The default floor shows the tool output that tools actually produce.
    ///
    /// ⚠ The first assertion is the bug: the floor was 0.5, and the only
    /// priorities emitted anywhere in this repo are 0.2 (`text_editor`) and 0.0
    /// (`developer`, `analyze`). Every one of them is below 0.5, so the gate
    /// rejected all of them and the classic renderer printed nothing in its
    /// default configuration. Restore 0.5 and this fails.
    ///
    /// The rest pin what must NOT change while fixing that: a raised floor
    /// still filters, and an unannotated result is still hidden outside debug.
    /// Turning the second clause into "show anything without a priority" would
    /// spill raw protocol chatter into the transcript, and a floor of 0.0 makes
    /// that mistake easy to reach for.
    #[test]
    fn the_default_priority_floor_shows_real_tool_output() {
        for emitted in [0.0_f32, 0.2] {
            assert!(
                content_clears_priority_gate(Some(emitted), DEFAULT_CLI_MIN_PRIORITY, false),
                "priority {emitted} is a value tools really emit and must be shown by default"
            );
        }

        assert!(
            !content_clears_priority_gate(Some(0.2), 0.5, false),
            "a floor the user raised must still filter"
        );

        assert!(
            !content_clears_priority_gate(None, DEFAULT_CLI_MIN_PRIORITY, false),
            "an unannotated result stays hidden outside debug, floor or no floor"
        );
        assert!(
            content_clears_priority_gate(None, DEFAULT_CLI_MIN_PRIORITY, true),
            "and visible in debug"
        );
    }

    /// The code default and the two documented defaults are one number.
    ///
    /// They disagreed for the entire life of the file, and nothing noticed
    /// because a default is only wrong in its effects. Read the value out of
    /// both tables rather than restating it here, so this fails whichever side
    /// moves.
    #[test]
    fn the_documented_default_is_the_one_the_code_uses() {
        for (doc, text) in [
            (
                "docs/configuration/environment-variables.md",
                include_str!("../../../../docs/configuration/environment-variables.md"),
            ),
            (
                "docs/configuration/config-file-reference.md",
                include_str!("../../../../docs/configuration/config-file-reference.md"),
            ),
        ] {
            let row = text
                .lines()
                .find(|l| l.contains("BIOROUTER_CLI_MIN_PRIORITY"))
                .unwrap_or_else(|| panic!("{doc} no longer documents BIOROUTER_CLI_MIN_PRIORITY"));
            let documented = row
                .split('|')
                .map(str::trim)
                .filter_map(|cell| cell.parse::<f32>().ok())
                .next()
                .unwrap_or_else(|| panic!("{doc}: no numeric default in its row: {row}"));
            assert_eq!(
                documented, DEFAULT_CLI_MIN_PRIORITY,
                "{doc} documents {documented} but the code uses {DEFAULT_CLI_MIN_PRIORITY}"
            );
        }
    }

    /// The classic renderer prints the tool's output, not the guardrail's
    /// envelope around it.
    ///
    /// ⚠ A source check, because this path cannot be observed from a test:
    /// `render_tool_response` writes through `print_markdown_source`, which
    /// hands the text to `bat` and out to the real stdout. There is no seam to
    /// capture and no value returned, so the alternative to reading the source
    /// is asserting nothing at all.
    ///
    /// What it pins is the realistic regression: the display branch going back
    /// to printing `text.text` straight. It deliberately does NOT require the
    /// `{:#?}` debug branch to unwrap, which is a structural dump for
    /// diagnosis and is meant to show the wire format.
    #[test]
    fn the_classic_renderer_unframes_tool_output_before_printing_it() {
        let src = include_str!("output.rs");
        let (_, body) = src
            .split_once("fn render_tool_response(")
            .expect("render_tool_response is gone");
        let (body, _) = body
            .split_once("\n}\n")
            .expect("could not find the end of render_tool_response");

        assert!(
            body.contains("unframe_tool_output"),
            "render_tool_response no longer strips the guardrail frame, so every tool result \
             prints its `<tool-output untrusted=...>` envelope at the terminal"
        );
        assert!(
            !body.contains("print_markdown_source(&text.text"),
            "the display branch is printing the framed text directly again; it must pass \
             through unframe_tool_output first"
        );
    }

    fn embedded_html(uri: &str, mime_type: &str, html: &str) -> Content {
        Content::resource(ResourceContents::BlobResourceContents {
            uri: uri.to_string(),
            mime_type: Some(mime_type.to_string()),
            blob: base64::engine::general_purpose::STANDARD.encode(html),
            meta: None,
        })
    }

    #[test]
    fn test_title_from_ui_uri() {
        // Auto Visualiser chart URIs → titled artifacts (mirrors the desktop).
        assert_eq!(
            title_from_ui_uri("ui://volcano/plot").as_deref(),
            Some("Volcano Plot")
        );
        assert_eq!(
            title_from_ui_uri("ui://line/chart").as_deref(),
            Some("Line Chart")
        );
        // Dashboard slugs are hyphen/underscore separated and title-cased.
        assert_eq!(
            title_from_ui_uri("ui://dashboard/omics-summary").as_deref(),
            Some("Dashboard Omics Summary")
        );
        assert_eq!(
            title_from_ui_uri("ui://report/qc_report").as_deref(),
            Some("Report Qc Report")
        );
        // Query/fragment segments are ignored.
        assert_eq!(
            title_from_ui_uri("ui://bar/chart?v=2#top").as_deref(),
            Some("Bar Chart")
        );
        // Not a ui:// URI, or empty → None.
        assert_eq!(title_from_ui_uri("file:///tmp/x.html"), None);
        assert_eq!(title_from_ui_uri("ui://"), None);
    }

    #[test]
    fn ui_html_artifact_is_saved_as_a_browser_url() {
        let html = "<!doctype html><title>Safe preview</title>";
        let content = embedded_html("ui://chart/safe-preview", "text/html; charset=utf-8", html);
        let note = artifact_note_from_content(&content).expect("artifact note");

        assert_eq!(note.title, "Chart Safe Preview");
        let path = note.saved_path.expect("saved HTML path");
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains(html));
        assert!(saved.contains("Content-Security-Policy"));
        assert!(saved.contains("connect-src 'none'"));
        assert!(saved.contains("navigate-to 'none'"));
        assert!(saved.contains("sandbox=\"allow-scripts allow-downloads\""));
        assert!(saved.contains("credentialless referrerpolicy=\"no-referrer\""));
        assert!(!saved.contains("allow-same-origin"));
        assert_eq!(
            note.browser_url.as_deref(),
            url::Url::from_file_path(&path)
                .ok()
                .map(|url| url.to_string())
                .as_deref()
        );
        assert!(
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(
                    |name| name.starts_with("biorouter-artifact-") && name.ends_with(".html")
                )
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unsafe_or_non_html_artifacts_are_not_written() {
        let plain = embedded_html(
            "ui://chart/not-html",
            "text/plain",
            "<script>bad()</script>",
        );
        let note = artifact_note_from_content(&plain).expect("ui artifact note");
        assert!(note.saved_path.is_none());
        assert!(note.browser_url.is_none());

        let malformed = Content::resource(ResourceContents::BlobResourceContents {
            uri: "ui://chart/malformed".to_string(),
            mime_type: Some("text/html".to_string()),
            blob: "not-base64".to_string(),
            meta: None,
        });
        let note = artifact_note_from_content(&malformed).expect("ui artifact note");
        assert!(note.saved_path.is_none());

        let assistant_only = embedded_html(
            "ui://chart/private",
            "text/html",
            "<p>assistant-only detail</p>",
        )
        .with_audience(vec![rmcp::model::Role::Assistant]);
        assert!(artifact_note_from_content(&assistant_only).is_none());

        let oversized_uri = embedded_html(
            &format!("ui://chart/{}", "x".repeat(MAX_BROWSER_URL_BYTES)),
            "text/html",
            "<p>too long</p>",
        );
        assert!(artifact_note_from_content(&oversized_uri).is_none());
    }

    #[test]
    fn browser_resource_links_are_exposed_but_unsafe_schemes_are_rejected() {
        let mut resource = RawResource::new("https://example.test/report.html", "report");
        resource.title = Some("Study report".to_string());
        let note = artifact_note_from_content(&Content::resource_link(resource)).unwrap();
        assert_eq!(note.title, "Study report");
        assert_eq!(
            note.browser_url.as_deref(),
            Some("https://example.test/report.html")
        );

        let resource = RawResource::new("javascript:alert(1)", "unsafe");
        assert!(artifact_note_from_content(&Content::resource_link(resource)).is_none());

        let resource = RawResource::new("file:///etc/passwd", "local file");
        assert!(artifact_note_from_content(&Content::resource_link(resource)).is_none());

        let resource = RawResource::new("https://user:secret@example.test/report", "credentials");
        assert!(artifact_note_from_content(&Content::resource_link(resource)).is_none());

        let resource = RawResource::new(
            format!("https://example.test/{}", "x".repeat(MAX_BROWSER_URL_BYTES)),
            "oversized",
        );
        assert!(artifact_note_from_content(&Content::resource_link(resource)).is_none());

        let mut resource = RawResource::new("https://example.test/report", "report");
        resource.title = Some("Safe\u{1b}]8;;https://evil.test\u{7}spoof\u{202e}".to_string());
        let note = artifact_note_from_content(&Content::resource_link(resource)).unwrap();
        assert_eq!(note.title, "Safe]8;;https://evil.testspoof");
    }

    #[test]
    fn uri_list_artifacts_expose_the_first_safe_browser_url() {
        let content = Content::resource(ResourceContents::TextResourceContents {
            uri: "ui://report/published".to_string(),
            mime_type: Some("text/uri-list; charset=utf-8".to_string()),
            text: "# generated report\njavascript:alert(1)\nhttps://example.test/report.html\n"
                .to_string(),
            meta: None,
        });
        let note = artifact_note_from_content(&content).expect("artifact note");

        assert_eq!(note.title, "Report Published");
        assert!(note.saved_path.is_none());
        assert_eq!(
            note.browser_url.as_deref(),
            Some("https://example.test/report.html")
        );
    }

    #[test]
    fn app_launch_metadata_becomes_terminal_links() {
        let mut result = CallToolResult::success(vec![]);
        result.meta = Some(Meta(
            serde_json::from_value(serde_json::json!({
                "biorouter/app-path": "/apps/direct/",
                "biorouter/app-paths": ["/apps/nested/", "/apps/direct/", "/apps/../escape/"]
            }))
            .unwrap(),
        ));

        let notes = app_launch_notes(&result);
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().any(|note| {
            note.title == "App: direct"
                && note
                    .browser_url
                    .as_deref()
                    .is_some_and(|url| url.ends_with("/apps/direct/"))
        }));
        assert!(notes.iter().any(|note| note.title == "App: nested"));
    }

    #[test]
    fn app_launch_metadata_does_not_duplicate_a_resource_link() {
        let mut result = CallToolResult::success(vec![Content::resource_link(RawResource::new(
            "http://127.0.0.1:3000/apps/direct/",
            "direct app",
        ))]);
        result.meta = Some(Meta(
            serde_json::from_value(serde_json::json!({
                "biorouter/app-path": "/apps/direct/"
            }))
            .unwrap(),
        ));

        assert!(app_launch_notes(&result).is_empty());

        result.content = vec![Content::resource_link(RawResource::new(
            "https://example.test/apps/direct/",
            "unrelated external app",
        ))];
        assert_eq!(app_launch_notes(&result).len(), 1);

        result.content = vec![Content::resource_link(RawResource::new(
            "http://127.0.0.1:3000/apps/direct/",
            "assistant-only app link",
        ))
        .with_audience(vec![rmcp::model::Role::Assistant])];
        assert_eq!(app_launch_notes(&result).len(), 1);
    }

    #[test]
    fn csp_injection_does_not_treat_header_as_head() {
        let html = "<html><header>Title</header><main>Preview</main></html>";
        let secured = inject_artifact_browser_csp(html);

        assert!(secured.starts_with("<html><head><meta "));
        assert!(secured.contains("</head><header>"));
    }

    #[test]
    fn csp_injection_precedes_content_before_a_late_head() {
        let html = "<script>window.ran=true</script><head><title>Late</title></head>";
        let secured = inject_artifact_browser_csp(html);

        assert!(secured.starts_with("<head><meta "));
        assert!(
            secured.find("Content-Security-Policy").unwrap() < secured.find("<script>").unwrap()
        );
    }

    #[test]
    fn csp_injection_preserves_unicode_around_insertion_points() {
        for (html, marker) in [
            (
                r#"<!doctype html><html><head data-title="Résumé"><title>東京</title></head></html>"#,
                "Résumé",
            ),
            ("<html>é<head><title>Preview</title></head></html>", "é"),
            (
                "Предисловие<head><title>Preview</title></head>",
                "Предисловие",
            ),
        ] {
            let secured = inject_artifact_browser_csp(html);

            assert!(secured.contains("Content-Security-Policy"));
            assert!(secured.contains(marker));
        }
    }

    #[test]
    fn browser_wrapper_escapes_srcdoc_and_blocks_top_navigation() {
        let wrapped = wrap_artifact_for_browser(
            r#"<script>top.location='https://example.test/?a=1&b="break"'</script>"#,
        );

        assert!(wrapped.contains("&amp;"));
        assert!(wrapped.contains("&quot;break&quot;"));
        assert!(wrapped.contains("name=\"biorouter-artifact-preview\""));
        assert!(wrapped.contains("sandbox=\"allow-scripts allow-downloads\""));
        assert!(!wrapped.contains("allow-top-navigation"));
        assert!(!wrapped.contains("allow-same-origin"));
    }

    #[test]
    fn test_short_paths_unchanged() {
        assert_eq!(shorten_path("/usr/bin", false), "/usr/bin");
        assert_eq!(shorten_path("/a/b/c", false), "/a/b/c");
        assert_eq!(shorten_path("file.txt", false), "file.txt");
    }

    #[test]
    fn test_debug_mode_returns_full_path() {
        assert_eq!(
            shorten_path("/very/long/path/that/would/normally/be/shortened", true),
            "/very/long/path/that/would/normally/be/shortened"
        );
    }

    #[test]
    fn test_home_directory_conversion() {
        let home = Path::new("root").join("testuser");
        let path_in_home = home.join("documents").join("file.txt");

        assert_eq!(
            shorten_path_with_home(path_in_home.to_str().unwrap(), false, Some(&home)),
            format!("~/{}", Path::new("documents").join("file.txt").display())
        );

        let sibling_path = Path::new("root")
            .join("testuser2")
            .join("documents")
            .join("file.txt");
        assert_eq!(
            shorten_path_with_home(sibling_path.to_str().unwrap(), false, Some(&home)),
            sibling_path.display().to_string()
        );
    }

    #[test]
    fn test_toggle_full_tool_output() {
        let initial = get_show_full_tool_output();

        let after_first_toggle = toggle_full_tool_output();
        assert_eq!(after_first_toggle, !initial);
        assert_eq!(get_show_full_tool_output(), after_first_toggle);

        let after_second_toggle = toggle_full_tool_output();
        assert_eq!(after_second_toggle, initial);
        assert_eq!(get_show_full_tool_output(), initial);
    }

    /// The banner must name the base *this session* writes to. A session can
    /// pin its own primary — the agent's `kb_*` tools resolve
    /// `primary_for_session(session_id)` — so reading the machine-wide pointer
    /// here announced one base while the agent silently wrote to another.
    #[test]
    fn the_knowledge_row_names_this_session_s_primary_not_the_machine_s(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use biorouter::knowledge::service::KnowledgeService;

        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("machine-kb", "Machine", None)?;
        svc.create_base("session-kb", "Session", None)?;
        svc.set_primary_persisted(Some("machine-kb"))?;
        svc.set_primary_for_session("s1", Some("session-kb"))?;

        assert_eq!(
            primary_kb_row(&svc, &Some("s1".to_string())).as_deref(),
            Some("session-kb")
        );
        // No session (`biorouter run` without one) still falls back to the
        // machine-wide pointer, and a session that never chose inherits it.
        assert_eq!(primary_kb_row(&svc, &None).as_deref(), Some("machine-kb"));
        assert_eq!(
            primary_kb_row(&svc, &Some("s2".to_string())).as_deref(),
            Some("machine-kb")
        );
        Ok(())
    }

    #[test]
    fn test_long_path_shortening() {
        // Long paths collapse the middle to a single ellipsis but keep the last
        // few components (the in-project path) in full, so it's clear which file
        // is being touched.
        assert_eq!(
            shorten_path(
                "/vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv/long/path/with/many/components/file.txt",
                false
            ),
            "/…/with/many/components/file.txt"
        );
    }
}
