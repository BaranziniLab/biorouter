//! Full-screen TUI front-end for interactive sessions — a Claude-Code-style
//! layout with a scrolling history pane and a pinned bottom input box.
//!
//! The agent's streaming `reply()` events are consumed inline (so the borrow of
//! `agent` and the mutation of `messages` stay disjoint, exactly like the
//! classic readline loop) and rendered into a ratatui scrollback buffer.

mod app;

use std::io::Stdout;
use std::time::Duration;

use anyhow::Result;
use biorouter::agents::{AgentEvent, SessionConfig};
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::permission::permission_confirmation::PrincipalType;
use biorouter::permission::{Permission, PermissionConfirmation};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
#[cfg(unix)]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use rmcp::model::{ErrorCode, ErrorData, Role};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use biorouter::conversation::Conversation;

use self::app::{App, PermissionModal, StatusInfo, ACCENT};
use super::CliSession;

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Max queued messages shown in the preview pane (older ones beyond this are
/// summarized as a "+N more" line so the pane never eats the screen).
const QUEUED_PREVIEW_MAX: u16 = 4;

/// Terminal input events are read by a single dedicated task and forwarded over
/// this channel, so the draw/stream loops `recv()` (cancel-safe, buffered)
/// instead of racing multiple `EventStream::next()` futures across `select!`
/// arms — which could drop a partially-decoded keypress (e.g. a mid-stream
/// Ctrl-C).
type Events = mpsc::UnboundedReceiver<Event>;

/// RAII guard owning the alternate screen + raw mode; restores on drop.
struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    fn new() -> Result<Self> {
        // Restore the terminal on panic *before* the default hook prints, so a
        // render panic doesn't leave the user in raw mode / alt screen.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(
                std::io::stdout(),
                SetCursorStyle::DefaultUserShape,
                DisableBracketedPaste,
                LeaveAlternateScreen,
                DisableMouseCapture
            );
            #[cfg(unix)]
            let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
            prev(info);
        }));

        enable_raw_mode()?;
        let mut out = std::io::stdout();
        // A blinking vertical bar (I-beam) cursor at the insertion point, plus
        // bracketed paste so multi-line pastes arrive as one chunk instead of a
        // flood of keystrokes (which would submit prematurely on embedded \n).
        //
        // Mouse capture is ON so the scroll wheel scrolls the *conversation*
        // (the app scrollback) rather than the terminal's own buffer — without it,
        // in the alternate screen a wheel-scroll just exposed the pre-launch banner
        // and the conversation looked lost. Click-drag inside the input now
        // drives the app's own text selection; native terminal selection still
        // works through the terminal's bypass modifier.
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            SetCursorStyle::BlinkingBar
        )?;
        #[cfg(unix)]
        execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok(Self { terminal })
    }

    fn draw(&mut self, app: &mut App) -> Result<()> {
        self.terminal.draw(|f| draw(f, app))?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            SetCursorStyle::DefaultUserShape,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        #[cfg(unix)]
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        let _ = self.terminal.show_cursor();
    }
}

/// Run the interactive session in the full-screen TUI.
pub async fn run(session: &mut CliSession, initial_prompt: Option<String>) -> Result<()> {
    let mut tui = Tui::new()?;
    let mut app = App::new(status_from_session(session));
    app.set_catalog(build_catalog(session));
    greeting_into(&mut app);
    push_startup_notices(&mut app).await;
    refresh_context(session, &mut app).await;

    // Single dedicated reader task → channel (see `Events`).
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let reader = tokio::spawn(async move {
        let mut es = EventStream::new();
        while let Some(Ok(ev)) = es.next().await {
            if tx.send(ev).is_err() {
                break;
            }
        }
    });
    let _reader = AbortOnDropHandle::new(reader);

    if let Some(prompt) = initial_prompt {
        submit(session, &mut app, &mut tui, &mut rx, prompt).await?;
        drain_queue(session, &mut app, &mut tui, &mut rx).await?;
    }

    while !app.should_quit {
        tui.draw(&mut app)?;
        let Some(event) = rx.recv().await else {
            break; // event reader closed
        };
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(submission) = on_key(&mut app, key) {
                    submit(session, &mut app, &mut tui, &mut rx, submission).await?;
                    // Anything typed while that turn streamed runs next, in order.
                    drain_queue(session, &mut app, &mut tui, &mut rx).await?;
                }
            }
            Event::Paste(s) => app.paste(&s),
            Event::Mouse(m) => on_mouse(&mut app, m),
            _ => {}
        }
    }
    Ok(())
}

/// Handle a key press in the idle (composing) state. Returns the text to submit
/// when the user presses Enter on a non-empty buffer.
fn on_key(app: &mut App, key: KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    // The completion popup captures navigation keys first.
    if app.completion_active() {
        match key.code {
            KeyCode::Up => {
                app.completion_move(-1);
                return None;
            }
            KeyCode::Down => {
                app.completion_move(1);
                return None;
            }
            KeyCode::Tab | KeyCode::Enter => {
                // Accept the highlighted entry rather than submitting the raw
                // half-typed token (so arrow-key selection actually applies).
                app.completion_accept();
                app.refresh_completion();
                return None;
            }
            KeyCode::Esc => {
                app.dismiss_completion();
                return None;
            }
            _ => {}
        }
    }

    if handle_clipboard_shortcut(app, key) {
        app.refresh_completion();
        return None;
    }

    let mut submit = None;
    match key.code {
        KeyCode::Char('c') if ctrl => {
            if app.input.is_empty() {
                app.should_quit = true;
            } else {
                app.clear_input();
            }
        }
        KeyCode::Char('d') if ctrl => app.should_quit = true,
        KeyCode::Char('j') if ctrl => app.insert_newline(),
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.insert_newline()
        }
        KeyCode::Enter => {
            let text = app.input.trim().to_string();
            if !text.is_empty() {
                app.history.push(text.clone());
                app.clear_input();
                submit = Some(text);
            }
        }
        KeyCode::Tab => app.accept_ghost(),
        KeyCode::Backspace => app.backspace(),
        KeyCode::Delete => app.delete_forward(),
        KeyCode::Left => app.move_left_extending(shift),
        KeyCode::Right => app.move_right_extending(shift),
        KeyCode::Home => app.move_home_extending(shift),
        KeyCode::End => app.move_end_extending(shift),
        KeyCode::Up if should_recall_history(app, shift) => app.history_prev(),
        KeyCode::Down if should_recall_history(app, shift) => app.history_next(),
        KeyCode::Up => move_cursor_vertical(app, -1, shift),
        KeyCode::Down => move_cursor_vertical(app, 1, shift),
        KeyCode::PageUp => app.scroll_up(10),
        KeyCode::PageDown => app.scroll_down(10),
        KeyCode::Char(c) => app.insert_char(c),
        _ => {}
    }
    // Recompute the popup against the (possibly edited) input.
    app.refresh_completion();
    submit
}

/// Outcome of a keypress received *while a response is streaming*.
enum StreamAction {
    /// Buffer edited (or nothing to do) — keep streaming.
    None,
    /// Stop the in-flight response (Ctrl-C).
    Cancel,
    /// User submitted a line — queue it to run after the current turn.
    Queue(String),
}

/// Handle a key while the agent is replying: full input editing stays live, so
/// the user can compose the next message (or steer) instead of being locked
/// out. Enter queues; Ctrl-C cancels the in-flight turn.
fn on_key_streaming(app: &mut App, key: KeyEvent) -> StreamAction {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    if app.completion_active() {
        match key.code {
            KeyCode::Up => {
                app.completion_move(-1);
                return StreamAction::None;
            }
            KeyCode::Down => {
                app.completion_move(1);
                return StreamAction::None;
            }
            KeyCode::Tab | KeyCode::Enter => {
                app.completion_accept();
                app.refresh_completion();
                return StreamAction::None;
            }
            KeyCode::Esc => {
                app.dismiss_completion();
                return StreamAction::None;
            }
            _ => {}
        }
    }

    if handle_clipboard_shortcut(app, key) {
        app.refresh_completion();
        return StreamAction::None;
    }

    match key.code {
        KeyCode::Char('c') if ctrl => return StreamAction::Cancel,
        KeyCode::Char('j') if ctrl => app.insert_newline(),
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.insert_newline()
        }
        KeyCode::Enter => {
            let text = app.input.trim().to_string();
            if !text.is_empty() {
                app.history.push(text.clone());
                app.clear_input();
                return StreamAction::Queue(text);
            }
        }
        KeyCode::Tab => app.accept_ghost(),
        KeyCode::Backspace => app.backspace(),
        KeyCode::Delete => app.delete_forward(),
        KeyCode::Left => app.move_left_extending(shift),
        KeyCode::Right => app.move_right_extending(shift),
        KeyCode::Home => app.move_home_extending(shift),
        KeyCode::End => app.move_end_extending(shift),
        KeyCode::Up => move_cursor_vertical(app, -1, shift),
        KeyCode::Down => move_cursor_vertical(app, 1, shift),
        KeyCode::PageUp => app.scroll_up(10),
        KeyCode::PageDown => app.scroll_down(10),
        KeyCode::Char(c) => app.insert_char(c),
        _ => {}
    }
    app.refresh_completion();
    StreamAction::None
}

fn shortcut_modifier(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META)
        || key.modifiers.contains(KeyModifiers::CONTROL)
}

fn handle_clipboard_shortcut(app: &mut App, key: KeyEvent) -> bool {
    if !shortcut_modifier(key) {
        return false;
    }
    match key.code {
        KeyCode::Char('a') => {
            app.select_all();
            true
        }
        KeyCode::Char('c')
            if key.modifiers.contains(KeyModifiers::SUPER)
                || key.modifiers.contains(KeyModifiers::META) =>
        {
            if let Some(text) = app.selected_text() {
                write_clipboard(&text);
            }
            true
        }
        KeyCode::Char('x') => {
            if let Some(text) = app.cut_selection() {
                write_clipboard(&text);
            }
            true
        }
        KeyCode::Char('v') => {
            if let Some(text) = read_clipboard() {
                app.paste(&text);
            }
            true
        }
        _ => false,
    }
}

fn read_clipboard() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok()
}

fn write_clipboard(text: &str) {
    if text.is_empty() {
        return;
    }
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_string());
    }
}

fn should_recall_history(app: &App, shift: bool) -> bool {
    !shift && input_visual_rows(&app.input, app.last_input_text_width).len() <= 1
}

fn move_cursor_vertical(app: &mut App, delta: i16, extend: bool) {
    let width = app.last_input_text_width.max(1);
    let rows = input_visual_rows(&app.input, width);
    let (row, col) = visual_position(&rows, &app.input, app.cursor);
    let next_row = if delta < 0 {
        row.saturating_sub(delta.unsigned_abs() as usize)
    } else {
        (row + delta as usize).min(rows.len().saturating_sub(1))
    };
    let cursor = byte_at_visual_cell(&rows, &app.input, next_row, col);
    app.set_cursor(cursor, extend);
}

fn on_mouse(app: &mut App, m: MouseEvent) {
    match m.kind {
        MouseEventKind::ScrollUp => app.scroll_up(3),
        MouseEventKind::ScrollDown => app.scroll_down(3),
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(cursor) = input_cursor_at_mouse(app, m.column, m.row) {
                if m.modifiers.contains(KeyModifiers::SHIFT) {
                    app.set_cursor(cursor, true);
                } else {
                    app.start_mouse_selection(cursor);
                }
                app.refresh_completion();
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(cursor) = input_cursor_at_mouse(app, m.column, m.row) {
                app.drag_mouse_selection(cursor);
                app.refresh_completion();
            }
        }
        MouseEventKind::Up(MouseButton::Left) => app.finish_mouse_selection(),
        _ => {}
    }
}

fn input_cursor_at_mouse(app: &App, column: u16, row: u16) -> Option<usize> {
    let inner = app.last_input_inner?;
    if column < inner.x
        || column >= inner.x.saturating_add(inner.width)
        || row < inner.y
        || row >= inner.y.saturating_add(inner.height)
    {
        return None;
    }
    let target_row = row.saturating_sub(inner.y) as usize;
    let target_col = column.saturating_sub(inner.x).saturating_sub(2) as usize;
    let rows = input_visual_rows(&app.input, app.last_input_text_width);
    Some(byte_at_visual_cell(
        &rows, &app.input, target_row, target_col,
    ))
}

/// Submit a line: handle TUI-local slash commands, else send to the agent.
async fn submit(
    session: &mut CliSession,
    app: &mut App,
    tui: &mut Tui,
    rx: &mut Events,
    text: String,
) -> Result<()> {
    // /compact runs a normal turn whose trigger tells the agent to condense the
    // history (mirrors the classic `/compact`).
    if text.trim() == "/compact" {
        app.push_note("Compacting the chat…");
        let msg = Message::user().with_text(biorouter::agents::COMPACT_TRIGGERS[0]);
        session.messages.push(msg.clone());
        return drive_response(session, app, tui, rx, msg).await;
    }
    if text.starts_with('/') && handle_slash(session, app, &text).await {
        return Ok(());
    }

    app.push_user(&text);
    let user_message = Message::user().with_text(&text);
    session.messages.push(user_message.clone());
    drive_response(session, app, tui, rx, user_message).await
}

/// Run any submissions the user queued (by typing + Enter) while the previous
/// response was streaming, in the order they were entered.
async fn drain_queue(
    session: &mut CliSession,
    app: &mut App,
    tui: &mut Tui,
    rx: &mut Events,
) -> Result<()> {
    while let Some(text) = app.queued.pop_front() {
        submit(session, app, tui, rx, text).await?;
    }
    Ok(())
}

/// Consume the agent's streaming reply, rendering each event into the
/// scrollback while keeping the UI responsive (spinner ticks, scroll, cancel).
async fn drive_response(
    session: &mut CliSession,
    app: &mut App,
    tui: &mut Tui,
    rx: &mut Events,
    user_message: Message,
) -> Result<()> {
    session.last_abort = None;
    let config = SessionConfig {
        id: session.session_id.clone(),
        schedule_id: session.scheduled_job_id.clone(),
        max_turns: session.max_turns,
        max_tool_calls: None,
        budget: None,
        retry_config: session.retry_config.clone(),
        reasoning_effort: None,
    };
    let debug = session.debug;
    let cancel = CancellationToken::new();
    app.thinking = Some(super::thinking::get_random_thinking_message().to_string());

    // Consume the raw reply stream so assistant text shows token-by-token: each
    // delta is appended to a live preview (re-rendered as Markdown in place), and
    // the completed text is committed once the run ends — so streaming is visible
    // *and* tables/code/lists still render correctly when finished.
    let mut stream = session
        .agent
        .reply(user_message, config, Some(cancel.clone()))
        .await?;
    let mut tick = tokio::time::interval(Duration::from_millis(110));
    // BR-53: cap redraws to ~60 fps *while streaming text* so a fast token
    // stream doesn't re-render the whole Markdown preview on every delta (the
    // reparse is deferred into `draw` via `render_stream_if_dirty`). When no
    // text is streaming (spinner, tool output, idle) we draw every iteration, so
    // interactive latency is unchanged.
    let frame = Duration::from_millis(16);
    let mut last_draw = std::time::Instant::now()
        .checked_sub(frame)
        .unwrap_or_else(std::time::Instant::now);

    loop {
        let now = std::time::Instant::now();
        if app.stream_start.is_none() || now.duration_since(last_draw) >= frame {
            tui.draw(app)?;
            last_draw = now;
        }
        tokio::select! {
            _ = tick.tick() => { app.spin = app.spin.wrapping_add(1); }
            ev = rx.recv() => {
                match ev {
                    Some(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                        match on_key_streaming(app, k) {
                            StreamAction::Cancel => cancel.cancel(),
                            // Park it to run after the current turn. We do NOT
                            // push to the scrollback here: the live stream preview
                            // re-renders by truncating back to its start, which
                            // would wipe the line. The count shows in the input
                            // title instead (see draw_input).
                            StreamAction::Queue(text) => app.queued.push_back(text),
                            StreamAction::None => {}
                        }
                    }
                    Some(Event::Paste(s)) => app.paste(&s),
                    Some(Event::Mouse(m)) => on_mouse(app, m),
                    _ => {}
                }
            }
            res = stream.next() => {
                match res {
                    Some(Ok(AgentEvent::Message(message))) => {
                        if let Some((id, prompt)) = super::find_tool_confirmation(&message) {
                            commit_stream_to_session(app, &mut session.messages);
                            app.push_message(&message, debug);
                            app.thinking = None;
                            let permission = run_permission_modal(app, tui, rx, prompt).await?;
                            if permission == Permission::Cancel {
                                let mut resp = Message::user();
                                resp.content.push(MessageContent::tool_response(
                                    id,
                                    Err(ErrorData {
                                        code: ErrorCode::INVALID_REQUEST,
                                        message: std::borrow::Cow::from("Tool call cancelled by user"),
                                        data: None,
                                    }),
                                ));
                                session.messages.push(resp);
                                app.push_note("Tool call cancelled.");
                                cancel.cancel();
                                // Drain any events the agent already queued so the
                                // in-memory conversation doesn't desync.
                                while stream.next().await.is_some() {}
                                break;
                            }
                            session
                                .agent
                                .handle_confirmation_for_session(
                                    session.session_id(),
                                    id,
                                    PermissionConfirmation {
                                        principal_type: PrincipalType::Tool,
                                        permission,
                                    },
                                    // The modal is the same local human surface
                                    // as the CLI prompt: a person pressed a key
                                    // in a terminal, and no model can author
                                    // that.
                                    biorouter::pending_user_action::DecisionAuthority::from_local_human_surface(),
                                )
                                .await;
                            app.thinking = Some(super::thinking::get_random_thinking_message().to_string());
                        } else if super::find_elicitation_request(&message).is_some() {
                            commit_stream_to_session(app, &mut session.messages);
                            app.push_note("This step needs an interactive form not yet supported in the TUI, so it is being cancelled. Use `BIOROUTER_CLI_CLASSIC=1` for that flow.");
                            cancel.cancel();
                            while stream.next().await.is_some() {}
                            break;
                        } else if is_stream_text(&message) {
                            // A streaming assistant-text delta: stop the spinner
                            // and grow the live preview token-by-token.
                            app.thinking = None;
                            let id = message.id.clone();
                            let delta = message.as_concat_text();
                            app.stream_delta(id, &delta);
                        } else {
                            // Any non-text event ends the streamed block: commit it
                            // first so ordering is preserved, then render this one.
                            commit_stream_to_session(app, &mut session.messages);
                            session.messages.push(message.clone());
                            app.push_message(&message, debug);
                        }
                    }
                    Some(Ok(AgentEvent::McpNotification(_))) => {}
                    Some(Ok(AgentEvent::HistoryReplaced(c))) => { session.messages = c; }
                    Some(Ok(AgentEvent::ModelChange { .. })) => {}
                    // Issue #56 Gate B's repair arm. The TUI shows its binding in
                    // the status line and reads it from the same session the
                    // agent does, so it is already displaying the pinned model;
                    // the arm this frame exists for is the desktop composer's.
                    Some(Ok(AgentEvent::PrivacyProviderPinned { .. })) => {}
                    // BR-52: the TUI reads token counts from the session row.
                    Some(Ok(AgentEvent::TokenUsage(_))) => {}
                    // Advisory pending tool-call hint; TUI renders authoritative
                    // tool requests only.
                    Some(Ok(AgentEvent::ToolCallPending(_))) => {}
                    // #59: persisted-id bookkeeping; nothing for the TUI to draw.
                    Some(Ok(AgentEvent::MessagesPersisted(_))) => {}
                    Some(Ok(AgentEvent::TurnAborted { code, message })) => {
                        // The assistant Message carrying the human-readable text was
                        // already pushed; add a real error line so the turn does not
                        // look like it simply finished.
                        commit_stream_to_session(app, &mut session.messages);
                        app.push_error(&format!("{}: {message}", code.wire_code()));
                        session.last_abort = Some(code);
                        cancel.cancel();
                        break;
                    }
                    Some(Err(e)) => {
                        // Commit any streamed text first so the error renders
                        // *after* it (and isn't wiped by the preview truncation).
                        commit_stream_to_session(app, &mut session.messages);
                        app.push_error(&e.to_string());
                        cancel.cancel();
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    drop(stream);
    // Commit whatever streamed (including a partial reply if the user cancelled).
    commit_stream_to_session(app, &mut session.messages);
    app.thinking = None;
    refresh_context(session, app).await;
    tui.draw(app)?;
    Ok(())
}

/// A message that is a streamable assistant-text delta (text only, no tool
/// calls / thinking / notifications).
fn is_stream_text(m: &Message) -> bool {
    m.role == Role::Assistant
        && !m.content.is_empty()
        && m.content
            .iter()
            .all(|c| matches!(c, MessageContent::Text(_)))
}

/// Commit any in-progress streamed assistant text into permanent scrollback and
/// mirror it into the session's message list exactly once.
///
/// Takes `&mut Vec<Message>` (not `&mut CliSession`) on purpose: the reply
/// `stream` holds an immutable borrow of `session.agent` for the whole loop, so
/// only a *disjoint* field of the session may be borrowed mutably meanwhile.
fn commit_stream_to_session(app: &mut App, messages: &mut Conversation) {
    if let Some(text) = app.stream_commit() {
        messages.push(Message::assistant().with_text(text));
    }
}

/// Show the permission modal and block (within the response loop) until the
/// user chooses an option.
async fn run_permission_modal(
    app: &mut App,
    tui: &mut Tui,
    rx: &mut Events,
    prompt: Option<String>,
) -> Result<Permission> {
    let options: Vec<(&'static str, &'static str)> = if prompt.is_some() {
        vec![
            ("Allow", "allow once"),
            ("Deny", "deny"),
            ("Cancel", "cancel response"),
        ]
    } else {
        vec![
            ("Allow", "allow once"),
            ("Always allow", "always allow"),
            ("Deny", "deny"),
            ("Cancel", "cancel response"),
        ]
    };
    let len = options.len();
    app.modal = Some(PermissionModal {
        prompt,
        options,
        selected: 0,
    });

    let result = loop {
        tui.draw(app)?;
        let k = match rx.recv().await {
            Some(Event::Key(k)) => k,
            Some(_) => continue,
            None => break Permission::Cancel, // event reader closed
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        let modal = app.modal.as_mut().unwrap();
        match k.code {
            KeyCode::Up => modal.selected = (modal.selected + len - 1) % len,
            KeyCode::Down => modal.selected = (modal.selected + 1) % len,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                break Permission::Cancel
            }
            KeyCode::Esc => break Permission::Cancel,
            KeyCode::Enter => break label_to_permission(modal.options[modal.selected].0),
            _ => {}
        }
    };
    app.modal = None;
    Ok(result)
}

fn label_to_permission(label: &str) -> Permission {
    match label {
        "Allow" => Permission::AllowOnce,
        "Always allow" => Permission::AlwaysAllow,
        "Deny" => Permission::DenyOnce,
        _ => Permission::Cancel,
    }
}

/// Handle slash commands that the TUI services locally. Returns true if handled.
/// (`/compact` is handled in `submit` since it drives a turn.)
async fn handle_slash(session: &mut CliSession, app: &mut App, text: &str) -> bool {
    match text.trim() {
        "/exit" | "/quit" => {
            app.should_quit = true;
            true
        }
        "/clear" => {
            // Clear the persisted conversation + token counts too, so the
            // session and the context meter don't desync.
            if let Err(e) = session.clear_conversation().await {
                app.push_error(&e.to_string());
            } else {
                app.scrollback.clear();
                app.status.used_tokens = 0;
                greeting_into(app);
            }
            true
        }
        "/help" | "/?" => {
            help_into(app);
            true
        }
        s if s == "/diverge" || s.starts_with("/diverge ") => {
            let name = s.strip_prefix("/diverge").unwrap_or("").trim().to_string();
            let name = (!name.is_empty()).then_some(name);
            match session.diverge_and_open(|url| open::that(url), name).await {
                Ok(outcome) => match outcome.open_error {
                    None => app.push_note(&format!(
                        "Branched into a new window (session ID {}). This chat is unchanged.",
                        outcome.new_session_id
                    )),
                    Some(err) => app.push_note(&format!(
                        "Created a branched chat (session ID {}) but couldn't open a window ({}). Open Biorouter and run: {}",
                        outcome.new_session_id, err, outcome.url
                    )),
                },
                Err(e) => app.push_error(&format!("Couldn't diverge: {e}")),
            }
            true
        }
        s if s == "/rename" || s.starts_with("/rename ") => {
            let name = s.strip_prefix("/rename").unwrap_or("").trim();
            if name.is_empty() {
                app.push_note("Usage: /rename <new name>");
            } else {
                match session
                    .agent
                    .config
                    .session_manager
                    .update(session.session_id())
                    .user_provided_name(name)
                    .apply()
                    .await
                {
                    Ok(()) => app.push_note(&format!("Renamed this chat to \"{name}\".")),
                    Err(e) => app.push_error(&format!("Couldn't rename chat: {e}")),
                }
            }
            true
        }
        // Agent-serviced commands: let submit() send them through the normal
        // reply flow, where Agent::execute_command handles them. This includes
        // the deterministic resource-reference markers (/skill: /ext: /kb:, and
        // their function-call forms) — the agent's reply path extracts them, so
        // they must NOT be swallowed here.
        s if ["/goal", "/loop", "/schedule"]
            .iter()
            .any(|cmd| s == *cmd || s.starts_with(&format!("{cmd} "))) =>
        {
            false
        }
        s if ["/skill:", "/ext:", "/kb:", "/skill(", "/ext(", "/kb("]
            .iter()
            .any(|prefix| s.starts_with(prefix)) =>
        {
            false
        }
        other => {
            app.push_note(&format!(
                "`{}` isn't available in the TUI yet. Run `BIOROUTER_CLI_CLASSIC=1 biorouter session` for the full command set.",
                other
            ));
            true
        }
    }
}

// ── rendering ────────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &mut App) {
    // BR-53: flush any streamed delta buffered since the last frame into the
    // Markdown preview. Doing it here (once per frame) rather than in
    // `stream_delta` (once per token) is what caps the reparse at frame rate.
    app.render_stream_if_dirty();
    // Inset the whole UI so nothing renders edge-to-edge: 4 columns of left/right
    // padding and 1 row top/bottom.
    let area = f.area().inner(Margin::new(4, 1));
    // Height the input box to its *wrapped* row count (borders 2 + prompt 2 ⇒
    // text width = area.width − 4), so long lines soft-wrap and the box grows
    // like a textarea instead of clipping. Capped so it never eats the screen.
    let input_text_w = area.width.saturating_sub(4).max(1);
    let input_h = input_rows(&app.input, input_text_w).clamp(1, 10) + 2;
    let gap_h = 2u16; // blank rows separating the response from the input UI
                      // A "queued" preview pane: the messages the user typed while the agent was
                      // busy, shown in full (capped) so they can SEE what runs next instead of just
                      // a count. 0 rows when nothing is queued.
    let queued_h: u16 = if app.queued.is_empty() {
        0
    } else {
        // header (1) + shown messages + a "+N more" line when over the cap
        let over = u16::from(app.queued.len() as u16 > QUEUED_PREVIEW_MAX);
        (app.queued.len() as u16).min(QUEUED_PREVIEW_MAX) + 1 + over
    };
    let status_h = 2u16; // model/provider on line 1; counts + context on line 2
    let hints_h = 1u16;
    // The input cluster (status + box + hints) is pinned to the bottom and never
    // moves as the conversation grows; the history pane flexibly fills all the
    // space above it and scrolls to keep the latest output in view (Claude-style).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1), // history → all remaining space at the top
            Constraint::Length(gap_h),
            Constraint::Length(queued_h),
            Constraint::Length(status_h),
            Constraint::Length(input_h),
            Constraint::Length(hints_h),
        ])
        .split(area);

    draw_history(f, app, chunks[0]);
    if queued_h > 0 {
        draw_queued(f, app, chunks[2]);
    }
    draw_status(f, app, chunks[3]);
    draw_input(f, app, chunks[4]);
    draw_hints(f, chunks[5]);

    // The completion popup floats just above the input box.
    if app.completion.is_some() && app.modal.is_none() {
        draw_completion(f, app, chunks[4]);
    }
    if app.modal.is_some() {
        draw_modal(f, app);
    }
}

/// Floating popup listing the matching slash commands and references, anchored
/// just above the input box.
fn draw_completion(f: &mut Frame, app: &App, input_area: Rect) {
    let Some(c) = &app.completion else { return };
    let max_rows = 8usize;
    let rows = c.matches.len().min(max_rows);
    if rows == 0 {
        return;
    }
    let height = rows as u16 + 2; // + borders
    let width = input_area.width.clamp(30, 78);
    let y = input_area.y.saturating_sub(height);
    let rect = Rect {
        x: input_area.x,
        y,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            format!(" commands · {} ", c.matches.len()),
            Style::new().fg(ACCENT),
        ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // Scroll the window so the highlighted row stays visible.
    let start = c.selected.saturating_sub(rows.saturating_sub(1));
    let label_w = c
        .matches
        .iter()
        .skip(start)
        .take(rows)
        .map(|&i| app.catalog[i].label.chars().count())
        .max()
        .unwrap_or(0)
        .min(26);

    let mut lines: Vec<Line> = Vec::new();
    for (vi, &mi) in c.matches.iter().enumerate().skip(start).take(rows) {
        let item = &app.catalog[mi];
        let selected = vi == c.selected;
        let marker = if selected { "▸ " } else { "  " };
        let label_style = if selected {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(ACCENT)),
            Span::styled(
                format!("{:<7}", item.kind.tag()),
                Style::new().add_modifier(Modifier::DIM),
            ),
            Span::styled(format!("{:<label_w$}  ", item.label), label_style),
            Span::styled(
                item.description.clone(),
                Style::new().add_modifier(Modifier::DIM),
            ),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inner.inner(Margin::new(1, 0)),
    );
}

fn draw_history(f: &mut Frame, app: &mut App, area: Rect) {
    let para = Paragraph::new(app.scrollback.clone()).wrap(Wrap { trim: false });
    // The wrapped-line total is an O(content) unicode re-measure. Cache it by a
    // cheap content signature so spinner-tick redraws (where nothing changed)
    // don't recompute it; recompute only when scrollback grows, the viewport
    // resizes, or a streamed token extends the in-progress text.
    let sig = (app.scrollback.len(), area.width, app.stream_text.len());
    let total = match app.wrap_cache {
        Some((sb, w, st, cached)) if (sb, w, st) == sig => cached,
        _ => {
            let t = wrapped_count(&app.scrollback, area.width);
            app.wrap_cache = Some((sig.0, sig.1, sig.2, t));
            t
        }
    };
    app.last_total_lines = total as usize;
    app.last_viewport_h = area.height;
    let base = total.saturating_sub(area.height);
    let offset = base.saturating_sub(app.scroll);
    f.render_widget(para.scroll((offset, 0)), area);
}

/// Estimate how many terminal rows the lines occupy once wrapped to `width`.
/// (`Paragraph::line_count` is private in ratatui, so we approximate; chat
/// content is mostly short lines where this is exact.)
fn wrapped_count(lines: &[Line], width: u16) -> u16 {
    let w = (width.max(1)) as usize;
    let mut total: usize = 0;
    for line in lines {
        // Use unicode *display* width so wide glyphs (CJK = 2 cells) count
        // correctly and the latest output isn't clipped.
        let cells: usize = line
            .spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        total += cells.div_ceil(w).max(1);
    }
    total.min(u16::MAX as usize) as u16
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let sep = || Span::styled("  ·  ", dim);
    let s = &app.status;

    // Line 1: the large-language model and the provider.
    let line1 = Line::from(vec![
        Span::styled(s.model.clone(), Style::new().fg(ACCENT)),
        sep(),
        Span::styled(s.provider.clone(), dim),
    ]);

    // Line 2: skills · extensions · knowledge bases, with the context meter
    // right-aligned.
    let full_counts = vec![
        Span::styled(pluralize(s.skills, "skill"), dim),
        sep(),
        Span::styled(pluralize(s.extensions, "extension"), dim),
        sep(),
        Span::styled(pluralize(s.knowledge_bases, "knowledge base"), dim),
    ];
    let compact_counts = vec![
        Span::styled(format!("{} skills", s.skills), dim),
        Span::styled(" · ", dim),
        Span::styled(format!("{} ext", s.extensions), dim),
        Span::styled(" · ", dim),
        Span::styled(format!("{} KB", s.knowledge_bases), dim),
    ];
    let mut right = context_spans(app);
    let mut line2 = if line_width(&full_counts) + line_width(&right) < area.width as usize {
        full_counts
    } else {
        compact_counts
    };
    if line_width(&line2) + line_width(&right) >= area.width as usize {
        right = context_summary_spans(app);
    }
    let occupied = line_width(&line2) + line_width(&right);
    if !right.is_empty() && occupied < area.width as usize {
        line2.push(Span::styled(
            " ".repeat(area.width as usize - occupied),
            dim,
        ));
        line2.extend(right);
    }

    f.render_widget(Paragraph::new(vec![line1, Line::from(line2)]), area);
}

/// A compact context-window meter: `▕████░░░░▏ 42%`. Empty when the limit is
/// unknown (before the first response).
fn context_spans(app: &App) -> Vec<Span<'static>> {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let limit = app.status.context_limit;
    if limit == 0 {
        return Vec::new();
    }
    let pct = (((app.status.used_tokens as f64 / limit as f64) * 100.0).round() as usize).min(100);
    let width = 12usize;
    let filled = (pct * width / 100).min(width);
    // On-brand until the window is nearly full, then warn (no green).
    let color = if pct < 85 { ACCENT } else { Color::Red };
    vec![
        Span::styled("context ", dim),
        Span::styled("▕", dim),
        Span::styled("█".repeat(filled), Style::new().fg(color)),
        Span::styled("░".repeat(width - filled), dim),
        Span::styled(format!("▏ {}%", pct), dim),
    ]
}

fn context_summary_spans(app: &App) -> Vec<Span<'static>> {
    if app.status.context_limit == 0 {
        return Vec::new();
    }
    let pct = (((app.status.used_tokens as f64 / app.status.context_limit as f64) * 100.0).round()
        as usize)
        .min(100);
    vec![Span::styled(
        format!("ctx {pct}%"),
        Style::new().add_modifier(Modifier::DIM),
    )]
}

fn draw_hints(f: &mut Frame, area: Rect) {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let full = "↵ send · ^J newline · / commands · ↑↓ history · ^C stop · type anytime";
    let compact = "↵ send · ^J newline · / commands · ↑↓ history · ^C stop";
    let text = if UnicodeWidthStr::width(full) <= area.width as usize {
        full
    } else {
        compact
    };
    f.render_widget(Paragraph::new(Line::from(Span::styled(text, dim))), area);
}

/// The queued-messages preview: the lines the user typed while the agent was
/// busy. Shown in full (capped) so they're never hidden — they run next, in order.
fn draw_queued(f: &mut Frame, app: &App, area: Rect) {
    let n = app.queued.len();
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("⏳ {n} queued, will run next, in order:"),
        Style::new().fg(ACCENT).add_modifier(Modifier::DIM),
    )));
    let shown = (QUEUED_PREVIEW_MAX as usize).min(n);
    let text_w = area.width.saturating_sub(4).max(8) as usize;
    for q in app.queued.iter().take(shown) {
        // One row per queued message; flatten newlines and clip with an ellipsis.
        let flat: String = q.split_whitespace().collect::<Vec<_>>().join(" ");
        let clipped = clip_cells(&flat, text_w);
        lines.push(Line::from(vec![
            Span::styled("  ↵ ", Style::new().fg(ACCENT)),
            Span::styled(clipped, Style::new().fg(Color::Indexed(250))),
        ]));
    }
    if n > shown {
        lines.push(Line::from(Span::styled(
            format!("  …and {} more", n - shown),
            Style::new()
                .fg(Color::Indexed(244))
                .add_modifier(Modifier::DIM),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Clip a single-line string to at most `max` display cells, adding an ellipsis.
fn clip_cells(s: &str, max: usize) -> String {
    if UnicodeWidthStr::width(s) <= max {
        return s.to_string();
    }
    let budget = max.saturating_sub(1);
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

fn draw_input(f: &mut Frame, app: &mut App, area: Rect) {
    let (border_color, mut title) = if let Some(t) = &app.thinking {
        (
            ACCENT,
            format!(" {} {} ", SPINNER[app.spin % SPINNER.len()], t),
        )
    } else {
        (Color::Indexed(240), String::new())
    };
    // Surface messages typed while the agent is busy (they run next, in order).
    if !app.queued.is_empty() {
        title.push_str(&format!(" {} queued ↵ ", app.queued.len()));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color))
        .title(Span::styled(title, Style::new().fg(ACCENT)));
    let inner = block.inner(area);
    app.last_input_inner = Some(inner);
    f.render_widget(block, area);

    // Soft-wrap each logical line to the inner text width (prompt = 2 cells) so
    // overflowing text flows onto the next row instead of being clipped.
    let text_w = inner.width.saturating_sub(2).max(1) as usize;
    app.last_input_text_width = text_w;
    let ghost = if app.completion.is_some() {
        None
    } else {
        app.ghost()
    };
    let mut lines: Vec<Line> = Vec::new();
    if app.input.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("❯ ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            // A clearly-greyed placeholder so it doesn't read as real input.
            Span::styled(
                "Send a message…",
                Style::new()
                    .fg(Color::Indexed(244))
                    .add_modifier(Modifier::DIM),
            ),
        ]));
    } else {
        for (idx, row) in input_visual_rows(&app.input, text_w)
            .into_iter()
            .enumerate()
        {
            let prefix = if idx == 0 {
                Span::styled("❯ ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("  ")
            };
            let mut spans = vec![prefix];
            spans.extend(input_row_spans(app, row.start, row.end));
            lines.push(Line::from(spans));
        }
        if let (Some(g), Some(last)) = (ghost, lines.last_mut()) {
            last.spans.push(Span::styled(
                g.to_string(),
                Style::new().add_modifier(Modifier::DIM),
            ));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);

    if app.modal.is_none() {
        let rows = input_visual_rows(&app.input, text_w);
        let (row, col) = visual_position(&rows, &app.input, app.cursor);
        let x = inner.x + 2 + col as u16; // 2 = prompt width
        let y = inner.y + row as u16;
        f.set_cursor_position((
            x.min(inner.x + inner.width.saturating_sub(1)),
            y.min(inner.y + inner.height.saturating_sub(1)),
        ));
    }
}

fn input_row_spans(app: &App, start: usize, end: usize) -> Vec<Span<'static>> {
    let Some((sel_start, sel_end)) = app.selection_range() else {
        return vec![Span::raw(input_slice(&app.input, start, end).to_string())];
    };
    let hi_start = sel_start.max(start);
    let hi_end = sel_end.min(end);
    if hi_start >= hi_end {
        return vec![Span::raw(input_slice(&app.input, start, end).to_string())];
    }
    let mut spans = Vec::new();
    if start < hi_start {
        spans.push(Span::raw(
            input_slice(&app.input, start, hi_start).to_string(),
        ));
    }
    spans.push(Span::styled(
        input_slice(&app.input, hi_start, hi_end).to_string(),
        Style::new().fg(Color::Black).bg(ACCENT),
    ));
    if hi_end < end {
        spans.push(Span::raw(input_slice(&app.input, hi_end, end).to_string()));
    }
    spans
}

fn input_slice(input: &str, start: usize, end: usize) -> &str {
    input.get(start..end).unwrap_or_default()
}

#[derive(Clone, Copy)]
struct InputRow {
    start: usize,
    end: usize,
}

fn input_visual_rows(input: &str, width: usize) -> Vec<InputRow> {
    let w = width.max(1);
    let mut rows = Vec::new();
    let mut line_start = 0usize;
    for logical in input.split('\n') {
        let line_end = line_start + logical.len();
        let mut row_start = line_start;
        let mut row_width = 0usize;
        if logical.is_empty() {
            rows.push(InputRow {
                start: line_start,
                end: line_start,
            });
        } else {
            for (rel, ch) in logical.char_indices() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                if row_width + cw > w && row_start < line_start + rel {
                    rows.push(InputRow {
                        start: row_start,
                        end: line_start + rel,
                    });
                    row_start = line_start + rel;
                    row_width = 0;
                }
                row_width += cw;
            }
            rows.push(InputRow {
                start: row_start,
                end: line_end,
            });
        }
        line_start = line_end.saturating_add(1);
    }
    if rows.is_empty() {
        rows.push(InputRow { start: 0, end: 0 });
    }
    rows
}

fn visual_position(rows: &[InputRow], input: &str, cursor: usize) -> (usize, usize) {
    for (idx, row) in rows.iter().enumerate() {
        if cursor >= row.start && cursor <= row.end {
            let col = UnicodeWidthStr::width(input_slice(input, row.start, cursor));
            return (idx, col);
        }
    }
    let last_idx = rows.len().saturating_sub(1);
    let last = rows
        .last()
        .copied()
        .unwrap_or(InputRow { start: 0, end: 0 });
    (
        last_idx,
        UnicodeWidthStr::width(input_slice(input, last.start, last.end)),
    )
}

fn byte_at_visual_cell(rows: &[InputRow], input: &str, row: usize, col: usize) -> usize {
    let row = rows
        .get(row)
        .or_else(|| rows.last())
        .copied()
        .unwrap_or(InputRow { start: 0, end: 0 });
    let text = input_slice(input, row.start, row.end);
    let mut current_col = 0usize;
    for (rel, ch) in text.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col <= current_col {
            return row.start + rel;
        }
        if col < current_col + cw {
            return if col.saturating_sub(current_col) <= cw / 2 {
                row.start + rel
            } else {
                row.start + rel + ch.len_utf8()
            };
        }
        current_col += cw;
    }
    row.end
}

/// Break a logical line into display rows at `width` cells, like a textarea
/// (char wrap, not word wrap). Empty input yields one empty row.
fn wrap_cells(s: &str, width: usize) -> Vec<String> {
    let w = width.max(1);
    let mut rows = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if cur_w + cw > w && !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w += cw;
    }
    rows.push(cur);
    rows
}

/// Total wrapped row count for the whole (multi-line) input buffer.
fn input_rows(input: &str, text_w: u16) -> u16 {
    let w = text_w.max(1) as usize;
    let mut n = 0u16;
    for logical in input.split('\n') {
        n = n.saturating_add(wrap_cells(logical, w).len() as u16);
    }
    n.max(1)
}

fn draw_modal(f: &mut Frame, app: &App) {
    let Some(modal) = &app.modal else { return };
    let area = f.area();
    let w = 64u16.min(area.width.saturating_sub(4));
    let h = (modal.options.len() as u16) + 4;
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            " Tool approval ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines = vec![Line::from(Span::styled(
        modal
            .prompt
            .clone()
            .unwrap_or_else(|| "Biorouter would like to call the above tool.".to_string()),
        Style::default(),
    ))];
    lines.push(Line::default());
    for (i, (label, desc)) in modal.options.iter().enumerate() {
        let selected = i == modal.selected;
        let marker = if selected { "❯ " } else { "  " };
        let style = if selected {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(ACCENT)),
            Span::styled(format!("{:<14}", label), style),
            Span::styled(
                (*desc).to_string(),
                Style::new().add_modifier(Modifier::DIM),
            ),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inner.inner(Margin::new(1, 0)),
    );
}

fn line_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

// ── content helpers ──────────────────────────────────────────────────────────

fn greeting_into(app: &mut App) {
    app.push_blank();
    for line in crate::session::output::BIOROUTER_ASCII {
        app.push_raw(*line, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD));
    }
    app.push_blank();
    // Tagline with the running version (CARGO_PKG_VERSION = the workspace
    // version, so it tracks the release automatically).
    app.push_line(Line::from(vec![
        Span::styled(
            "Biorouter: integrated biomedical research environment",
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            concat!("  ·  v", env!("CARGO_PKG_VERSION")),
            Style::new().add_modifier(Modifier::DIM),
        ),
    ]));
    if !app.status.workdir.is_empty() {
        app.push_line(Line::from(vec![
            Span::styled(
                "working directory: ",
                Style::new().add_modifier(Modifier::DIM),
            ),
            Span::styled(app.status.workdir.clone(), Style::new().fg(ACCENT)),
        ]));
    }
    app.push_raw(
        "Enter your instructions, or type /help for commands",
        Style::new().add_modifier(Modifier::DIM),
    );
}

/// List skill references using the same directories as the desktop slash popup.
fn list_skills() -> Vec<String> {
    crate::session::completion::list_skill_reference_names()
}

/// ⚠ **User-installed skills only** (#77). What ships with Biorouter is offered
/// as Contexts, managed in Settings -> Chat, and the GUI already excludes them
/// from its chip. Counting them here made the two surfaces disagree about the
/// same word — exactly the drift the capability exclusion below was written to
/// fix for extensions.
///
/// ⚠ **`is_shipped_entry_name`, not `is_builtin_skill_name`.** The list this
/// filters is [`list_skills`], which names a bundle by its *directory* and
/// never by a member (`completion.rs`) — so the skill-only predicate would let
/// `KNOWLEDGE_BUNDLE` through as one skill the user installed, on every
/// install. It is the same predicate the seeder, the reset path and the count
/// in the GUI use, rather than a second list that would drift from them.
///
/// Split from [`count_skills`] so the exclusion can be exercised on a list this
/// test can name. Counting the real directory cannot check it: on a machine with
/// no user skills the filtered and unfiltered counts are both what the seeder
/// left, so a test of the whole function passes with the filter deleted.
fn count_user_skills_in(names: &[String]) -> usize {
    names
        .iter()
        .filter(|name| !biorouter::agents::skills_extension::is_shipped_entry_name(name))
        .count()
}

fn count_skills() -> usize {
    count_user_skills_in(&list_skills())
}

/// The six foundational built-ins surfaced as their own "Capabilities" section
/// in the GUI and intentionally excluded from the extension list/count there
/// (see ui/desktop/.../capabilities/capabilities.ts). The CLI status line was
/// counting them as extensions, so it showed e.g. 11 where the GUI shows 5.
/// Keys are normalized like the GUI's `nameToKey` (whitespace removed, lowercased).
const CAPABILITY_KEYS: [&str; 6] = [
    "developer",
    "extensionmanager",
    "skills",
    "todo",
    "memory",
    "knowledge",
];

fn name_to_key(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// Enabled extensions, excluding the foundational capabilities — matches the
/// count the GUI shows.
fn count_non_capability_extensions() -> usize {
    biorouter::config::get_enabled_extensions()
        .iter()
        .filter(|ext| !CAPABILITY_KEYS.contains(&name_to_key(&ext.name()).as_str()))
        .count()
}

/// Pluralize a noun for a count: "1 skill", "0 skills", "2 skills".
fn pluralize(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

/// One extension entry in the TUI completion popup.
///
/// Issue #65, third site: the label keeps the extension's display name (with
/// its space — that is what the user is picking and filtering on) while the
/// INSERTED reference is whatever survives the agent's extractor intact, which
/// for a spaced name is the canonical `<biorouter-ref …>` tag rather than a
/// compacted `/ext:` marker. Shares `completion::extension_marker` with the
/// rustyline completer so the two front-ends cannot drift.
fn extension_completion_item(name: &str, description: &str, builtin: bool) -> app::CompletionItem {
    app::CompletionItem {
        label: name.to_string(),
        description: description.to_string(),
        insert: format!("{} ", super::completion::extension_marker(name)),
        filter: if builtin {
            format!("extension builtin {}", name.to_lowercase())
        } else {
            format!("extension {}", name.to_lowercase())
        },
        kind: app::CompletionKind::Extension,
    }
}

/// One skill entry in the TUI completion popup.
///
/// Sibling of [`extension_completion_item`]: the label is what the user picks
/// and filters on, while the INSERTED reference is whatever survives the
/// agent's extractor intact — for a name with a space, the canonical
/// `<biorouter-ref …>` tag rather than a `/skill:` marker that would reach the
/// resolver truncated at the first word (issue #65).
fn skill_completion_item(name: &str) -> app::CompletionItem {
    app::CompletionItem {
        label: name.to_string(),
        description: "Ask the agent to use this skill".to_string(),
        insert: format!(
            "{} ",
            biorouter::agents::resource_refs::reference_marker(
                biorouter::agents::resource_refs::RefKind::Skill,
                name,
            )
        ),
        filter: format!("skill {}", name.to_lowercase()),
        kind: app::CompletionKind::Skill,
    }
}

/// One knowledge-base entry in the TUI completion popup.
///
/// A base is picked by name and resolved by id, so the label carries the id
/// `kb_search` takes and the filter matches on both.
fn knowledge_completion_item(id: &str, display_name: &str) -> app::CompletionItem {
    app::CompletionItem {
        label: id.to_string(),
        description: format!("Focus knowledge base · {display_name}"),
        insert: format!(
            "{} ",
            biorouter::agents::resource_refs::reference_marker(
                biorouter::agents::resource_refs::RefKind::KnowledgeBase,
                id,
            )
        ),
        filter: format!(
            "kb knowledge {} {}",
            id.to_lowercase(),
            display_name.to_lowercase()
        ),
        kind: app::CompletionKind::Knowledge,
    }
}

/// Build the completion catalog shown in the slash-command popup: the TUI slash
/// commands, plus references to skills, extensions, and knowledge bases.
fn build_catalog(_session: &CliSession) -> Vec<app::CompletionItem> {
    use app::{CompletionItem, CompletionKind};
    let mut items: Vec<CompletionItem> = Vec::new();

    let cmd = |name: &str, desc: &str| CompletionItem {
        label: name.to_string(),
        description: desc.to_string(),
        insert: format!("{} ", name),
        filter: name.trim_start_matches('/').to_lowercase(),
        kind: CompletionKind::Command,
    };
    items.push(cmd("/help", "Show commands and shortcuts"));
    items.push(cmd("/compact", "Condense the chat to reclaim context"));
    items.push(cmd("/clear", "Clear the chat"));
    items.push(cmd("/exit", "Leave the chat"));

    for name in list_skills() {
        items.push(skill_completion_item(&name));
    }

    // Extensions enabled for this chat plus bundled built-ins that can be
    // enabled deterministically when selected.
    let compact_extension_canonicals = ["agent_drafter", "autovisualiser", "Extension Manager"];
    for ext in biorouter::config::get_enabled_extensions() {
        let name = ext.name();
        if compact_extension_canonicals
            .iter()
            .any(|canonical| name.eq_ignore_ascii_case(canonical))
        {
            continue;
        }
        items.push(extension_completion_item(
            &name,
            "Ask the agent to use this extension",
            false,
        ));
    }
    for name in biorouter_mcp::BUILTIN_EXTENSIONS.keys() {
        if compact_extension_canonicals.contains(name) {
            continue;
        }
        items.push(extension_completion_item(
            name,
            "Ask the agent to use this built-in extension",
            true,
        ));
    }
    items.push(CompletionItem {
        label: "agentdrafter".to_string(),
        description: "Ask the agent to use Agent Drafter".to_string(),
        insert: "/ext:agentdrafter ".to_string(),
        filter: "extension builtin agentdrafter agent drafter".to_string(),
        kind: CompletionKind::Extension,
    });
    items.push(CompletionItem {
        label: "autovisualizer".to_string(),
        description: "Ask the agent to use Auto Visualiser".to_string(),
        insert: "/ext:autovisualizer ".to_string(),
        filter: "extension builtin autovisualizer auto visualizer autovisualiser auto visualiser"
            .to_string(),
        kind: CompletionKind::Extension,
    });
    items.push(CompletionItem {
        label: "extensionmanager".to_string(),
        description: "Ask the agent to use Extension Manager".to_string(),
        insert: "/ext:extensionmanager ".to_string(),
        filter: "extension builtin extensionmanager extension manager".to_string(),
        kind: CompletionKind::Extension,
    });

    // Knowledge bases visible to the agent.
    if let Ok(svc) = biorouter::knowledge::service::KnowledgeService::new_default() {
        if let Ok(bases) = svc.list_bases() {
            let hidden = svc.get_hidden_persisted().unwrap_or_default();
            for b in bases.into_iter().filter(|b| !hidden.contains(&b.id)) {
                items.push(knowledge_completion_item(&b.id, &b.name));
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert((item.kind.tag(), item.insert.clone(), item.label.clone())))
        .collect()
}

/// Surface the same checks `biorouter doctor` runs — missing required
/// prerequisites and an available update — so the interactive session reflects
/// them too. The update probe is bounded (2s) so it never stalls startup.
async fn push_startup_notices(app: &mut App) {
    let missing: Vec<_> = biorouter::system::check_all()
        .into_iter()
        .filter(|d| d.required && !d.installed)
        .collect();
    if !missing.is_empty() {
        app.push_blank();
        for d in &missing {
            app.push_line(Line::from(vec![
                Span::styled("⚠ ", Style::new().fg(Color::Yellow)),
                Span::styled(
                    format!("{} not found.", d.display_name),
                    Style::new().fg(Color::Yellow),
                ),
                Span::styled(
                    "  Run `biorouter doctor` to set up prerequisites".to_string(),
                    Style::new().add_modifier(Modifier::DIM),
                ),
            ]));
        }
    }

    if let Ok(Some(u)) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        biorouter::system::check_for_update(),
    )
    .await
    {
        if u.update_available {
            app.push_blank();
            app.push_line(Line::from(vec![
                Span::styled("↑ ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("Biorouter {} is available", u.latest),
                    Style::new().fg(ACCENT),
                ),
                Span::styled(
                    format!(
                        "  (you have {}). Install the latest release to upgrade",
                        u.current
                    ),
                    Style::new().add_modifier(Modifier::DIM),
                ),
            ]));
        }
    }
}

fn help_into(app: &mut App) {
    app.push_blank();
    app.push_raw(
        "Commands",
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    for (cmd, desc) in [
        ("/help, /?", "show this help"),
        ("/compact", "condense the chat to reclaim context"),
        ("/clear", "clear the chat"),
        (
            "/goal <condition>",
            "keep working until the condition is met",
        ),
        (
            "/loop <interval> <prompt>",
            "run a prompt on an interval (5m, 2h…)",
        ),
        (
            "/schedule <spec> <prompt>",
            "recurring prompt: 5m, @daily, or cron",
        ),
        ("/exit, /quit", "leave the chat"),
    ] {
        app.push_line(Line::from(vec![
            Span::styled(format!("  {:<14}", cmd), Style::new().fg(ACCENT)),
            Span::styled(desc.to_string(), Style::new().add_modifier(Modifier::DIM)),
        ]));
    }
    app.push_raw(
        "Navigation",
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    for (k, d) in [
        ("↵", "send · ⇧↵ or ^J for a newline"),
        ("↑ / ↓", "history · scroll with PageUp/Down or the mouse"),
        ("^C", "clear the line, or quit when empty"),
    ] {
        app.push_line(Line::from(vec![
            Span::styled(format!("  {:<14}", k), Style::new().fg(ACCENT)),
            Span::styled(d.to_string(), Style::new().add_modifier(Modifier::DIM)),
        ]));
    }
}

fn status_from_session(_session: &CliSession) -> StatusInfo {
    let config = biorouter::config::Config::global();

    // Databases = knowledge bases visible to the agent (total minus hidden).
    let databases = biorouter::knowledge::service::KnowledgeService::new_default()
        .ok()
        .and_then(|svc| {
            let bases = svc.list_bases().ok()?;
            let hidden = svc.get_hidden_persisted().unwrap_or_default();
            Some(bases.iter().filter(|b| !hidden.contains(&b.id)).count())
        })
        .unwrap_or(0);

    StatusInfo {
        provider: config.get_biorouter_provider().unwrap_or_default(),
        model: config.get_biorouter_model().unwrap_or_default(),
        workdir: std::env::current_dir()
            .ok()
            .map(|p| shorten_home(&p))
            .unwrap_or_default(),
        skills: count_skills(),
        extensions: count_non_capability_extensions(),
        knowledge_bases: databases,
        used_tokens: 0,
        context_limit: 0,
    }
}

/// Replace the home-dir prefix with `~` for a tidier path display.
fn shorten_home(p: &std::path::Path) -> String {
    if let Ok(home) = etcetera::home_dir() {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}

/// Refresh the context-window meter (tokens used / model limit) from the live
/// session. Best-effort: failures leave the meter hidden.
async fn refresh_context(session: &CliSession, app: &mut App) {
    if let Ok(provider) = session.agent.provider().await {
        app.status.context_limit = provider.get_model_config().context_limit();
    }
    if let Ok(meta) = session.get_session().await {
        app.status.used_tokens = meta.total_tokens.unwrap_or(0) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn buffer_text(app: &mut App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn renders_greeting_and_input_without_panic() {
        let mut app = App::new(StatusInfo {
            provider: "versa_azure".into(),
            model: "gpt-5.2".into(),
            skills: 3,
            extensions: 5,
            knowledge_bases: 2,
            ..Default::default()
        });
        greeting_into(&mut app);
        app.push_user("hello there");
        let text = buffer_text(&mut app, 90, 30);
        assert!(text.contains("Biorouter"));
        assert!(text.contains("hello there"));
        assert!(text.contains("gpt-5.2"));
        assert!(text.contains("extensions"));
        assert!(text.contains("knowledge bases"));
        assert!(text.contains("Send a message"));
    }

    #[test]
    fn narrow_status_keeps_counts_and_context_separate() {
        let mut app = App::new(StatusInfo {
            provider: "openai".into(),
            model: "gpt-4.1".into(),
            skills: 8,
            extensions: 1,
            knowledge_bases: 0,
            used_tokens: 1_000,
            context_limit: 128_000,
            ..Default::default()
        });

        let text = buffer_text(&mut app, 72, 24);
        assert!(text.contains("8 skills · 1 ext · 0 KB"), "{text}");
        assert!(text.contains("context"), "{text}");
        assert!(text.contains("^C stop"), "{text}");
        assert!(!text.contains("type a"), "{text}");
        assert!(!text.contains("KBcontext"), "{text}");
    }

    #[test]
    fn renders_markdown_message() {
        let mut app = App::new(StatusInfo::default());
        let msg = biorouter::conversation::message::Message::assistant()
            .with_text("# Title\n**bold** and `code`\n- a bullet");
        app.push_message(&msg, false);
        let text = buffer_text(&mut app, 80, 24);
        assert!(text.contains("Title"));
        assert!(text.contains("bold"));
        assert!(text.contains("bullet"));
    }

    #[test]
    fn renders_browser_links_for_resources_and_launched_apps() {
        use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Meta, RawResource};

        let mut report = RawResource::new("https://example.test/report.html", "report");
        report.title = Some("Study report".to_string());
        let mut result = CallToolResult::success(vec![
            Content::resource_link(report),
            Content::text("assistant-only duplicate URL")
                .with_audience(vec![rmcp::model::Role::Assistant]),
        ]);
        result.meta = Some(Meta(
            serde_json::from_value(serde_json::json!({
                "biorouter/app-path": "/apps/cohort-viewer/"
            }))
            .unwrap(),
        ));
        let message = biorouter::conversation::message::Message::assistant()
            .with_tool_response("tool-1", Ok(result));

        let mut app = App::new(StatusInfo::default());
        app.push_message(&message, false);
        let next_tool = biorouter::conversation::message::Message::assistant().with_tool_request(
            "tool-2",
            Ok(CallToolRequestParams {
                name: "developer__shell".into(),
                arguments: None,
                meta: None,
                task: None,
            }),
        );
        app.push_message(&next_tool, false);
        let rendered = app
            .scrollback
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("Artifact: Study report"), "{rendered}");
        assert!(
            rendered.contains("https://example.test/report.html"),
            "{rendered}"
        );
        assert!(
            rendered.contains("Artifact: App: cohort-viewer"),
            "{rendered}"
        );
        assert!(rendered.contains("/apps/cohort-viewer/"), "{rendered}");
        assert!(
            !rendered.contains("assistant-only duplicate URL"),
            "{rendered}"
        );
    }

    #[test]
    fn renders_permission_modal() {
        let mut app = App::new(StatusInfo::default());
        app.modal = Some(PermissionModal {
            prompt: None,
            options: vec![("Allow", "allow once"), ("Deny", "deny")],
            selected: 0,
        });
        let text = buffer_text(&mut app, 80, 24);
        assert!(text.contains("Tool approval"));
        assert!(text.contains("Allow"));
    }

    #[test]
    fn paste_inserts_multiline_without_submitting() {
        let mut app = App::new(StatusInfo::default());
        app.insert_char('a');
        app.paste("line1\r\nline2");
        // \r stripped, \n kept literal (no submit), inserted at the cursor.
        assert_eq!(app.input, "aline1\nline2");
        assert_eq!(app.cursor, app.input.len());
    }

    #[test]
    fn paste_file_uri_list_inserts_local_paths() {
        let mut app = App::new(StatusInfo::default());
        app.paste("file:///Users/wgu/Desktop/biorouter/README.md\n");
        assert_eq!(app.input, "/Users/wgu/Desktop/biorouter/README.md");
    }

    #[test]
    fn paste_file_uri_list_quotes_paths_with_spaces() {
        let mut app = App::new(StatusInfo::default());
        app.paste(
            "# dragged files\nfile:///Users/wgu/Desktop/first%20file.pdf\nfile://localhost/Users/wgu/Desktop/second.txt\n",
        );
        assert_eq!(
            app.input,
            "'/Users/wgu/Desktop/first file.pdf' /Users/wgu/Desktop/second.txt"
        );
    }

    #[test]
    fn completion_popup_filters_and_accepts() {
        use app::{CompletionItem, CompletionKind};
        let mut app = App::new(StatusInfo::default());
        app.set_catalog(vec![
            CompletionItem {
                label: "/help".into(),
                description: "help".into(),
                insert: "/help ".into(),
                filter: "help".into(),
                kind: CompletionKind::Command,
            },
            CompletionItem {
                label: "/compact".into(),
                description: "compact".into(),
                insert: "/compact ".into(),
                filter: "compact".into(),
                kind: CompletionKind::Command,
            },
        ]);
        // Typing "/" opens the popup with everything.
        app.insert_char('/');
        app.refresh_completion();
        assert!(app.completion_active());
        assert_eq!(app.completion.as_ref().unwrap().matches.len(), 2);
        // Narrowing to "/he" filters to one and accepting fills the input.
        app.insert_char('h');
        app.insert_char('e');
        app.refresh_completion();
        assert_eq!(app.completion.as_ref().unwrap().matches.len(), 1);
        assert!(app.completion_accept());
        assert_eq!(app.input, "/help ");
        assert!(!app.completion_active());

        app.input.clear();
        app.cursor = 0;
        app.input = "please use /he".to_string();
        app.cursor = app.input.len();
        app.refresh_completion();
        assert!(app.completion_accept());
        assert_eq!(app.input, "please use /help ");

        // The popup renders without panicking.
        let text = buffer_text(
            &mut {
                let mut a = App::new(StatusInfo::default());
                a.set_catalog(vec![CompletionItem {
                    label: "/help".into(),
                    description: "show help".into(),
                    insert: "/help ".into(),
                    filter: "help".into(),
                    kind: CompletionKind::Command,
                }]);
                a.insert_char('/');
                a.refresh_completion();
                a
            },
            80,
            24,
        );
        assert!(text.contains("commands"));
    }

    #[test]
    fn long_input_wraps_and_grows_the_box() {
        // A single long line (no newlines) must wrap to multiple rows and the
        // box must report >1 text row — i.e. overflow is shown, not clipped.
        let long = "x".repeat(200);
        assert!(input_rows(&long, 40) >= 5);
        assert_eq!(wrap_cells(&long, 40).len(), 5);
        // Empty/blank cases stay a single row.
        assert_eq!(input_rows("", 40), 1);
    }

    #[test]
    fn streaming_preview_renders_then_commits() {
        let mut app = App::new(StatusInfo::default());
        app.stream_delta(Some("r1".into()), "Hello ");
        app.stream_delta(Some("r1".into()), "world");
        // The in-progress preview is visible mid-stream.
        let mid = buffer_text(&mut app, 80, 24);
        assert!(mid.contains("Hello world"));
        // Committing returns the whole text and leaves it in the scrollback.
        assert_eq!(app.stream_commit(), Some("Hello world".to_string()));
        assert!(app.stream_start.is_none() && app.stream_text.is_empty());
        let after = buffer_text(&mut app, 80, 24);
        assert!(after.contains("Hello world"));
    }

    #[test]
    fn renders_markdown_table_as_box() {
        let mut app = App::new(StatusInfo::default());
        let msg = biorouter::conversation::message::Message::assistant()
            .with_text("| Area | High |\n|------|------|\n| Bay | 72 |\n| Inland | 78 |");
        app.push_message(&msg, false);
        let text = buffer_text(&mut app, 80, 24);
        // Box-drawing borders + header + cells present.
        assert!(text.contains('┌') && text.contains('┼') && text.contains('└'));
        assert!(text.contains("Area") && text.contains("Inland") && text.contains("78"));
    }

    #[test]
    fn input_editing_and_ghost() {
        let mut app = App::new(StatusInfo::default());
        for c in "/he".chars() {
            app.insert_char(c);
        }
        assert_eq!(app.ghost(), Some("lp"));
        app.accept_ghost();
        assert_eq!(app.input, "/help ");
        app.backspace();
        assert_eq!(app.input, "/help");
    }

    #[test]
    fn selection_deletes_and_replaces_text() {
        let mut app = App::new(StatusInfo::default());
        app.paste("alpha beta gamma");
        app.set_cursor(6, false);
        app.set_cursor(10, true);
        assert_eq!(app.selected_text().as_deref(), Some("beta"));
        app.backspace();
        assert_eq!(app.input, "alpha  gamma");
        assert_eq!(app.cursor, 6);

        app.set_cursor(6, false);
        app.set_cursor(7, true);
        app.insert_char('X');
        assert_eq!(app.input, "alpha Xgamma");
    }

    #[test]
    fn mouse_position_maps_to_wrapped_input_cursor() {
        let mut app = App::new(StatusInfo::default());
        app.paste("abcdef");
        app.last_input_inner = Some(Rect {
            x: 10,
            y: 5,
            width: 8,
            height: 3,
        });
        app.last_input_text_width = 3;

        assert_eq!(input_cursor_at_mouse(&app, 12, 5), Some(0));
        assert_eq!(input_cursor_at_mouse(&app, 14, 5), Some(2));
        assert_eq!(input_cursor_at_mouse(&app, 12, 6), Some(3));
        assert_eq!(input_cursor_at_mouse(&app, 15, 6), Some(6));
    }

    // B3: queued messages are rendered in full (not just a count).
    #[test]
    fn queued_messages_shown_in_full() {
        let mut app = App::new(StatusInfo::default());
        app.queued.push_back("first queued task".to_string());
        app.queued.push_back("second queued task".to_string());
        let text = buffer_text(&mut app, 80, 24);
        assert!(text.contains("2 queued"), "header missing:\n{text}");
        assert!(
            text.contains("first queued task"),
            "queued #1 not shown:\n{text}"
        );
        assert!(
            text.contains("second queued task"),
            "queued #2 not shown:\n{text}"
        );
    }

    // B3: when more than the cap are queued, the "+N more" line is allocated room
    // and rendered (not clipped).
    #[test]
    fn queued_overflow_shows_more_line() {
        let mut app = App::new(StatusInfo::default());
        for i in 0..(QUEUED_PREVIEW_MAX + 2) {
            app.queued.push_back(format!("queued task {i}"));
        }
        let text = buffer_text(&mut app, 80, 30);
        let more = (QUEUED_PREVIEW_MAX + 2) as usize - QUEUED_PREVIEW_MAX as usize;
        assert!(
            text.contains(&format!("and {more} more")),
            "the +N more overflow line should be visible:\n{text}"
        );
    }

    // B2: an older tool result collapses once a newer tool call starts; the
    // current tool result stays expanded.
    #[test]
    fn older_tool_results_collapse() {
        use rmcp::model::{CallToolRequestParams, CallToolResult, Content};
        let req = |id: &str| {
            biorouter::conversation::message::Message::assistant().with_tool_request(
                id,
                Ok(CallToolRequestParams {
                    name: "developer__shell".into(),
                    arguments: None,
                    meta: None,
                    task: None,
                }),
            )
        };
        let resp = |id: &str, body: &str| {
            biorouter::conversation::message::Message::user().with_tool_response(
                id,
                Ok(CallToolResult {
                    content: vec![Content::text(body.to_string())],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            )
        };
        let mut app = App::new(StatusInfo::default());
        app.push_message(&req("a"), false);
        app.push_message(&resp("a", "AAA-1\nAAA-2\nAAA-3\nAAA-4"), false);
        // New tool call → the previous result collapses.
        app.push_message(&req("b"), false);
        app.push_message(&resp("b", "BBB-1\nBBB-2"), false);

        let text = buffer_text(&mut app, 80, 40);
        assert!(
            text.contains("tool output collapsed"),
            "expected a collapse summary line:\n{text}"
        );
        assert!(
            !text.contains("AAA-2"),
            "older tool output should be collapsed away:\n{text}"
        );
        assert!(
            text.contains("BBB-1"),
            "current tool output should stay visible:\n{text}"
        );
    }
    /// Issue #65, third site (neither the desktop composer nor the rustyline
    /// completer covers this one): the TUI's own `/`-popup inserts the
    /// reference for the extension the user picked.
    ///
    /// `chatrecall` is registered under the display string `"Chat Recall"`
    /// (`agents::chatrecall_extension::EXTENSION_NAME`) and is NOT one of the
    /// three names `compact_extension_canonicals` swaps for an alias, so
    /// nothing rescues it here either. #60 inserted `/ext:ChatRecall`, which
    /// resolved only because `/ext:` consumers re-normalise whitespace away;
    /// the insert now carries the name exactly, which is the guarantee skills
    /// need and extensions may as well have. The label and the filter keep the
    /// space either way — that is what the user reads and types against.
    #[test]
    fn extension_popup_carries_a_spaced_name_intact() {
        use biorouter::agents::resource_refs::{extracted_refs, RefKind};

        let item = extension_completion_item("Chat Recall", "Ask the agent", false);
        assert_eq!(item.label, "Chat Recall");
        assert_eq!(item.filter, "extension chat recall");
        assert_eq!(
            extracted_refs(RefKind::Extension, &item.insert),
            vec!["Chat Recall".to_string()]
        );

        // A name the compact marker can carry keeps it: `_` and `-` are part of
        // a user extension's key, and a terminal has no chip to render a tag
        // into.
        let item = extension_completion_item("my_tool-v2", "Ask the agent", false);
        assert_eq!(item.insert, "/ext:my_tool-v2 ");
    }

    /// ⚠ **The status line counts what the user installed, not what shipped**
    /// (#77). The five Contexts are seeded into the same directory on every
    /// start, so before this exclusion the terminal reported "5 skills" on a
    /// machine with none and disagreed with the GUI chip about the same word —
    /// the drift `CAPABILITY_KEYS` already fixes for extensions.
    ///
    /// Nothing asserted it. `count_skills` reads the real config directory, and
    /// on the common machine (contexts seeded, no user skills) the filtered and
    /// unfiltered counts are both what the seeder left, so deleting the filter
    /// changes no number a test of that function could see.
    #[test]
    fn the_status_line_counts_only_user_installed_skills() {
        // ⚠ Exactly the shape `list_skills` produces: flat skills by their own
        // name, and a bundle by its DIRECTORY name — never by a member. That is
        // what makes `is_shipped_entry_name` the right predicate here, and a
        // fixture of skill names only could not tell the two apart.
        let shipped: Vec<String> = biorouter::agents::skills_extension::context_ids()
            .map(str::to_string)
            .collect();
        let names: Vec<String> = shipped
            .iter()
            .cloned()
            .chain(["ggplot".to_string(), "single-cell".to_string()])
            .collect();

        assert_eq!(count_user_skills_in(&names), 2);
        assert!(
            shipped.contains(&biorouter::agents::skills_extension::KNOWLEDGE_BUNDLE.to_string()),
            "the knowledge bundle left the Context list; this test no longer covers a bundle row"
        );
        assert!(
            shipped.len() >= 5,
            "the Context list emptied out; this test would then pass on any input"
        );
        assert_eq!(count_user_skills_in(&shipped), 0);
        // A bundle MEMBER can reach this list from another surface, and is also
        // not the user's.
        assert_eq!(
            count_user_skills_in(&["knowledge-lint".to_string(), "update-soul".to_string()]),
            0
        );
        // The trap: a user skill whose name merely resembles a Context's.
        assert_eq!(
            count_user_skills_in(&["about-biorouter-notes".to_string()]),
            1
        );
    }
}
