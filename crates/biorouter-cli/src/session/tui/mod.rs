//! Full-screen TUI front-end for interactive sessions — a Claude-Code-style
//! layout with a scrolling history pane and a pinned bottom input box.
//!
//! The agent's streaming `reply()` events are consumed inline (so the borrow of
//! `agent` and the mutation of `messages` stay disjoint, exactly like the
//! classic readline loop) and rendered into a ratatui scrollback buffer.

mod app;
mod decisions;
mod subagents;

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
use biorouter::session::SessionManager;

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
    app.set_catalog(build_catalog(session).await);
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

    // The subagent pane reads the roster in this process: the TUI's children
    // run here, and no daemon knows about them.
    let session_manager = session.agent.config.session_manager.clone();
    let parent_id = session.session_id.clone();
    let debug = session.debug;
    // A resumed chat shows the subagents it already has.
    refresh_subagents(&session_manager, &parent_id, &mut app).await;

    if let Some(prompt) = initial_prompt {
        submit(session, &mut app, &mut tui, &mut rx, prompt).await?;
        drain_queue(session, &mut app, &mut tui, &mut rx).await?;
    }

    // Children keep working while the parent is idle, so the idle loop wakes on
    // a timer as well as on input. The timer redraws only when the roster or an
    // open child's chat changed, so an idle screen stays still.
    let mut refresh = tokio::time::interval(subagents::REFRESH_INTERVAL);
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut redraw = true;
    while !app.should_quit {
        if redraw {
            tui.draw(&mut app)?;
        }
        redraw = true;
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else {
                    break; // event reader closed
                };
                match event {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if let Some(submission) = on_key(&mut app, key) {
                            submit(session, &mut app, &mut tui, &mut rx, submission).await?;
                            // Anything typed while that turn streamed runs next, in order.
                            drain_queue(session, &mut app, &mut tui, &mut rx).await?;
                        }
                        open_pending_child(&session_manager, debug, &mut app).await;
                        open_pending_decision_calls(&session_manager, &parent_id, &mut app).await;
                    }
                    Event::Paste(s) => on_paste(&mut app, &s),
                    Event::Mouse(m) => on_mouse(&mut app, m),
                    _ => {}
                }
            }
            _ = refresh.tick() => {
                let mut changed = false;
                if app.subagents.wants_poll(app.child.is_some()) {
                    changed |= refresh_subagents(&session_manager, &parent_id, &mut app).await;
                }
                changed |= sync_child(&session_manager, &mut app).await;
                redraw = changed;
            }
            received = subagents::next_child_event(app.child.as_deref_mut()) => {
                if let Some(child) = app.child.as_deref_mut() {
                    child.apply_recv(received);
                    // Take the rest of a burst in one go, so a fast stream costs
                    // one redraw per wake rather than one per token.
                    child.drain();
                }
                sync_child(&session_manager, &mut app).await;
            }
        }
    }
    Ok(())
}

/// Re-read this chat's subagents into the pane. Returns true when anything the
/// pane draws changed, so a caller on a timer redraws only then.
async fn refresh_subagents(
    session_manager: &SessionManager,
    parent_id: &str,
    app: &mut App,
) -> bool {
    match biorouter::agents::subagent_roster::session_subagents(session_manager, parent_id).await {
        Ok(snapshot) => app
            .subagents
            .set_model(subagents::PaneModel::from_snapshot(&snapshot)),
        Err(error) => {
            tracing::debug!(%error, "could not read this chat's subagents");
            false
        }
    }
}

/// Open the `/decisions` overlay over this chat's decision aggregates, read in
/// this process from the session store (the TUI's turns run here).
async fn open_decisions(session_manager: &SessionManager, session_id: &str, app: &mut App) {
    match biorouter::decision::log::summary(session_manager, session_id).await {
        Ok(summary) => {
            app.completion = None;
            let last = crate::commands::session_decisions::last_calls_from_store(
                session_manager,
                session_id,
                &summary,
            )
            .await;
            let mut overlay = decisions::DecisionsOverlay::new(summary);
            overlay.last = last;
            app.decisions = Some(overlay);
        }
        Err(error) => app.push_error(&format!("Couldn't read this chat's decisions: {error}")),
    }
}

/// Read the last calls of the aggregate Enter picked in the overlay, if any.
async fn open_pending_decision_calls(
    session_manager: &SessionManager,
    session_id: &str,
    app: &mut App,
) {
    let Some(overlay) = app.decisions.as_mut() else {
        return;
    };
    let Some(index) = overlay.pending_calls.take() else {
        return;
    };
    let Some(aggregate) = overlay.summary.aggregates.get(index).cloned() else {
        return;
    };
    let query = decisions::DecisionsOverlay::calls_query(&aggregate);
    match biorouter::decision::log::list(session_manager, session_id, &query).await {
        Ok(page) => {
            overlay.error = None;
            overlay.scroll = 0;
            overlay.calls = Some(decisions::CallsView {
                label: decisions::calls_label(&aggregate),
                records: page.records,
                total: page.total,
            });
        }
        Err(error) => overlay.error = Some(format!("Couldn't read those calls: {error}")),
    }
}

/// Read the decision-log row a decision-chat reply names, so the reply draws as
/// a card. A row that cannot be read leaves the reply's own text to be drawn.
async fn load_decision_record(
    session_manager: &SessionManager,
    session_id: &str,
    app: &mut App,
    message: &Message,
) {
    let Some(id) = message.metadata.decision_id.as_ref() else {
        return;
    };
    if app.decision_records.contains_key(id) {
        return;
    }
    match biorouter::decision::log::get(session_manager, session_id, id).await {
        Ok(Some(detail)) => {
            app.decision_records.insert(id.clone(), detail.record);
        }
        Ok(None) => {}
        Err(error) => tracing::debug!(%error, "could not read a decision-log row"),
    }
}

/// Apply what the open child's bus delivered, and re-read its chat from the
/// store when a turn ended or the bus lagged. Returns true when anything
/// changed on screen.
async fn sync_child(session_manager: &SessionManager, app: &mut App) -> bool {
    let Some(child) = app.child.as_deref_mut() else {
        return false;
    };
    let mut changed = child.drain();
    if child.needs_reload {
        if child.subscription.is_none() {
            child.subscription = Some(biorouter::session_events::subscribe(&child.session_id));
        }
        match session_manager.get_session(&child.session_id, true).await {
            Ok(stored) => {
                let messages = stored
                    .conversation
                    .as_ref()
                    .map(|c| c.messages().as_slice())
                    .unwrap_or(&[]);
                child.load(messages);
            }
            Err(error) => {
                child.needs_reload = false;
                tracing::debug!(%error, "could not re-read a subagent's chat");
            }
        }
        changed = true;
    }
    changed
}

/// Open the row Enter picked in the pane, if any.
async fn open_pending_child(session_manager: &SessionManager, debug: bool, app: &mut App) {
    if let Some(id) = app.subagents.pending_open.take() {
        open_child(session_manager, debug, app, &id, true).await;
    }
}

/// Show a subagent's chat, read-only, in place of the history.
///
/// Only this chat's own subagents reach here (`/attach` and the pane resolve
/// against the roster), and a child's privacy tier equals its parent's, so
/// reading it in this process crosses no privacy boundary.
async fn open_child(
    session_manager: &SessionManager,
    debug: bool,
    app: &mut App,
    child_id: &str,
    return_to_pane: bool,
) {
    let mut view = subagents::ChildView::new(child_id, return_to_pane, debug);
    // Subscribe before the store read, so nothing published in between is
    // lost; the view skips events for messages the read already drew.
    view.subscription = Some(biorouter::session_events::subscribe(child_id));
    match session_manager.get_session(child_id, true).await {
        Ok(stored) => {
            view.name = stored
                .name
                .strip_prefix("Subagent: ")
                .unwrap_or(&stored.name)
                .to_string();
            view.model = stored.model_config.as_ref().map(|m| m.model_name.clone());
            let messages = stored
                .conversation
                .as_ref()
                .map(|c| c.messages().as_slice())
                .unwrap_or(&[]);
            view.load(messages);
            app.subagents.focused = false;
            app.subagents.selected = Some(child_id.to_string());
            app.child = Some(Box::new(view));
        }
        Err(error) => app.push_error(&format!("Couldn't open that subagent's chat: {error}")),
    }
}

/// Leave a subagent's chat, putting focus back in the pane when the chat was
/// opened from it.
fn close_child(app: &mut App) {
    let back_to_pane = app.child.take().is_some_and(|c| c.return_to_pane);
    if back_to_pane && app.subagents.visible() {
        app.subagents.focused = true;
    }
}

/// Scroll whatever the history area shows: the open subagent's chat, else
/// this chat.
fn scroll_history(app: &mut App, up: bool, lines: u16) {
    let target = match app.child.as_deref_mut() {
        Some(child) => &mut child.transcript,
        None => app,
    };
    if up {
        target.scroll_up(lines);
    } else {
        target.scroll_down(lines);
    }
}

fn on_paste(app: &mut App, text: &str) {
    if app.child.is_some() {
        return; // read-only
    }
    app.subagents.focused = false;
    app.paste(text);
}

/// What a key did to the subagent pane or a subagent's chat.
#[derive(Debug, PartialEq, Eq)]
enum PaneKey {
    /// The pane or the child view used it; nothing else runs.
    Consumed,
    /// Not theirs: the normal input handling runs.
    PassThrough,
}

/// Keys for the subagent pane and a subagent's chat, checked before the input
/// editor sees a key. `Ctrl-B` shows or hides the pane and `Ctrl-O` moves focus
/// into it and back; neither was bound before. Arrow keys stay the editor's
/// unless the pane has focus, so composing a message is never affected.
fn on_subagent_key(app: &mut App, key: KeyEvent, streaming: bool) -> PaneKey {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let typed = !key.modifiers.intersects(
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META,
    );
    match key.code {
        KeyCode::Char('b') if ctrl => {
            app.subagents.toggle_visible();
            return PaneKey::Consumed;
        }
        KeyCode::Char('o') if ctrl => {
            if app.subagents.focused {
                app.subagents.focused = false;
            } else {
                app.completion = None;
                app.subagents.focus();
            }
            return PaneKey::Consumed;
        }
        _ => {}
    }

    if app.subagents.focused {
        match key.code {
            KeyCode::Up => app.subagents.move_selection(-1),
            KeyCode::Down => app.subagents.move_selection(1),
            KeyCode::Home => app.subagents.select_edge(false),
            KeyCode::End => app.subagents.select_edge(true),
            KeyCode::Enter => app.subagents.pending_open = app.subagents.selected.clone(),
            KeyCode::Esc => app.subagents.focused = false,
            KeyCode::PageUp => scroll_history(app, true, 10),
            KeyCode::PageDown => scroll_history(app, false, 10),
            // While a turn runs, ^C stops it wherever focus is.
            KeyCode::Char('c') if ctrl && streaming => return PaneKey::PassThrough,
            KeyCode::Char('c') if ctrl => app.subagents.focused = false,
            KeyCode::Char('d') if ctrl && !streaming && app.child.is_none() => {
                return PaneKey::PassThrough
            }
            // Typing goes back to the input and lands there, so the pane
            // never swallows the start of a message.
            // A paste shortcut is typing too.
            KeyCode::Char(_)
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Tab
                if typed || (matches!(key.code, KeyCode::Char('v')) && shortcut_modifier(key)) =>
            {
                app.subagents.focused = false;
                if app.child.is_none() {
                    return PaneKey::PassThrough;
                }
            }
            _ => {}
        }
        return PaneKey::Consumed;
    }

    if app.child.is_some() {
        match key.code {
            KeyCode::Esc => close_child(app),
            KeyCode::Char('c') if ctrl && streaming => return PaneKey::PassThrough,
            KeyCode::Char('c') if ctrl => close_child(app),
            KeyCode::Char('d') if ctrl && !streaming => return PaneKey::PassThrough,
            KeyCode::PageUp => scroll_history(app, true, 10),
            KeyCode::PageDown => scroll_history(app, false, 10),
            KeyCode::Up => scroll_history(app, true, 1),
            KeyCode::Down => scroll_history(app, false, 1),
            KeyCode::End => {
                if let Some(child) = app.child.as_deref_mut() {
                    child.transcript.scroll = 0;
                }
            }
            // Read-only: nothing is typed into a subagent's chat from here.
            _ => {}
        }
        return PaneKey::Consumed;
    }
    PaneKey::PassThrough
}

/// Handle a key press in the idle (composing) state. Returns the text to submit
/// when the user presses Enter on a non-empty buffer.
fn on_key(app: &mut App, key: KeyEvent) -> Option<String> {
    if let Some(overlay) = app.decisions.as_mut() {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl_c || decisions::on_key(overlay, key.code) == decisions::OverlayKey::Closed {
            app.decisions = None;
        }
        return None;
    }
    if on_subagent_key(app, key, false) == PaneKey::Consumed {
        app.refresh_completion();
        return None;
    }
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
            // A command typed out in full runs on this Enter (E-4); the
            // popup closes and the Enter below submits it.
            KeyCode::Enter if app.completion_is_typed_command() => app.completion = None,
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
    if let Some(overlay) = app.decisions.as_mut() {
        // While a turn runs, ^C stops it wherever focus is, and closes the
        // overlay with it.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            app.decisions = None;
            return StreamAction::Cancel;
        }
        if decisions::on_key(overlay, key.code) == decisions::OverlayKey::Closed {
            app.decisions = None;
        }
        return StreamAction::None;
    }
    if on_subagent_key(app, key, true) == PaneKey::Consumed {
        app.refresh_completion();
        return StreamAction::None;
    }
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
            KeyCode::Enter if app.completion_is_typed_command() => app.completion = None,
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
        MouseEventKind::ScrollUp => scroll_history(app, true, 3),
        MouseEventKind::ScrollDown => scroll_history(app, false, 3),
        // A subagent's chat is read-only, so there is no input to select in.
        _ if app.child.is_some() => {}
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
        if let Some(note) = compact_note(&app.status.provider) {
            app.push_note(note);
        }
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

    let mut computer_use_challenge = None;
    let mut computer_use_poll = tokio::time::interval(Duration::from_millis(500));
    // The subagent pane follows children this turn spawns: the spinner tick
    // re-reads the roster, throttled to the pane's refresh interval.
    let session_manager = session.agent.config.session_manager.clone();
    let parent_id = session.session_id.clone();
    // Subagents the person's Stop stopped this turn, said once the turn ends.
    let mut stopped_subagents: Vec<String> = Vec::new();
    let mut last_roster_read = std::time::Instant::now()
        .checked_sub(subagents::REFRESH_INTERVAL)
        .unwrap_or_else(std::time::Instant::now);
    loop {
        let now = std::time::Instant::now();
        if app.stream_start.is_none() || now.duration_since(last_draw) >= frame {
            tui.draw(app)?;
            last_draw = now;
        }
        tokio::select! {
            _ = tick.tick() => {
                app.spin = app.spin.wrapping_add(1);
                if last_roster_read.elapsed() >= subagents::REFRESH_INTERVAL {
                    last_roster_read = std::time::Instant::now();
                    refresh_subagents(&session_manager, &parent_id, app).await;
                }
                sync_child(&session_manager, app).await;
            }
            ev = rx.recv() => {
                match ev {
                    Some(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                        let action = on_key_streaming(app, k);
                        open_pending_child(&session_manager, debug, app).await;
                        open_pending_decision_calls(&session_manager, &parent_id, app).await;
                        match action {
                            StreamAction::Cancel => {
                                session.agent.extension_manager.computer_use.revoke();
                                stop_turn(&cancel, &parent_id, &mut stopped_subagents);
                            }
                            // Park it to run after the current turn. We do NOT
                            // push to the scrollback here: the live stream preview
                            // re-renders by truncating back to its start, which
                            // would wipe the line. The count shows in the input
                            // title instead (see draw_input).
                            StreamAction::Queue(text) => app.queued.push_back(text),
                            StreamAction::None => {}
                        }
                    }
                    Some(Event::Paste(s)) => on_paste(app, &s),
                    Some(Event::Mouse(m)) => on_mouse(app, m),
                    _ => {}
                }
            }
            _ = computer_use_poll.tick() => {
                if let Ok(status) = session.agent.extension_manager.computer_use_status(session.session_id()).await {
                    if super::computer_use_needs_prompt(&status, computer_use_challenge.as_deref()) {
                        computer_use_challenge = Some(status.challenge_id.clone());
                        let permission = run_permission_modal(app, tui, rx, Some(format!("Biorouter Copilot task approval\n{}", status.disclosure))).await?;
                        if permission == Permission::AllowOnce {
                            session.agent.extension_manager.approve_computer_use(session.session_id(), &status.challenge_id).await?;
                            app.push_note("Biorouter Copilot allowed for this task. Press Esc or Ctrl-C to stop.");
                        } else {
                            session.agent.extension_manager.computer_use.revoke();
                            if permission == Permission::Cancel { cancel.cancel(); }
                        }
                    }
                }
            }
            res = stream.next() => {
                match res {
                    Some(Ok(AgentEvent::Message(message))) => {
                        if let Some((id, prompt)) = super::find_tool_confirmation(&message) {
                            commit_stream_to_session(app, &mut session.messages);
                            app.push_message(&message, debug);
                            app.thinking = None;
                            // The card's prompt first (the modal opens on it), then
                            // what approval files, read with PgUp/PgDn.
                            let prompt =
                                with_artefact(prompt, super::confirmation_artefact(&message));
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
                            let outcome = session
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
                            if let Some(note) = super::late_confirmation_note(&outcome) {
                                app.push_note(note);
                            }
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
                            load_decision_record(&session_manager, &parent_id, app, &message).await;
                            session.messages.push(message.clone());
                            app.push_message(&message, debug);
                        }
                    }
                    Some(Ok(AgentEvent::McpNotification(_))) => {}
                    Some(Ok(AgentEvent::HistoryReplaced(c))) => { session.messages = c; }
                    Some(Ok(AgentEvent::ModelChange { .. })) => {}
                    // Issue #56 Gate B. The TUI shows its binding in the status
                    // line and reads it from the same session the agent does, so
                    // it is already displaying what the frame reports; the arm
                    // this frame exists for is the desktop composer's.
                    Some(Ok(AgentEvent::PrivacyProviderPinned { .. })) => {}
                    // BR-52: the TUI reads token counts from the session row.
                    Some(Ok(AgentEvent::TokenUsage(_))) => {}
                    // Advisory pending tool-call hint; TUI renders authoritative
                    // tool requests only.
                    Some(Ok(AgentEvent::ToolCallPending(_))) => {}
                        Some(Ok(AgentEvent::ToolCallsRetracted { .. } | AgentEvent::SteerWaiting { .. })) => {}
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
    // After the partial reply, not when Stop was pressed: a note pushed under
    // the live preview is wiped when the preview re-renders.
    if let Some(line) =
        crate::commands::session_watch::stopped_subagents_sentence(stopped_subagents.len())
    {
        app.push_note(&line);
    }
    app.thinking = None;
    app.set_catalog(build_catalog(session).await);
    refresh_context(session, app).await;
    // A child spawned in the turn's last moments shows without waiting for
    // the idle loop's first tick.
    refresh_subagents(&session_manager, &parent_id, app).await;
    tui.draw(app)?;
    Ok(())
}

/// The person's Stop while a turn runs (Ctrl-C, from the input or any pane).
/// It ends the turn and, as the classic REPL's Ctrl-C, `biorouter session
/// cancel` and the desktop's Stop do, stops the subagents this chat started
/// (item B): they run on tokens of their own, so ending the turn alone left
/// them running, and spending. Adds the children it stopped to `stopped`, once
/// each: Stop pressed again while the turn unwinds names the ones still
/// unwinding again, and they must not be counted twice.
fn stop_turn(cancel: &CancellationToken, parent_id: &str, stopped: &mut Vec<String>) {
    cancel.cancel();
    for child in super::stop_chat_subagents(parent_id) {
        if !stopped.contains(&child) {
            stopped.push(child);
        }
    }
}

/// A message that is a streamable assistant-text delta (text only, no tool
/// calls / thinking / notifications).
fn is_stream_text(m: &Message) -> bool {
    m.role == Role::Assistant
        && m.metadata.decision_id.is_none()
        && m.metadata.decision_summary.is_none()
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

/// The modal text for a confirmation that carries an artefact to approve
/// (`session::confirmation_artefact`): the card's prompt, then the artefact.
fn with_artefact(prompt: Option<String>, artefact: Option<String>) -> Option<String> {
    match (prompt, artefact) {
        (Some(prompt), Some(artefact)) => Some(format!("{prompt}\n\n{artefact}")),
        (prompt, artefact) => prompt.or(artefact),
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
    let options: Vec<(&'static str, &'static str)> = if prompt
        .as_deref()
        .is_some_and(|text| text.starts_with("Biorouter Copilot task approval\n"))
    {
        vec![
            ("Allow task", "view and control desktop for this task"),
            ("Deny", "keep desktop control off"),
            ("Cancel", "cancel response"),
        ]
    } else if prompt
        .as_deref()
        .is_some_and(|text| text.starts_with("External folder access\n"))
    {
        vec![
            ("Deny", "keep this folder unavailable for this turn"),
            ("Allow", "only the listed files for this operation"),
            (
                "Allow for this session",
                "folder and descendants until revoked or app exit",
            ),
            ("Cancel", "cancel response"),
        ]
    } else if prompt.is_some() {
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
    let selected = usize::from(
        options
            .first()
            .is_some_and(|(label, _)| *label == "Allow task"),
    );
    app.modal = Some(PermissionModal {
        prompt,
        options,
        selected,
        scroll: 0,
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
            KeyCode::PageDown => modal.scroll = modal.scroll.saturating_add(5),
            KeyCode::PageUp => modal.scroll = modal.scroll.saturating_sub(5),
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
        "Allow" | "Allow task" => Permission::AllowOnce,
        "Always allow" | "Allow for this session" => Permission::AlwaysAllow,
        "Deny" => Permission::DenyOnce,
        _ => Permission::Cancel,
    }
}

fn backend_slash_command(text: &str) -> bool {
    let command = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('/');
    command == "extend"
        || command == "knowledge"
        || command == "summarize"
        || biorouter::agents::execute_commands::list_commands()
            .iter()
            .any(|def| def.name == command)
        || biorouter::slash_commands::get_workflow_for_command(command).is_some()
}

/// The TUI-only commands the subagent pane answers. Not every entry of
/// [`app::TUI_ONLY_SLASH_COMMANDS`]: `/decisions` is TUI-only too, and parsing
/// it here sent it to `/attach`'s usage line instead of the overlay.
const SUBAGENT_SLASH_COMMANDS: &[&str] = &["/subagents", "/attach"];

/// `/subagents` or `/attach`, with what follows it.
fn subagent_command(text: &str) -> Option<(&'static str, &str)> {
    SUBAGENT_SLASH_COMMANDS.iter().find_map(|&command| {
        let rest = text.strip_prefix(command)?;
        (rest.is_empty() || rest.starts_with(' ')).then_some((command, rest.trim()))
    })
}

/// `/subagents [list|show|hide]` and `/attach <n|id|name>`. Both re-read the
/// roster first, so a child spawned a moment ago can be named.
async fn subagent_slash(session: &CliSession, app: &mut App, text: &str) {
    let Some((command, arg)) = subagent_command(text) else {
        return;
    };
    let session_manager = session.agent.config.session_manager.clone();
    refresh_subagents(&session_manager, session.session_id(), app).await;
    match (command, arg) {
        ("/subagents", "") => app.subagents.toggle_visible(),
        ("/subagents", "list") => subagents::list_into(app),
        ("/subagents", "show") => app.subagents.toggle = Some(true),
        ("/subagents", "hide") => {
            app.subagents.toggle = Some(false);
            app.subagents.focused = false;
        }
        ("/subagents", _) => app.push_note("Usage: /subagents [list|show|hide]"),
        (_, "") => {
            app.push_note("Usage: /attach <number|id|name>. /subagents list shows the numbers.")
        }
        (_, arg) => match app.subagents.model.resolve(arg) {
            Ok(child_id) => {
                open_child(&session_manager, session.debug, app, &child_id, false).await
            }
            Err(reason) => app.push_note(&reason),
        },
    }
}

/// Handle slash commands that the TUI services locally. Returns true if handled.
/// (`/compact` is handled in `submit` since it drives a turn.)
async fn handle_slash(session: &mut CliSession, app: &mut App, text: &str) -> bool {
    match text.trim() {
        s if s == "/folder-access" || s.starts_with("/folder-access ") => {
            app.push_note(&session.folder_access_command(
                s.strip_prefix("/folder-access").unwrap_or_default().trim(),
            ));
            true
        }
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
        s if subagent_command(s).is_some() => {
            subagent_slash(session, app, s).await;
            true
        }
        "/decisions" => {
            let session_manager = session.agent.config.session_manager.clone();
            open_decisions(&session_manager, session.session_id(), app).await;
            true
        }
        s if s == "/diverge" || s.starts_with("/diverge ") => {
            let name = s.strip_prefix("/diverge").unwrap_or("").trim().to_string();
            let name = (!name.is_empty()).then_some(name);
            match session.diverge_and_open(|url| open::that(url), name).await {
                Ok(outcome) => match outcome.open_error {
                    None => app.push_note(&format!(
                        "Diverged into a new window (session ID {}). This chat is unchanged.",
                        outcome.new_session_id
                    )),
                    Some(err) => app.push_note(&format!(
                        "Created a diverged chat (session ID {}) but couldn't open a window ({}). Open Biorouter and run: {}",
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
        s if backend_slash_command(s) => false,
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
    // The subagent pane: beside the history when there is room for both, else
    // a short strip above the status lines. Hidden while the chat has none.
    let pane_shown = app.subagents.visible();
    let wide = area.width >= subagents::WIDE_MIN_WIDTH;
    let strip_h = if pane_shown && !wide {
        subagents::strip_height(&app.subagents)
    } else {
        0
    };
    // The input cluster (status + box + hints) is pinned to the bottom and never
    // moves as the conversation grows; the history pane flexibly fills all the
    // space above it and scrolls to keep the latest output in view (Claude-style).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1), // history → all remaining space at the top
            Constraint::Length(gap_h),
            Constraint::Length(queued_h),
            Constraint::Length(strip_h),
            Constraint::Length(status_h),
            Constraint::Length(input_h),
            Constraint::Length(hints_h),
        ])
        .split(area);

    let open_child = app.child.as_ref().map(|c| c.session_id.clone());
    let mut history_area = chunks[0];
    if pane_shown && wide {
        let split = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(subagents::side_width(area.width)),
        ])
        .split(chunks[0]);
        history_area = split[0];
        subagents::draw_side(f, &mut app.subagents, split[2], open_child.as_deref());
    }
    if let Some(child) = app.child.as_deref_mut() {
        child.transcript.render_stream_if_dirty();
        let row = app.subagents.model.row(&child.session_id);
        subagents::draw_child(f, child, row, history_area);
    } else {
        draw_history(f, app, history_area);
    }
    if queued_h > 0 {
        draw_queued(f, app, chunks[2]);
    }
    if strip_h > 0 {
        subagents::draw_strip(f, &mut app.subagents, chunks[3], open_child.as_deref());
    }
    draw_status(f, app, chunks[4]);
    draw_input(f, app, chunks[5]);
    draw_hints(f, app, chunks[6]);

    // The completion popup floats just above the input box, and only while
    // the input is the thing being typed into.
    if app.completion.is_some()
        && app.modal.is_none()
        && app.decisions.is_none()
        && !app.subagents.focused
        && app.child.is_none()
    {
        draw_completion(f, app, chunks[5]);
    }
    if let Some(overlay) = &app.decisions {
        decisions::draw_overlay(f, overlay, chunks[0]);
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

/// Draw the history anchored to its newest row, `app.scroll` rows up from it.
///
/// ⚠ The rows are wrapped HERE ([`wrap_line`]) and handed to ratatui one row
/// per `Line`, never handed whole with `Wrap`. The view is anchored by counting
/// rows, and the count used to be cells ÷ width while ratatui wrapped at word
/// boundaries: every long prose line took a row or so more than counted, so a
/// transcript of long lines (a subagent's 143-line spawn context) opened that
/// many rows above its newest message. One function both counts and draws, so
/// the two cannot disagree.
fn draw_history(f: &mut Frame, app: &mut App, area: Rect) {
    // The wrapped-row total is an O(content) re-measure. Cache it by a cheap
    // content signature so spinner-tick redraws (where nothing changed) don't
    // recompute it; recompute only when scrollback grows, the viewport resizes,
    // or a streamed token extends the in-progress text.
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
    let height = area.height as usize;
    let scroll = (app.scroll as usize).min((total as usize).saturating_sub(height));
    // Only the rows from the bottom up to the top of the viewport are wrapped
    // to be drawn, so a long history costs a redraw nothing it does not show.
    let wanted = scroll + height;
    let mut rows: Vec<Line<'static>> = Vec::with_capacity(wanted);
    for line in app.scrollback.iter().rev() {
        if rows.len() >= wanted {
            break;
        }
        rows.extend(wrap_line(line, area.width).into_iter().rev());
    }
    rows.reverse();
    let end = rows.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    rows.truncate(end);
    f.render_widget(Paragraph::new(rows.split_off(start)), area);
}

/// How many terminal rows `lines` take in a `width`-cell viewport: exactly the
/// rows [`wrap_line`] makes of them.
fn wrapped_count(lines: &[Line<'static>], width: u16) -> u16 {
    let total: usize = lines
        .iter()
        .map(|line| {
            if fits_one_row(line, width) {
                1
            } else {
                wrap_line(line, width).len()
            }
        })
        .sum();
    total.min(u16::MAX as usize) as u16
}

/// A space a row may break at. A no-break space is not one.
fn is_break_space(ch: char) -> bool {
    ch.is_whitespace() && ch != '\u{a0}' && ch != '\u{202f}'
}

/// Whether `line` is drawn as it is, on one row: it fits, in display cells, and
/// holds no control character (which [`wrap_line`] drops).
fn fits_one_row(line: &Line, width: u16) -> bool {
    let mut cells = 0usize;
    for span in &line.spans {
        for ch in span.content.chars() {
            if ch.is_control() {
                return false;
            }
            cells += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
    }
    cells <= usize::from(width.max(1))
}

/// The rows `line` takes in a `width`-cell viewport, word-wrapped: a row breaks
/// after its last space when the next word does not fit, a space at the edge is
/// not carried onto the next row, and a word wider than the row is cut. Styles,
/// the line's own style and its alignment ride along; control characters (a
/// stray `\n` or `\t` inside a span) are dropped, since a row cannot hold them.
pub(super) fn wrap_line(line: &Line<'static>, width: u16) -> Vec<Line<'static>> {
    if fits_one_row(line, width) {
        return vec![line.clone()];
    }
    let width = usize::from(width.max(1));
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let mut row: Vec<(char, Style)> = Vec::new();
    let mut row_w = 0usize;
    // Where the row may break (just after a space that follows a word), and
    // the row's width up to there.
    let mut brk: Option<(usize, usize)> = None;
    let mut has_word = false;
    for span in &line.spans {
        for ch in span.content.chars() {
            if ch.is_control() {
                continue;
            }
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            let space = is_break_space(ch);
            if row_w + cw > width && !row.is_empty() {
                if space {
                    rows.push(std::mem::take(&mut row));
                    row_w = 0;
                    brk = None;
                    has_word = false;
                    continue;
                }
                match brk.take() {
                    Some((at, w_at)) => {
                        // The word being written moves down whole.
                        let word = row.split_off(at);
                        rows.push(std::mem::replace(&mut row, word));
                        row_w -= w_at;
                        has_word = !row.is_empty();
                        if row_w + cw > width {
                            rows.push(std::mem::take(&mut row));
                            row_w = 0;
                            has_word = false;
                        }
                    }
                    None => {
                        rows.push(std::mem::take(&mut row));
                        row_w = 0;
                        has_word = false;
                    }
                }
            }
            row.push((ch, span.style));
            row_w += cw;
            if !space {
                has_word = true;
            } else if has_word {
                brk = Some((row.len(), row_w));
            }
        }
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows.into_iter()
        .map(|cells| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut text = String::new();
            let mut current: Option<Style> = None;
            for (ch, style) in cells {
                if current != Some(style) {
                    if let Some(prev) = current {
                        spans.push(Span::styled(std::mem::take(&mut text), prev));
                    }
                    current = Some(style);
                }
                text.push(ch);
            }
            if let Some(prev) = current {
                spans.push(Span::styled(text, prev));
            }
            let mut out = Line::from(spans);
            out.style = line.style;
            out.alignment = line.alignment;
            out
        })
        .collect()
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let sep = || Span::styled("  ·  ", dim);
    let s = &app.status;

    // Line 1: the large-language model and the provider. A Baseten decision
    // model reads by its alias (`joey-1`), as its cards and replies do, and a
    // decision model by where it runs rather than its provider key.
    let line1 = Line::from(vec![
        Span::styled(
            crate::commands::decide::model_name(&s.provider, &s.model),
            Style::new().fg(ACCENT),
        ),
        sep(),
        Span::styled(status_provider_text(&s.provider, &s.model), dim),
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

fn draw_hints(f: &mut Frame, app: &App, area: Rect) {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let text = hint_text(app, area.width as usize);
    f.render_widget(Paragraph::new(Line::from(Span::styled(text, dim))), area);
}

/// The hints line: the longest candidate for what has focus that fits.
fn hint_text(app: &App, width: usize) -> String {
    const COMPACT: &str = "↵ send · ^J newline · / commands · ↑↓ history · ^C stop";
    let candidates: Vec<String> = if let Some(overlay) = &app.decisions {
        if overlay.calls.is_some() {
            vec![
                "esc back to the sources · ↑↓ or PgUp/PgDn to scroll".into(),
                "esc back".into(),
            ]
        } else {
            vec![
                "↑↓ select a source · ↵ its last calls · esc close".into(),
                "↑↓ · ↵ calls · esc close".into(),
            ]
        }
    } else if app.child.is_some() {
        vec![
            "esc back to your chat · PgUp/PgDn or the mouse to scroll · read-only".into(),
            "esc back to your chat · PgUp/PgDn scroll · read-only".into(),
            "esc back · read-only".into(),
        ]
    } else if app.subagents.focused {
        vec![
            "↑↓ select · ↵ open the subagent's chat · esc back to the input · ^B hide".into(),
            "↑↓ select · ↵ open · esc back · ^B hide".into(),
            "↑↓ · ↵ open · esc back".into(),
        ]
    } else if let Some(summary) = app.subagents.summary().filter(|_| !app.subagents.visible()) {
        vec![
            format!("{summary} · ^B show · ↵ send · / commands · ^C stop"),
            format!("{summary} · ^B show · ^C stop"),
            COMPACT.into(),
        ]
    } else if !app.subagents.model.rows.is_empty() {
        vec![
            "↵ send · ^J newline · / commands · ↑↓ history · ^C stop · ^O select a subagent · ^B hide".into(),
            "↵ send · / commands · ↑↓ history · ^C stop · ^O select · ^B hide".into(),
            COMPACT.into(),
        ]
    } else {
        vec![
            "↵ send · ^J newline · / commands · ↑↓ history · ^C stop · ^B subagents · type anytime"
                .into(),
            "↵ send · ^J newline · / commands · ↑↓ history · ^C stop · ^B subagents".into(),
            COMPACT.into(),
        ]
    };
    let last = candidates.last().cloned().unwrap_or_default();
    candidates
        .into_iter()
        .find(|c| UnicodeWidthStr::width(c.as_str()) <= width)
        .unwrap_or(last)
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
    if app.child.is_some() {
        // A subagent's chat is read-only here: say why instead of offering a
        // prompt that would send nothing. What was typed for this chat stays
        // in the buffer and comes back with Esc.
        app.last_input_inner = None;
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("⊘ ", Style::new().fg(Color::Indexed(244))),
                Span::styled(
                    "Read-only: this is a subagent's chat. Esc returns to yours.",
                    Style::new()
                        .fg(Color::Indexed(244))
                        .add_modifier(Modifier::DIM),
                ),
            ])),
            inner,
        );
        return;
    }
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

    if app.modal.is_none() && !app.subagents.focused {
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
    let w = 80u16.min(area.width.saturating_sub(4));
    let h = if modal.prompt.is_some() {
        area.height.saturating_sub(4).min(26)
    } else {
        (modal.options.len() as u16 + 4).min(area.height)
    };
    let rect = Rect {
        x: area.width.saturating_sub(w) / 2,
        y: area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            " Tool approval · PgUp/PgDn to read ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect).inner(Margin::new(1, 0));
    f.render_widget(block, rect);
    let regions = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(modal.options.len() as u16 + 1),
    ])
    .split(inner);
    let prompt = modal
        .prompt
        .as_deref()
        .unwrap_or("Biorouter would like to call the above tool.");
    f.render_widget(
        Paragraph::new(prompt)
            .wrap(Wrap { trim: false })
            .scroll((modal.scroll, 0)),
        regions[0],
    );
    let mut lines = vec![Line::default()];
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
    f.render_widget(Paragraph::new(lines), regions[1]);
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

/// Enabled extensions, excluding Biorouter's own capabilities, which the GUI
/// lists in Settings → Chat → Capabilities and leaves out of every extension
/// count (`ui/desktop/.../capabilities/capabilities.ts`). It asks
/// `ExtensionConfig::is_capability`, the daemon's own classification, rather
/// than a list of keys here: the list this replaced named six of the fourteen
/// capabilities, so the status line counted Biorouter Copilot, Crew, Workspace
/// Control and five more as extensions the person had installed.
fn count_non_capability_extensions() -> usize {
    biorouter::config::get_enabled_extensions()
        .iter()
        .filter(|ext| !ext.is_capability())
        .count()
}

/// What follows the model on the status line: a decision model's place
/// (`this computer`, `Baseten`), never its provider key
/// (`ollama_decision`), which is how a person reads every decision model;
/// any other provider by its name, as `biorouter configure` names it.
fn status_provider_text(provider: &str, model: &str) -> String {
    crate::commands::decide::model_place(provider, model)
        .map(str::to_string)
        .unwrap_or_else(|| provider.to_string())
}

/// How many extensions the status line names for a chat whose agent loaded
/// `loaded`: every one of them, the built-in ones included, as each
/// subagent's row counts them. `None` before the agent has loaded any, when
/// the configured count stands.
fn loaded_extension_count(loaded: &[biorouter::agents::ExtensionConfig]) -> Option<usize> {
    (!loaded.is_empty()).then_some(loaded.len())
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
        label: super::completion::extension_reference_label(name),
        description: description.to_string(),
        insert: format!("{} ", super::completion::extension_marker(name)),
        filter: if builtin {
            format!(
                "extension builtin {}",
                super::completion::extension_reference_search_terms(name)
            )
        } else {
            format!(
                "extension {}",
                super::completion::extension_reference_search_terms(name)
            )
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
async fn build_catalog(session: &CliSession) -> Vec<app::CompletionItem> {
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
    items.push(cmd(
        "/folder-access",
        "List or revoke this chat's folder grants",
    ));
    for def in biorouter::agents::execute_commands::list_commands() {
        items.push(cmd(&format!("/{}", def.name), def.description));
    }
    for mapping in biorouter::slash_commands::list_commands() {
        items.push(cmd(
            &format!("/{}", mapping.command),
            &format!("Run workflow · {}", mapping.workflow_path),
        ));
    }
    items.push(cmd(
        "/subagents",
        "Show or hide this chat's subagents (list: number them)",
    ));
    items.push(cmd("/attach", "Read one subagent's chat"));
    items.push(cmd(
        "/decisions",
        "This chat's decision-model calls, by source and turn",
    ));
    items.push(cmd("/diverge", "Diverge this chat into a new window"));
    items.push(cmd("/rename", "Rename this chat"));
    items.push(cmd("/exit", "Leave the chat"));

    for name in list_skills() {
        items.push(skill_completion_item(&name));
    }

    let enabled = session
        .agent
        .extension_manager
        .get_extension_configs()
        .await;
    for name in super::completion::extension_reference_names(
        enabled.into_iter().map(|extension| extension.name()),
    ) {
        items.push(extension_completion_item(
            &name,
            "Use this extension for the request",
            false,
        ));
    }

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
                    if d.timed_out {
                        format!("{} did not answer in time.", d.display_name)
                    } else {
                        format!("{} not found.", d.display_name)
                    },
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
    let mut commands = vec![
        ("/help, /?".to_string(), "show this help".to_string()),
        (
            "/folder-access [revoke <id>]".to_string(),
            "list or revoke folder grants".to_string(),
        ),
        (
            "/diverge [name]".to_string(),
            "diverge this chat into a new window".to_string(),
        ),
        ("/rename <name>".to_string(), "rename this chat".to_string()),
        (
            "/subagents [list|show|hide]".to_string(),
            "show or hide the subagent pane, or number this chat's subagents".to_string(),
        ),
        (
            "/attach <n|id|name>".to_string(),
            "read one subagent's chat (Esc returns)".to_string(),
        ),
        (
            "/decisions".to_string(),
            "this chat's decision-model calls by source and turn (↵ shows the last calls)"
                .to_string(),
        ),
        ("/exit, /quit".to_string(), "leave the chat".to_string()),
    ];
    commands.extend(
        biorouter::agents::execute_commands::list_commands()
            .iter()
            .map(|def| (format!("/{}", def.name), def.description.to_string())),
    );
    commands.extend(
        biorouter::slash_commands::list_commands()
            .into_iter()
            .map(|mapping| {
                (
                    format!("/{}", mapping.command),
                    format!("Run workflow · {}", mapping.workflow_path),
                )
            }),
    );
    let width = help_column_width(commands.iter().map(|(cmd, _)| cmd.as_str()));
    for (cmd, desc) in commands {
        app.push_line(Line::from(vec![
            Span::styled(help_cell(&cmd, width), Style::new().fg(ACCENT)),
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
        ("^B", "show or hide the subagent pane"),
        (
            "^O",
            "select a subagent: ↑↓ choose, ↵ opens its chat, esc back",
        ),
    ] {
        app.push_line(Line::from(vec![
            Span::styled(help_cell(k, width), Style::new().fg(ACCENT)),
            Span::styled(d.to_string(), Style::new().add_modifier(Modifier::DIM)),
        ]));
    }
}

/// The widest `/help` name, so every description starts in one column; at
/// most [`HELP_COLUMN_MAX`], past which a long name is followed by the gap
/// alone. It was a fixed 14, and `/folder-access [revoke <id>]` ran into its
/// description.
fn help_column_width<'a>(names: impl Iterator<Item = &'a str>) -> usize {
    names
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
        .min(HELP_COLUMN_MAX)
}

/// The widest a `/help` name column grows.
const HELP_COLUMN_MAX: usize = 30;

/// A `/help` name, indented, padded to `width` display cells, then a gap of
/// two that no name runs into.
fn help_cell(name: &str, width: usize) -> String {
    let pad = width.saturating_sub(UnicodeWidthStr::width(name));
    format!("  {name}{}  ", " ".repeat(pad))
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
    // E-3: the extensions this chat has loaded, the count its subagents'
    // rows give too ("14 ext"). The configured list leaves out what is built
    // in, so it read "0 extensions" while fourteen were loaded.
    let loaded = session.agent.get_extension_configs().await;
    if let Some(count) = loaded_extension_count(&loaded) {
        app.status.extensions = count;
    }
    if let Ok(provider) = session.agent.provider().await {
        app.status.context_limit = gauge_limit(
            provider.get_name(),
            provider.get_model_config().context_limit(),
        );
    }
    if let Ok(meta) = session.get_session().await {
        app.status.used_tokens = meta.total_tokens.unwrap_or(0) as usize;
        status_follow_binding(
            &mut app.status,
            meta.provider_name.as_deref(),
            meta.model_config.as_ref().map(|m| m.model_name.as_str()),
        );
    }
}

/// Whether a chat bound to this provider has a context meter, here and at the
/// classic prompt. A chat whose model is a decision model has none: each
/// message is asked on its own and nothing adds to the chat's token total, so
/// a meter there would measure nothing. The only limit is per message, and the
/// size refusal names it when a message is too long.
pub(super) fn draws_context_meter(provider_name: &str) -> bool {
    !biorouter::providers::decision::chat::is_decision_chat_provider(provider_name)
}

/// The note shown as `/compact` starts. None in a decision chat: the agent
/// answers there that there is nothing to compact, and announcing a compaction
/// first contradicted that answer on the next line.
fn compact_note(provider_name: &str) -> Option<&'static str> {
    draws_context_meter(provider_name).then_some("Compacting the chat…")
}

/// The window the context meter measures against, or 0, which draws no meter
/// ([`context_spans`] and [`context_summary_spans`] are empty at 0).
fn gauge_limit(provider_name: &str, limit: usize) -> usize {
    if draws_context_meter(provider_name) {
        limit
    } else {
        0
    }
}

/// The status line names the model this chat is bound to, not the configured
/// default: `--provider`/`--model` and a decision chat bind something else, and
/// the line used to keep naming the default (`gpt-5.5 · versa_azure` over a
/// chat answering with `tev1:0.8b`).
fn status_follow_binding(status: &mut StatusInfo, provider: Option<&str>, model: Option<&str>) {
    if let Some(provider) = provider.filter(|p| !p.is_empty()) {
        status.provider = provider.to_string();
    }
    if let Some(model) = model.filter(|m| !m.is_empty()) {
        status.model = model.to_string();
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
    fn a_decision_chat_draws_no_context_meter() {
        assert_eq!(gauge_limit("ollama_decision", 8_192), 0);
        assert_eq!(gauge_limit("baseten_decision", 16_384), 0);
        assert_eq!(gauge_limit("anthropic", 200_000), 200_000);
        assert!(!draws_context_meter("ollama_decision"));
        assert!(draws_context_meter("ollama"));

        // Drawn, at a width where the full meter fits and at one where only
        // the short `ctx N%` would: neither appears, and no "0%" stands in.
        for width in [120, 72] {
            let mut app = App::new(StatusInfo {
                provider: "ollama_decision".into(),
                model: "tev1:0.8b".into(),
                used_tokens: 1_000,
                context_limit: gauge_limit("ollama_decision", 8_192),
                ..Default::default()
            });
            let text = buffer_text(&mut app, width, 24);
            assert!(text.contains("tev1:0.8b"), "{text}");
            assert!(!text.contains("context ▕"), "{text}");
            assert!(!text.contains("ctx "), "{text}");
            assert!(!text.contains("0%"), "{text}");
        }
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
            scroll: 0,
        });
        let text = buffer_text(&mut app, 80, 24);
        assert!(text.contains("Tool approval"));
        assert!(text.contains("Allow"));
    }

    #[test]
    fn a_bug_report_modal_opens_on_the_prompt_and_scrolls_to_the_report() {
        assert_eq!(with_artefact(None, None), None);
        assert_eq!(with_artefact(Some("p".into()), None).as_deref(), Some("p"));
        assert_eq!(with_artefact(None, Some("a".into())).as_deref(), Some("a"));
        let text = with_artefact(
            Some("Publish this bug report?".into()),
            Some(format!(
                "Title: Blank chart\n\n{}End of the issue body",
                "The panel is blank.\n".repeat(30)
            )),
        )
        .unwrap();
        assert!(text.starts_with("Publish this bug report?\n\n"), "{text}");

        let mut app = App::new(StatusInfo::default());
        app.modal = Some(PermissionModal {
            prompt: Some(text),
            options: vec![("Allow", "allow once"), ("Deny", "deny")],
            selected: 1,
            scroll: 0,
        });
        let first = buffer_text(&mut app, 80, 24);
        assert!(first.contains("Publish this bug report?"));
        assert!(first.contains("Title: Blank chart"));
        app.modal.as_mut().unwrap().scroll = 30;
        let last = buffer_text(&mut app, 80, 24);
        assert!(last.contains("End of the issue body"));
        assert!(last.contains("Deny"));
    }

    #[test]
    fn long_task_disclosure_scrolls_while_choices_remain_visible() {
        let mut app = App::new(StatusInfo::default());
        app.modal = Some(PermissionModal {
            prompt: Some(format!(
                "{}End of sensitive information disclosure",
                "Biorouter Copilot on backend host\n".repeat(24)
            )),
            options: vec![("Allow task", "approve task"), ("Deny", "keep control off")],
            selected: 1,
            scroll: 0,
        });
        let first = buffer_text(&mut app, 80, 24);
        assert!(first.contains("Biorouter Copilot on backend host"));
        assert!(first.contains("Allow task"));
        app.modal.as_mut().unwrap().scroll = 24;
        let last = buffer_text(&mut app, 80, 24);
        assert!(last.contains("End of sensitive information disclosure"));
        assert!(last.contains("Deny"));
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

    /// E-4: a slash command typed out in full runs on the first Enter; a
    /// half-typed one is completed first, as before.
    #[test]
    fn a_typed_out_slash_command_runs_on_the_first_enter() {
        use app::{CompletionItem, CompletionKind};
        let catalog = || {
            vec![CompletionItem {
                label: "/compact".into(),
                description: "compact".into(),
                insert: "/compact ".into(),
                filter: "compact".into(),
                kind: CompletionKind::Command,
            }]
        };
        let mut app = App::new(StatusInfo::default());
        app.set_catalog(catalog());
        for c in "/compact".chars() {
            app.insert_char(c);
        }
        app.refresh_completion();
        assert!(
            app.completion_active(),
            "the popup is open on the full name"
        );
        assert_eq!(
            on_key(&mut app, press(KeyCode::Enter)),
            Some("/compact".to_string())
        );

        let mut app = App::new(StatusInfo::default());
        app.set_catalog(catalog());
        for c in "/comp".chars() {
            app.insert_char(c);
        }
        app.refresh_completion();
        assert_eq!(on_key(&mut app, press(KeyCode::Enter)), None);
        assert_eq!(
            app.input, "/compact ",
            "a half-typed name is completed first"
        );
    }

    /// E-5: every `/help` description starts in one column, after a gap that
    /// a long name never runs into.
    #[test]
    fn help_names_and_descriptions_never_run_together() {
        let names = ["/help, /?", "/folder-access [revoke <id>]", "/exit"];
        let width = help_column_width(names.iter().copied());
        assert_eq!(width, "/folder-access [revoke <id>]".len());
        let cells: Vec<String> = names.iter().map(|n| help_cell(n, width)).collect();
        assert!(
            cells.iter().all(|c| c.chars().count() == width + 4),
            "{cells:?}"
        );
        assert!(cells[1].ends_with(">]  "), "{}", cells[1]);
        let long = "/a-very-long-command-name [with <several> arguments]";
        assert!(help_cell(long, HELP_COLUMN_MAX).ends_with("]  "));
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
    /// the drift `count_non_capability_extensions` already fixes for extensions.
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
    #[test]
    fn shared_backend_commands_are_forwarded_including_effort() {
        for command in biorouter::agents::execute_commands::list_commands() {
            assert!(backend_slash_command(&format!("/{} value", command.name)));
        }
        assert!(backend_slash_command("/extend Biorouter Copilot"));
        assert!(backend_slash_command("/knowledge"));
        assert!(!backend_slash_command("/not-a-command"));
    }

    // ── subagent pane ───────────────────────────────────────────────────────

    use biorouter::agents::orchestration_plan::{PhaseCounts, PhaseKind, PhaseStatus};
    use biorouter::agents::subagent_roster::SubagentRunStatus;
    use subagents::{ChildView, PaneModel, PanePhase, PanePlan, PaneRow};

    fn buffer_lines(app: &mut App, w: u16, h: u16) -> Vec<String> {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn sub(id: &str, name: &str, status: SubagentRunStatus) -> PaneRow {
        let mut row = PaneRow::new(id, name, status);
        row.model = Some("gpt-5.5".into());
        row.provider = Some("versa_azure".into());
        row.elapsed_seconds = Some(30);
        row
    }

    fn two_subagents() -> PaneModel {
        let mut research = sub("child-research", "research-api", SubagentRunStatus::Running);
        research.role = Some("researcher".into());
        research.extensions = vec!["developer".into(), "memory".into(), "webdocuments".into()];
        research.skills = vec!["ggplot".into(), "single-cell".into()];
        research.knowledge_bases = vec!["soul".into()];
        research.elapsed_seconds = Some(102);
        research.total_tokens = Some(12_345);
        research.tool_call_count = 14;
        research.message_count = 30;
        research.loop_phase = Some("tool_batch".into());
        research.result_line = Some("Found three candidate genes".into());
        PaneModel {
            rows: vec![
                research,
                sub("child-writer", "writer", SubagentRunStatus::Completed),
            ],
            plan: None,
        }
    }

    fn app_with(model: PaneModel) -> App {
        let mut app = App::new(StatusInfo {
            provider: "versa_azure".into(),
            model: "gpt-5.2".into(),
            ..Default::default()
        });
        app.push_user("hello parent");
        app.subagents.set_model(model);
        app
    }

    fn line_with<'a>(lines: &'a [String], needle: &str) -> Option<(usize, &'a String)> {
        lines.iter().enumerate().find(|(_, l)| l.contains(needle))
    }

    #[test]
    fn the_pane_is_hidden_while_the_chat_has_no_subagents() {
        let mut app = app_with(PaneModel::default());
        for width in [160, 90] {
            let text = buffer_text(&mut app, width, 30);
            assert!(!text.contains("Subagents"), "width {width}:\n{text}");
            assert!(text.contains("hello parent"));
        }
        // The person can still ask for it, and it says why it is empty.
        on_key(&mut app, ctrl('b'));
        assert!(app.subagents.visible());
        let text = buffer_text(&mut app, 160, 30);
        assert!(text.contains("Subagents"), "{text}");
        assert!(text.contains("No subagents in this chat yet"), "{text}");
        let narrow = buffer_text(&mut app, 90, 30);
        assert!(narrow.contains("none in this chat yet"), "{narrow}");
    }

    #[test]
    fn a_wide_pane_draws_up_to_three_subagents_on_two_lines_each() {
        let mut app = app_with(two_subagents());
        let lines = buffer_lines(&mut app, 160, 30);
        let all = lines.join("\n");
        let (header, _) = line_with(&lines, "Subagents").expect("a header");
        assert!(lines[header].contains("running 1 · done 1/2"), "{all}");
        let (name_at, name_line) = line_with(&lines, "◐ researcher").expect("the row");
        assert!(
            name_line.contains("◐ researcher  research-api"),
            "the role leads and the name follows:\n{all}"
        );
        // The pane is 48 cells inside, too narrow for the words, so the
        // knowledge base takes its short form and the time, which no longer
        // fits whole, is left off rather than cut in half.
        assert!(
            lines[name_at + 1].contains("gpt-5.5 · 3 ext · 2 skills · 1 kb base …"),
            "the setup sits on the row's second line:\n{all}"
        );
        let (writer_at, writer) = line_with(&lines, "● writer").expect("the second row");
        assert!(writer.contains('●'), "{all}");
        assert!(
            lines[writer_at + 1].contains("    gpt-5.5 · 30s  "),
            "a count that is zero is left out:\n{all}"
        );
        // Beside the history, not over it: the pane's edge is right of the text.
        let (_, hello) = line_with(&lines, "hello parent").expect("history");
        let text_at = hello.find("hello parent").unwrap();
        let edge_at = hello
            .rfind('│')
            .expect("the pane's left edge on the same row");
        assert!(text_at < edge_at, "{all}");
        assert!(all.contains("^O select"), "{all}");
    }

    fn phased_subagents() -> PaneModel {
        let mut rows = Vec::new();
        let mut add = |n: usize, phase: &str, status: SubagentRunStatus| {
            for i in 0..n {
                let mut row = sub(
                    &format!("{phase}-{i}-{}", rows.len()),
                    &format!("{phase} agent {}", rows.len() + 1),
                    status,
                );
                row.phase_id = Some(phase.to_string());
                rows.push(row);
            }
        };
        add(2, "plan", SubagentRunStatus::Completed);
        add(3, "research", SubagentRunStatus::Completed);
        add(2, "research", SubagentRunStatus::Running);
        add(10, "implementation", SubagentRunStatus::Queued);
        add(7, "implementation", SubagentRunStatus::Running);
        add(1, "implementation", SubagentRunStatus::Failed);
        rows[5].quiet_seconds = Some(600);
        let counts = |total, done, running, failed, quiet| PhaseCounts {
            total,
            done,
            running,
            blocked: 0,
            failed,
            quiet,
        };
        let mut plan = PanePhase::new("plan", PhaseKind::Plan, PhaseStatus::Completed);
        plan.counts = counts(2, 2, 0, 0, 0);
        let mut research = PanePhase::new("research", PhaseKind::Research, PhaseStatus::InProgress);
        research.counts = counts(5, 3, 2, 0, 1);
        let mut implementation = PanePhase::new(
            "implementation",
            PhaseKind::Implementation,
            PhaseStatus::InProgress,
        );
        implementation.counts = counts(18, 0, 17, 1, 0);
        let mut testing = PanePhase::new("testing", PhaseKind::Testing, PhaseStatus::Pending);
        testing.title = "testing".into();
        PaneModel {
            rows,
            plan: Some(PanePlan {
                title: "Gene survey".into(),
                phases: vec![plan, research, implementation, testing],
            }),
        }
    }

    #[test]
    fn many_subagents_take_one_line_each_under_their_phase_headers() {
        let mut app = app_with(phased_subagents());
        assert_eq!(app.subagents.model.rows.len(), 25);
        let lines = buffer_lines(&mut app, 160, 30);
        let all = lines.join("\n");
        assert!(
            all.contains("running 19 · done 5/25 · failed 1 · 1 quiet"),
            "{all}"
        );
        assert!(all.contains("Gene survey"), "the plan title:\n{all}");
        assert!(all.contains("● plan 2/2"), "{all}");
        assert!(
            all.contains("◐ research 3/5 · 2 running · 1 quiet"),
            "{all}"
        );
        assert!(
            !all.contains(" ext · "),
            "dense rows carry no setup line:\n{all}"
        );
        assert!(all.contains("quiet 10m"), "the quiet row says so:\n{all}");
        assert!(
            all.contains("more"),
            "25 rows do not fit; the rest are counted:\n{all}"
        );

        // Selecting the last row scrolls it into view and counts what is above.
        on_key(&mut app, ctrl('o'));
        on_key(&mut app, press(KeyCode::End));
        let lines = buffer_lines(&mut app, 160, 30);
        let all = lines.join("\n");
        assert!(all.contains("implementation agent 25"), "{all}");
        assert!(all.contains("↑ "), "{all}");
        assert!(all.contains("○ testing · not started"), "{all}");
    }

    #[test]
    fn a_narrow_terminal_gets_a_strip_with_a_more_line() {
        let rows = (1..=10)
            .map(|i| {
                sub(
                    &format!("c{i}"),
                    &format!("agent {i}"),
                    SubagentRunStatus::Running,
                )
            })
            .collect();
        let mut app = app_with(PaneModel { rows, plan: None });
        let lines = buffer_lines(&mut app, 90, 30);
        let all = lines.join("\n");
        assert_eq!(subagents::strip_height(&app.subagents), 6);
        let (strip_at, _) = line_with(&lines, "Subagents").expect("the strip");
        let (status_at, _) = line_with(&lines, "gpt-5.2").expect("the status line");
        assert!(
            strip_at < status_at,
            "the strip sits above the status:\n{all}"
        );
        assert!(all.contains("agent 1"), "{all}");
        assert!(all.contains("+6 more"), "{all}");
        let (_, hello) = line_with(&lines, "hello parent").expect("history");
        assert!(
            !hello.contains("agent"),
            "no side pane at this width:\n{all}"
        );
    }

    #[test]
    fn the_selected_subagent_shows_its_setup() {
        let mut app = app_with(two_subagents());
        let unfocused = buffer_text(&mut app, 160, 40);
        assert!(!unfocused.contains("developer, memory"), "{unfocused}");

        on_key(&mut app, ctrl('o'));
        assert_eq!(
            app.subagents.selected.as_deref(),
            Some("child-research"),
            "focus selects the running row"
        );
        let text = buffer_text(&mut app, 160, 40);
        for needle in [
            "role       researcher",
            "gpt-5.5 · versa_azure",
            "developer, memory, webdocuments",
            "ggplot, single-cell",
            "knowledge  soul",
            "tokens     12.3k",
            "tool calls 14 · 30 messages",
            "tool batch · 1m42s",
            "Found three candidate genes",
            "↑↓ select",
        ] {
            assert!(text.contains(needle), "missing {needle:?}:\n{text}");
        }
        // The narrow strip gives the same answer on one line.
        let narrow = buffer_text(&mut app, 90, 30);
        assert!(narrow.contains("ext: developer, memory"), "{narrow}");
    }

    #[test]
    fn private_subagents_carry_a_lock() {
        let mut model = two_subagents();
        model.rows[1].private = true;
        let mut app = app_with(model);
        let text = buffer_text(&mut app, 160, 30);
        assert!(text.contains('🔒'), "{text}");
    }

    #[test]
    fn pane_keys_toggle_focus_and_never_eat_typing() {
        let mut app = app_with(two_subagents());
        assert!(
            app.subagents.visible(),
            "shown because the chat has subagents"
        );
        on_key(&mut app, ctrl('b'));
        assert!(!app.subagents.visible());
        assert!(
            hint_text(&app, 120).starts_with("◐ 1 of 2 subagents running · ^B show"),
            "{}",
            hint_text(&app, 120)
        );
        on_key(&mut app, ctrl('b'));
        assert!(app.subagents.visible());

        // Unfocused, the arrows and letters are the editor's.
        on_key(&mut app, press(KeyCode::Char('h')));
        assert_eq!(app.input, "h");
        assert!(!app.subagents.focused);

        on_key(&mut app, ctrl('o'));
        assert!(app.subagents.focused);
        on_key(&mut app, press(KeyCode::Down));
        assert_eq!(app.subagents.selected.as_deref(), Some("child-writer"));
        on_key(&mut app, press(KeyCode::Up));
        assert_eq!(app.subagents.selected.as_deref(), Some("child-research"));
        // Typing returns to the input and lands there.
        on_key(&mut app, press(KeyCode::Char('x')));
        assert!(!app.subagents.focused);
        assert_eq!(app.input, "hx");
        // ^C in the pane leaves it rather than quitting.
        on_key(&mut app, ctrl('o'));
        on_key(&mut app, ctrl('c'));
        assert!(!app.subagents.focused && !app.should_quit);
        // Hiding the pane takes focus with it.
        on_key(&mut app, ctrl('o'));
        on_key(&mut app, ctrl('b'));
        assert!(!app.subagents.focused && !app.subagents.visible());
        // While a turn streams, ^C still stops it from inside the pane.
        on_key(&mut app, ctrl('o'));
        assert!(matches!(
            on_key_streaming(&mut app, ctrl('c')),
            StreamAction::Cancel
        ));
    }

    /// Item B in the full-screen TUI, which is what `biorouter session` opens
    /// on a terminal: the person's Stop (^C while a turn runs) ends the turn
    /// AND stops the subagents the chat started, as the classic REPL's ^C and
    /// the desktop's Stop do. ^C pressed again while they unwind counts each
    /// child once.
    #[test]
    fn stop_while_a_turn_runs_also_stops_the_chats_subagents() {
        use biorouter::agents::subagent_handle::{BackgroundSubagent, ChildStopCause};
        let parent = "p10-tui-stop-parent";
        let register = |parent: &str, child: &str| {
            let handle = BackgroundSubagent::register(
                parent,
                child,
                "delegated work",
                CancellationToken::new(),
            );
            assert!(handle.is_running());
            handle
        };
        let first = register(parent, "p10-tui-stop-child-1");
        let second = register(parent, "p10-tui-stop-child-2");
        let other = register("p10-tui-stop-other-parent", "p10-tui-stop-other");

        let mut app = App::new(StatusInfo::default());
        let turn = CancellationToken::new();
        let mut stopped = Vec::new();
        assert!(matches!(
            on_key_streaming(&mut app, ctrl('c')),
            StreamAction::Cancel
        ));
        stop_turn(&turn, parent, &mut stopped);

        assert!(turn.is_cancelled(), "Stop no longer ends the turn");
        assert!(
            first.is_cancelled() && second.is_cancelled(),
            "the TUI's Stop left the chat's subagents running"
        );
        assert_eq!(first.stop_cause(), Some(ChildStopCause::ParentStopped));
        assert!(!other.is_cancelled(), "another chat's subagent was stopped");

        // Pressed again while both are still unwinding.
        stop_turn(&turn, parent, &mut stopped);
        assert_eq!(stopped, ["p10-tui-stop-child-1", "p10-tui-stop-child-2"]);
        assert_eq!(
            crate::commands::session_watch::stopped_subagents_sentence(stopped.len()).as_deref(),
            Some("Also stopped 2 subagents this chat started.")
        );

        for handle in [&first, &second, &other] {
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error("test cleanup"),
            );
        }
    }

    fn open_research_child(app: &mut App) {
        // What `open_child` does once the store read succeeds.
        let mut view = ChildView::new("child-research", true, false);
        view.load(&[
            Message::user().with_text("Survey the GWAS catalog"),
            Message::assistant().with_text("Reading **three** papers"),
        ]);
        app.subagents.focused = false;
        app.subagents.selected = Some("child-research".into());
        app.child = Some(Box::new(view));
    }

    #[test]
    fn enter_opens_a_read_only_child_view_and_esc_returns() {
        let mut app = app_with(two_subagents());
        app.input = "draft".into();
        app.cursor = app.input.len();
        on_key(&mut app, ctrl('o'));
        assert_eq!(on_key(&mut app, press(KeyCode::Enter)), None);
        assert_eq!(
            app.subagents.pending_open.as_deref(),
            Some("child-research"),
            "Enter asks the loop to open the selected row"
        );
        assert_eq!(app.input, "draft", "Enter in the pane sends nothing");

        app.subagents.pending_open = None;
        open_research_child(&mut app);
        let lines = buffer_lines(&mut app, 160, 30);
        let all = lines.join("\n");
        let (head, _) = line_with(&lines, "◂ Subagent").expect("a header");
        for needle in ["researcher", "running", "gpt-5.5", "esc returns"] {
            assert!(lines[head].contains(needle), "{needle}:\n{all}");
        }
        assert!(all.contains("Survey the GWAS catalog"), "{all}");
        assert!(all.contains("Reading three papers"), "{all}");
        assert!(
            !all.contains("hello parent"),
            "the child replaces the history:\n{all}"
        );
        assert!(
            all.contains("Read-only: this is a subagent's chat"),
            "{all}"
        );
        assert!(all.contains("esc back to your chat"), "{all}");
        assert!(
            all.contains("› "),
            "the open row is marked in the pane:\n{all}"
        );

        // Read-only: typing changes nothing, Enter submits nothing.
        on_key(&mut app, press(KeyCode::Char('q')));
        assert_eq!(on_key(&mut app, press(KeyCode::Enter)), None);
        assert_eq!(app.input, "draft");
        // The page keys scroll the child's chat, not the parent's.
        app.child.as_mut().unwrap().transcript.last_total_lines = 100;
        on_key(&mut app, press(KeyCode::PageUp));
        assert_eq!(app.child.as_ref().unwrap().transcript.scroll, 10);
        assert_eq!(app.scroll, 0);

        // Esc returns to the parent and to the pane it was opened from.
        on_key(&mut app, press(KeyCode::Esc));
        assert!(app.child.is_none());
        assert!(app.subagents.focused);
        on_key(&mut app, press(KeyCode::Esc));
        assert!(!app.subagents.focused);
        let back = buffer_text(&mut app, 160, 30);
        assert!(back.contains("hello parent"), "{back}");

        // From /attach (not the pane), ^C also leaves, and focus stays on the input.
        open_research_child(&mut app);
        app.child.as_mut().unwrap().return_to_pane = false;
        on_key(&mut app, ctrl('c'));
        assert!(app.child.is_none() && !app.should_quit && !app.subagents.focused);
    }

    #[test]
    fn subagents_list_numbers_rows_for_attach() {
        let mut app = app_with(two_subagents());
        subagents::list_into(&mut app);
        let lines = buffer_lines(&mut app, 160, 40);
        let text = lines.join("\n");
        let (first, _) = line_with(&lines, "1. ◐ researcher").expect("the role leads");
        assert!(
            lines[first + 1].trim_start().starts_with("research-api"),
            "the name follows on its own line:\n{text}"
        );
        assert!(text.contains("2. ● writer"), "{text}");
        assert!(text.contains("/attach <number>"), "{text}");
        assert_eq!(
            app.subagents.model.resolve("2").as_deref(),
            Ok("child-writer")
        );
    }

    /// The live finding: a child spawned with a role showed its
    /// instruction-derived name ("Convert 37 degrees Celsius to Fahrenheit…")
    /// and buried the role. The role leads every row; the name follows,
    /// dimmed, where it fits; the details keep both.
    fn celsius_child() -> PaneRow {
        let mut row = sub(
            "child-celsius",
            "Convert 37 degrees Celsius to Fahrenheit and report the number",
            SubagentRunStatus::Completed,
        );
        row.role = Some("celsius".into());
        row.model = Some("gpt-5.5-2026-04-24".into());
        row
    }

    fn cell_is_dim(app: &mut App, w: u16, h: u16, needle: &str) -> bool {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let cells = needle.chars().count() as u16;
        for y in 0..h {
            for x in 0..w.saturating_sub(cells) {
                let here: String = (x..x + cells).map(|c| buffer[(c, y)].symbol()).collect();
                if here == needle {
                    return buffer[(x, y)].modifier.contains(Modifier::DIM);
                }
            }
        }
        panic!("{needle:?} is not on screen");
    }

    #[test]
    fn a_role_is_the_rows_label_and_the_name_follows_dimmed() {
        let mut app = app_with(PaneModel {
            rows: vec![celsius_child()],
            plan: None,
        });
        // Wide: the side pane's two-line row.
        let lines = buffer_lines(&mut app, 160, 30);
        let all = lines.join("\n");
        let (at, row) = line_with(&lines, "● celsius").expect("the role leads the row");
        assert!(
            row.contains("● celsius  Convert 37 degrees"),
            "the name follows the role:\n{all}"
        );
        assert!(row.contains('…'), "and is cut to the pane:\n{all}");
        assert!(cell_is_dim(&mut app, 160, 30, "Convert 37"), "{all}");
        assert!(
            !cell_is_dim(&mut app, 160, 30, "celsius"),
            "the label is not dimmed:\n{all}"
        );
        assert!(lines[at + 1].contains("gpt-5.5 · 30s"), "{all}");

        // Narrow: the strip leads with the role too.
        let narrow = buffer_lines(&mut app, 90, 30);
        let narrow_all = narrow.join("\n");
        let (_, strip_row) = line_with(&narrow, "● celsius").expect("{narrow_all}");
        assert!(strip_row.contains("Convert 37"), "{narrow_all}");

        // The details keep both.
        on_key(&mut app, ctrl('o'));
        let text = buffer_text(&mut app, 160, 40);
        assert!(text.contains("role       celsius"), "{text}");
        assert!(
            text.contains("name       Convert 37 degrees Celsius"),
            "{text}"
        );

        // Many rows (one line each) lead with the role as well.
        let mut rows: Vec<PaneRow> = (0..5)
            .map(|i| {
                let mut row = celsius_child();
                row.session_id = format!("c{i}");
                row.role = Some(format!("reader-{i}"));
                row
            })
            .collect();
        rows[0].status = SubagentRunStatus::Running;
        let mut dense = app_with(PaneModel { rows, plan: None });
        let text = buffer_text(&mut dense, 160, 30);
        for i in 0..5 {
            assert!(text.contains(&format!("reader-{i}")), "{text}");
        }

        // `/subagents list` and `/attach` speak the role.
        subagents::list_into(&mut app);
        let lines = buffer_lines(&mut app, 160, 40);
        let all = lines.join("\n");
        let (n, _) = line_with(&lines, "1. ● celsius  done · gpt-5.5 · 30s").expect("{all}");
        assert!(
            lines[n + 1].trim_start().starts_with("Convert 37 degrees"),
            "{all}"
        );
        assert_eq!(
            app.subagents.model.resolve("celsius").as_deref(),
            Ok("child-celsius")
        );
        assert_eq!(
            app.subagents.model.resolve("cels").as_deref(),
            Ok("child-celsius")
        );
    }

    /// The live finding: "gpt-5.5-2026-04-24 · 0 ext · 0 skills · 0 k…". The
    /// second line now names the model as the desktop does, leaves out every
    /// count that is zero and spells knowledge bases out.
    #[test]
    fn the_setup_line_is_short_and_counts_only_what_a_child_has() {
        let mut child = celsius_child();
        child.knowledge_bases = vec!["soul".into()];
        let mut app = app_with(PaneModel {
            rows: vec![child],
            plan: None,
        });
        let lines = buffer_lines(&mut app, 160, 30);
        let all = lines.join("\n");
        let (at, _) = line_with(&lines, "● celsius").expect("{all}");
        assert!(
            lines[at + 1].contains("    gpt-5.5 · 1 knowledge base · 30s"),
            "{all}"
        );
        let row = format!("{}\n{}", lines[at], lines[at + 1]);
        for absent in ["2026-04-24", "0 ext", "0 skills", "0 kb", " KB", " k…"] {
            assert!(!row.contains(absent), "{absent:?} in the row:\n{all}");
        }
        assert!(!all.contains("2026-04-24"), "{all}");
        // The strip and the child's header name the model the same way.
        let narrow = buffer_text(&mut app, 90, 30);
        assert!(narrow.contains(" · gpt-5.5"), "{narrow}");
        assert!(!narrow.contains("2026-04-24"), "{narrow}");
        let mut view = ChildView::new("child-celsius", true, false);
        view.model = Some("gpt-5.5-2026-04-24".into());
        view.load(&[Message::assistant().with_text("98.6")]);
        app.child = Some(Box::new(view));
        let lines = buffer_lines(&mut app, 160, 30);
        let (head, header) = line_with(&lines, "◂ Subagent").expect("a header");
        assert!(header.contains("celsius"), "{header}");
        assert!(header.contains("· gpt-5.5"), "{header}");
        assert!(!lines[head].contains("2026-04-24"), "{header}");
    }

    /// A prose paragraph the way a rendered system prompt writes one: a long
    /// line of short words, a different length each time.
    fn prose_line(i: usize) -> String {
        let words = [
            "you",
            "are",
            "a",
            "careful",
            "subagent",
            "working",
            "for",
            "the",
            "parent",
            "and",
            "report",
            "back",
            "concisely",
            "with",
            "answer",
            "extraordinarily",
            "tool",
        ];
        let mut text = format!("Paragraph {i}:");
        let mut k = i;
        while text.len() < 500 + (i * 37) % 400 {
            k = (k * 7 + 3) % words.len();
            text.push(' ');
            text.push_str(words[k]);
        }
        text
    }

    /// The live finding: a child's chat opened in the middle of its transcript.
    /// The history counted a line's rows as cells ÷ width while it was drawn
    /// word-wrapped, so each long prose line took more rows than counted, and
    /// the bottom-anchored view stopped that many rows short of the newest
    /// message.
    #[test]
    fn a_child_view_opens_on_its_newest_message() {
        let mut app = app_with(two_subagents());
        let mut view = ChildView::new("child-research", true, false);
        let prompt: String = (0..60).map(|i| format!("{}\n", prose_line(i))).collect();
        view.load(&[
            Message::user().with_text(prompt),
            Message::user().with_text("Convert 37 degrees Celsius to Fahrenheit"),
            Message::assistant().with_text("FINAL ANSWER 98.6"),
        ]);
        app.child = Some(Box::new(view));
        for (w, h) in [(160, 30), (120, 24), (90, 40)] {
            let lines = buffer_lines(&mut app, w, h);
            let all = lines.join("\n");
            assert!(
                all.contains("FINAL ANSWER 98.6"),
                "{w}x{h} opens on the newest message:\n{all}"
            );
        }

        // The count is exact, so scrolling all the way up reaches the top, and
        // no further.
        let child = app.child.as_mut().unwrap();
        child.transcript.scroll_up(u16::MAX);
        let lines = buffer_lines(&mut app, 160, 30);
        let all = lines.join("\n");
        assert!(all.contains("Paragraph 0:"), "{all}");
        assert!(!all.contains("FINAL ANSWER"), "{all}");
    }

    /// The parent's own history draws through the same code and is anchored
    /// the same way.
    #[test]
    fn the_main_history_stays_on_its_newest_message_through_long_lines() {
        let mut app = App::new(StatusInfo::default());
        for i in 0..40 {
            app.push_message(&Message::assistant().with_text(prose_line(i)), false);
        }
        app.push_message(&Message::assistant().with_text("NEWEST LINE"), false);
        let text = buffer_text(&mut app, 100, 30);
        assert!(text.contains("NEWEST LINE"), "{text}");
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_counts_what_it_draws() {
        let text = prose_line(3);
        let line = Line::from(vec![
            Span::raw("▌ "),
            Span::styled(text.clone(), Style::new().fg(Color::Yellow)),
        ]);
        for width in [7u16, 40, 61, 97, 120] {
            let rows = wrap_line(&line, width);
            assert_eq!(
                wrapped_count(std::slice::from_ref(&line), width) as usize,
                rows.len()
            );
            let mut words: Vec<String> = Vec::new();
            for row in &rows {
                let row_text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(
                    UnicodeWidthStr::width(row_text.as_str()) <= width as usize,
                    "{width}: {row_text:?}"
                );
                words.extend(row_text.split_whitespace().map(str::to_string));
            }
            // No word is split across rows when it fits on one (the longest
            // word is 15 cells).
            if width >= 16 {
                let expected: Vec<String> = format!("▌ {text}")
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                assert_eq!(words, expected, "{width}");
            }
            // Styles ride along.
            assert!(rows
                .iter()
                .flat_map(|r| r.spans.iter())
                .any(|s| s.style.fg == Some(Color::Yellow)));
        }
        // A word wider than the row is cut, never lost.
        let long = Line::from("x".repeat(25));
        let rows = wrap_line(&long, 10);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .flat_map(|r| r.spans.iter())
                .map(|s| s.content.len())
                .sum::<usize>(),
            25
        );
        // An empty line is one empty row; a line that fits is itself.
        assert_eq!(wrap_line(&Line::default(), 10).len(), 1);
        assert_eq!(
            wrap_line(&Line::from("short"), 10),
            vec![Line::from("short")]
        );
    }

    fn spawn_context(task: &str, prompt_lines: usize) -> String {
        let prompt: Vec<String> = (0..prompt_lines)
            .map(|i| format!("Rule {i}: answer the parent concisely."))
            .collect();
        format!(
            "## Subagent spawn context\n\nSpawned by session: 20261104_parent\n\n\
             ### Task instructions\n{task}\n\n\
             ### Granted extensions\n(parent defaults)\n\n\
             ### Granted skills\n(none)\n\n\
             ### Knowledge bases\n(none)\n\n\
             ### Rendered system prompt\n{}",
            prompt.join("\n")
        )
    }

    fn stamped(text: String) -> Message {
        use biorouter::conversation::message::{MessageProvenance, ProvenanceKind};
        let mut message = Message::user().with_text(text).user_only();
        message.metadata.provenance = Some(MessageProvenance {
            kind: ProvenanceKind::SpawnContext,
            from_session_id: Some("20261104_parent".into()),
            from_session_name: None,
        });
        message
    }

    fn scrollback_text(app: &App) -> String {
        app.scrollback
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The live finding: the child view printed the 143-line spawn-context
    /// record in full. It folds to its task, the way the desktop clamps it
    /// under its "Spawn context" chip, and is found by its provenance stamp.
    #[test]
    fn the_spawn_context_folds_to_its_task_instructions() {
        let record = spawn_context("Convert 37 degrees Celsius to Fahrenheit.", 120);
        let total = record.lines().count();
        let mut view = ChildView::new("child-celsius", true, false);
        view.load(&[
            stamped(record.clone()),
            Message::user().with_text("Convert 37 degrees Celsius to Fahrenheit."),
            Message::assistant().with_text("98.6"),
        ]);
        let text = scrollback_text(&view.transcript);
        assert!(text.contains("▌ Spawn context"), "{text}");
        assert!(text.contains("### Task instructions"), "{text}");
        assert!(
            text.contains("▌ Convert 37 degrees Celsius to Fahrenheit."),
            "the task stays visible:\n{text}"
        );
        // Shown: the session line, a blank, the heading and the task.
        let hidden = total - 6;
        assert!(
            text.contains(&format!("⋯ {hidden} more lines")),
            "{total} lines, {hidden} folded:\n{text}"
        );
        for folded in ["Granted extensions", "Rendered system prompt", "Rule 7:"] {
            assert!(!text.contains(folded), "{folded:?} is folded:\n{text}");
        }
        assert!(
            !text.contains("## Subagent spawn context"),
            "the label replaces the record's heading:\n{text}"
        );
        assert!(
            view.transcript.scrollback.len() < 20,
            "{} rows",
            view.transcript.scrollback.len()
        );

        // Drawn, it opens on the reply, with the folded record above it.
        let mut app = app_with(two_subagents());
        app.child = Some(Box::new(view));
        let screen = buffer_text(&mut app, 160, 40);
        assert!(screen.contains("Spawn context"), "{screen}");
        assert!(screen.contains("98.6"), "{screen}");

        // The stamp decides, not the text: the same words typed as a message
        // are drawn as they are.
        let mut plain = ChildView::new("c", true, false);
        plain.load(&[Message::user().with_text(record.clone())]);
        let text = scrollback_text(&plain.transcript);
        assert!(text.contains("Rendered system prompt"), "{text}");
        assert!(!text.contains("more lines"), "{text}");

        // A record that arrives live folds too, and debug shows all of it.
        let mut live = ChildView::new("c", true, false);
        live.load(&[]);
        live.apply(biorouter::session_events::SessionBusEvent::Agent(
            AgentEvent::Message(stamped(record.clone())),
        ));
        assert!(scrollback_text(&live.transcript).contains("more lines"));
        let mut debug = ChildView::new("c", true, true);
        debug.load(&[stamped(record)]);
        let text = scrollback_text(&debug.transcript);
        assert!(text.contains("Rule 119:"), "{text}");
        assert!(!text.contains("more lines"), "{text}");
    }

    #[test]
    fn a_long_task_is_capped_and_a_record_without_one_shows_its_head() {
        let task: String = (1..=20)
            .map(|i| format!("step {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let record = spawn_context(&task, 3);
        let (shown, hidden) = subagents::spawn_context_preview(&record);
        assert_eq!(shown[0], "Spawned by session: 20261104_parent");
        assert!(shown.contains(&"step 12"), "{shown:?}");
        assert!(!shown.contains(&"step 13"), "{shown:?}");
        assert_eq!(shown.len() + hidden, record.lines().count() - 2);

        let odd = "first\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\neighth";
        let (shown, hidden) = subagents::spawn_context_preview(odd);
        assert_eq!(
            shown,
            ["first", "second", "third", "fourth", "fifth", "sixth"]
        );
        assert_eq!(hidden, 2);
        let (shown, hidden) = subagents::spawn_context_preview("one line");
        assert_eq!((shown, hidden), (vec!["one line"], 0));
    }

    #[test]
    fn compact_announces_itself_only_where_there_is_a_context_to_compact() {
        assert_eq!(compact_note("versa_azure"), Some("Compacting the chat…"));
        assert_eq!(compact_note("ollama_decision"), None);
        assert_eq!(compact_note("baseten_decision"), None);
    }

    #[test]
    fn the_status_line_follows_the_chats_binding_not_the_default() {
        let mut status = StatusInfo {
            provider: "versa_azure".into(),
            model: "gpt-5.5-2026-04-24".into(),
            ..StatusInfo::default()
        };
        status_follow_binding(&mut status, Some("ollama_decision"), Some("tev1:0.8b"));
        assert_eq!(
            (status.provider.as_str(), status.model.as_str()),
            ("ollama_decision", "tev1:0.8b")
        );
        // A row that names nothing yet keeps what the line had.
        status_follow_binding(&mut status, None, Some(""));
        assert_eq!(status.model, "tev1:0.8b");

        // A chat bound to a Baseten decision model by its slug reads by the
        // alias.
        status_follow_binding(
            &mut status,
            Some("baseten_decision"),
            Some("decision-models/baseten-joey-2b-latest"),
        );
        let mut app = App::new(status);
        let text = buffer_text(&mut app, 120, 24);
        assert!(
            text.contains("joey-1  ·  Baseten"),
            "R3: where it runs: {text}"
        );
        assert!(!text.contains("baseten_decision"), "{text}");
        assert!(!text.contains("baseten-joey"), "{text}");
    }

    #[test]
    fn subagent_commands_parse_without_swallowing_lookalikes() {
        assert_eq!(subagent_command("/subagents"), Some(("/subagents", "")));
        assert_eq!(
            subagent_command("/subagents list"),
            Some(("/subagents", "list"))
        );
        assert_eq!(subagent_command("/attach  2 "), Some(("/attach", "2")));
        assert_eq!(subagent_command("/attachment"), None);
        assert_eq!(subagent_command("/subagentsx"), None);
        assert_eq!(subagent_command("/help"), None);
        // Another TUI-only command is not a subagent command.
        assert_eq!(subagent_command("/decisions"), None);
        for command in SUBAGENT_SLASH_COMMANDS {
            assert!(app::TUI_ONLY_SLASH_COMMANDS.contains(command), "{command}");
        }
    }

    #[test]
    fn hints_advertise_the_pane_keys() {
        let app = App::new(StatusInfo::default());
        assert!(hint_text(&app, 200).contains("^B subagents"));
        assert_eq!(
            hint_text(&app, 64),
            "↵ send · ^J newline · / commands · ↑↓ history · ^C stop"
        );
        let app = app_with(two_subagents());
        assert!(hint_text(&app, 200).contains("^O select a subagent"));
    }

    #[test]
    fn copilot_popup_shows_current_label_with_stable_wire_identity() {
        let item = extension_completion_item("computercontroller", "Use this extension", true);
        assert_eq!(item.label, "Biorouter Copilot");
        assert_eq!(item.insert, "/ext:BiorouterCopilot ");
        assert!(item.filter.contains("biorouter copilot"));
        let american = extension_completion_item("autovisualiser", "Use this extension", true);
        assert!(american.filter.contains("autovisualizer"));
        assert_eq!(american.insert, "/ext:autovisualiser ");
    }

    // ── decision models ─────────────────────────────────────────────────────

    use biorouter::conversation::message::{MessageMetadata, SystemNotificationType};
    use biorouter::decision::log::{
        DecisionTurnMarker, DecisionTurnMarkerItem, SessionDecisionSummary,
    };
    use biorouter::decision::DecisionOrigin;

    fn marker_item(
        origin: DecisionOrigin,
        label: &str,
        count: u64,
        fired: bool,
    ) -> DecisionTurnMarkerItem {
        DecisionTurnMarkerItem {
            origin,
            origin_id: Some(label.to_lowercase()),
            label: label.to_string(),
            count,
            calls: count,
            passed: 0,
            failed: count,
            errors: 0,
            last_pass_probability: Some(0.31),
            fired,
        }
    }

    fn marker_notice(items: Vec<DecisionTurnMarkerItem>) -> Message {
        let metadata = MessageMetadata {
            decision_summary: Some(DecisionTurnMarker {
                turn_id: "t1".into(),
                items,
            }),
            ..MessageMetadata::user_only()
        };
        Message::assistant()
            .with_system_notification(
                SystemNotificationType::InlineMessage,
                "Monitors checked 18 times this turn",
            )
            .with_metadata(metadata)
    }

    /// E-3: the status line counts every extension the chat loaded, the
    /// built-in ones included, as its subagents' rows do; it read
    /// "0 extensions" while fourteen were loaded.
    /// R3: a decision chat's status line names where its model runs, never
    /// the provider key.
    #[test]
    fn the_status_line_names_where_a_decision_model_runs() {
        let joey = "decision-models/baseten-joey-2b-latest";
        assert_eq!(status_provider_text("baseten_decision", joey), "Baseten");
        assert_eq!(
            status_provider_text("ollama_decision", "tev1:0.8b"),
            crate::commands::decide::model_place("ollama_decision", "tev1:0.8b").unwrap()
        );
        assert_ne!(
            status_provider_text("ollama_decision", "tev1:0.8b"),
            "ollama_decision"
        );
        // A cloud tag is served by ollama.com, never "this computer".
        assert_eq!(
            status_provider_text("ollama_decision", "gpt-oss:120b-cloud"),
            "Ollama cloud"
        );
        assert_eq!(
            status_provider_text("versa_azure", "gpt-5.5"),
            "versa_azure"
        );
    }

    #[test]
    fn the_status_line_counts_the_loaded_extensions() {
        let developer = biorouter::agents::ExtensionConfig::default();
        assert!(developer.is_capability(), "the default is built in");
        assert_eq!(
            loaded_extension_count(&[developer.clone(), developer]),
            Some(2)
        );
        assert_eq!(loaded_extension_count(&[]), None);
    }

    /// The owner rule: thousands of calls draw as ONE line per origin.
    #[test]
    fn a_turns_decision_marker_draws_one_line_per_origin() {
        let mut app = App::new(StatusInfo::default());
        app.push_message(
            &marker_notice(vec![
                marker_item(DecisionOrigin::Monitor, "Form reached", 1_800, false),
                marker_item(DecisionOrigin::Gate, "Implementation", 3, true),
            ]),
            false,
        );
        let lines = buffer_lines(&mut app, 120, 30);
        let markers: Vec<&String> = lines.iter().filter(|l| l.contains('◇')).collect();
        assert_eq!(markers.len(), 2, "{lines:#?}");
        assert!(markers[0].contains("Monitor 'Form reached': 1800 checks this turn · 1800 quiet"));
        assert!(
            markers[1].contains("Gate 'Implementation': 3 checks this turn · 3 not passed"),
            "a gate passes or does not, never \"failed\": {}",
            markers[1]
        );
        assert!(markers[1].contains("last P(yes) 0.31"), "{}", markers[1]);
        let all = lines.join("\n");
        assert!(
            !all.contains("Monitors checked 18 times"),
            "the marker replaces the notice's text, it does not repeat it"
        );

        // An empty marker draws nothing.
        let mut app = App::new(StatusInfo::default());
        let before = app.scrollback.len();
        app.push_message(&marker_notice(vec![]), false);
        assert_eq!(app.scrollback.len(), before);
    }

    #[test]
    fn an_inline_notice_without_decision_data_is_drawn_as_a_note() {
        let mut app = App::new(StatusInfo::default());
        app.push_message(
            &Message::assistant().with_system_notification(
                SystemNotificationType::InlineMessage,
                "Monitor 'Form reached' fired: P(yes) 0.91. The turn was stopped.",
            ),
            false,
        );
        let all = buffer_lines(&mut app, 120, 20).join("\n");
        assert!(all.contains("Monitor 'Form reached' fired"), "{all}");
    }

    fn decision_reply(id: &str) -> Message {
        let metadata = MessageMetadata {
            decision_id: Some(id.to_string()),
            ..MessageMetadata::default()
        };
        Message::assistant()
            .with_text("Yes · P(yes)=0.93 · tev1:0.8b")
            .with_metadata(metadata)
    }

    #[test]
    fn a_decision_chat_reply_is_a_card_once_its_row_is_read_else_its_text() {
        let reply = decision_reply("d1");
        assert!(
            !is_stream_text(&reply),
            "a decision reply is drawn whole, never streamed as text"
        );
        assert!(!is_stream_text(&marker_notice(vec![])));
        assert!(is_stream_text(&Message::assistant().with_text("plain")));

        let mut app = App::new(StatusInfo::default());
        app.push_message(&reply, false);
        let all = buffer_lines(&mut app, 100, 20).join("\n");
        assert!(all.contains("Yes · P(yes)=0.93 · tev1:0.8b"), "{all}");

        let mut record = crate::commands::session_decisions::tests::record("d1", 0.93);
        record.origin = DecisionOrigin::Chat;
        record.questions = serde_json::json!({
            "has_ms": {
                "instructions": "Does this note state the patient has MS?",
                "kind": {"binary": {}}
            }
        });
        let mut app = App::new(StatusInfo::default());
        app.decision_records.insert("d1".into(), record);
        app.push_message(&reply, false);
        let lines = buffer_lines(&mut app, 100, 20);
        let all = lines.join("\n");
        assert!(
            all.contains("◆ Decision · tev1:0.8b · private · 22 ms"),
            "{all}"
        );
        assert!(
            all.contains("Does this note state the patient has MS?"),
            "{all}"
        );
        let (_, yes) = line_with(&lines, "▸ yes").expect("the answer row is marked");
        assert!(yes.contains('█') && yes.contains("0.93"), "{yes}");
        assert!(line_with(&lines, "  no ").is_some(), "{all}");
        assert!(
            !all.contains("P(yes)=0.93 · tev1"),
            "the card replaces the text, it does not repeat it"
        );
    }

    #[test]
    fn the_decisions_overlay_draws_over_the_history_and_esc_closes_it() {
        let mut app = App::new(StatusInfo::default());
        let mut tool = crate::commands::session_decisions::tests::aggregate(
            DecisionOrigin::Tool,
            "decide",
            600,
        );
        tool.last_answer = Some("yes".into());
        app.decisions = Some(decisions::DecisionsOverlay::new(SessionDecisionSummary {
            total: 618,
            aggregates: vec![
                crate::commands::session_decisions::tests::aggregate(
                    DecisionOrigin::Monitor,
                    "Form reached",
                    18,
                ),
                tool,
            ],
            recent_turns: vec![],
        }));
        app.input = "/dec".into();
        app.cursor = app.input.len();
        app.refresh_completion();
        let all = buffer_lines(&mut app, 140, 30).join("\n");
        assert!(
            all.contains("Decisions · ↑↓ select · ↵ last calls · esc close"),
            "{all}"
        );
        assert!(all.contains("618 calls in this chat"), "{all}");
        assert!(all.contains("▸ monitor  Form reached"), "{all}");
        assert!(
            all.contains("↑↓ select a source · ↵ its last calls · esc close"),
            "the hints follow the overlay: {all}"
        );

        assert_eq!(on_key(&mut app, press(KeyCode::Down)), None);
        assert_eq!(app.decisions.as_ref().unwrap().selected, 1);
        assert_eq!(
            on_key(&mut app, press(KeyCode::Enter)),
            None,
            "Enter never submits here"
        );
        assert_eq!(app.decisions.as_ref().unwrap().pending_calls, Some(1));
        assert_eq!(app.input, "/dec", "keys go to the overlay, not the input");
        on_key(&mut app, press(KeyCode::Esc));
        assert!(app.decisions.is_none());

        app.decisions = Some(decisions::DecisionsOverlay::new(SessionDecisionSummary {
            total: 0,
            aggregates: vec![],
            recent_turns: vec![],
        }));
        assert!(matches!(
            on_key_streaming(&mut app, ctrl('c')),
            StreamAction::Cancel
        ));
        assert!(
            app.decisions.is_none(),
            "^C stops the turn and closes the overlay"
        );
    }

    #[test]
    fn decisions_is_a_tui_command_in_help_and_completion() {
        let mut app = App::new(StatusInfo::default());
        help_into(&mut app);
        let all = buffer_lines(&mut app, 140, 60).join("\n");
        assert!(all.contains("/decisions"), "{all}");
        assert!(app::TUI_ONLY_SLASH_COMMANDS.contains(&"/decisions"));
        for c in "/dec".chars() {
            app.insert_char(c);
        }
        assert_eq!(app.ghost(), Some("isions"));
    }
}
