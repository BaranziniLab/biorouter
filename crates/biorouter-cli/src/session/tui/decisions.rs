//! Decision models in the TUI: the per-turn marker line, the decision card of a
//! decision-chat reply, and the `/decisions` overlay.
//!
//! Decision calls can number in the thousands in one chat, so nothing here
//! draws one line per call in the transcript:
//!
//! - a stored notice whose `metadata.decision_summary` is set is drawn as ONE
//!   compact line per origin (a monitor, the `decide` tool, a phase gate),
//!   never one per call ([`marker_lines`]);
//! - a reply whose `metadata.decision_id` is set (a chat whose main model is a
//!   decision model) is drawn as a small card: the answer, its probability, a
//!   bar per option and the model ([`card_lines`]);
//! - `/decisions` opens an overlay over the history listing the chat's
//!   aggregates and recent turns, read in this process with
//!   `biorouter::decision::log::summary`, the function every surface reads.
//!   Enter on an aggregate shows its last calls; the overlay is the only place
//!   the TUI lists individual calls.
//!
//! The lines are plain functions of decision values, so the TestBackend tests
//! build records by hand and never need a store or an agent. The text of every
//! line comes from the same helpers `biorouter decide` and
//! `biorouter session decisions` print with, so the terminal surfaces agree.

use biorouter::decision::log::{
    DecisionAggregate, DecisionQuery, DecisionRecord, DecisionTurnMarker, SessionDecisionSummary,
};
use biorouter::decision::Question;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::app::ACCENT;
use crate::commands::decide::{
    answer_label, answer_probability, calibration_flag, clip, format_latency, format_p,
    origin_word, outcome_labels, probability_bar, short_model, threshold_line,
};
use crate::commands::session_decisions::{aggregate_line, marker_text, record_line, turn_line};

const DIM: Style = Style::new().add_modifier(Modifier::DIM);
/// Cells of a bar in a decision card.
pub const CARD_BAR: usize = 20;
/// Calls the overlay shows for one aggregate.
pub const LAST_CALLS: u32 = 20;
/// Widest an option label is drawn in a card.
const CARD_LABEL_MAX: usize = 20;

// ── the transcript marker ───────────────────────────────────────────────────

/// One line per origin of a turn's decision calls, never one per call.
pub fn marker_lines(marker: &DecisionTurnMarker) -> Vec<Line<'static>> {
    marker
        .items
        .iter()
        .map(|item| {
            let mut spans = vec![
                Span::styled("◇ ", Style::new().fg(ACCENT)),
                Span::styled(marker_text(item), DIM),
            ];
            if item.fired {
                spans.push(Span::styled(
                    " · fired",
                    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

// ── the decision card ───────────────────────────────────────────────────────

/// The questions a record asked, keyed by name, parsed back from its JSON.
fn questions_of(record: &DecisionRecord) -> Vec<(String, Question)> {
    record
        .questions
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(name, value)| {
                    serde_json::from_value::<Question>(value.clone())
                        .ok()
                        .map(|q| (name.clone(), q))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A decision-chat reply as a small card: a header with the model and latency,
/// then for each question its text, one bar per option with the answer
/// marked, and the answer line with its probability and calibration.
pub fn card_lines(record: &DecisionRecord) -> Vec<Line<'static>> {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let mut header = vec![
        Span::styled("◆ ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("Decision", bold),
    ];
    if let Some(model) = &record.model {
        header.push(Span::styled(format!(" · {}", short_model(model)), DIM));
    }
    if let Some(tier) = &record.tier {
        header.push(Span::styled(format!(" · {tier}"), DIM));
    }
    if let Some(ms) = record.latency_ms {
        header.push(Span::styled(format!(" · {}", format_latency(ms)), DIM));
    }
    let mut lines = vec![Line::from(header)];
    if let Some(error) = &record.error {
        lines.push(Line::from(Span::styled(
            format!("  {}", clip(error, 200)),
            Style::new().fg(Color::Red),
        )));
        return lines;
    }
    let questions = questions_of(record);
    for decision in &record.decisions {
        let question = questions
            .iter()
            .find(|(name, _)| *name == decision.question)
            .map(|(_, q)| q);
        if let Some(question) = question {
            lines.push(Line::from(Span::styled(
                format!("  {}", clip(&question.instructions, 120)),
                DIM,
            )));
        }
        let labels = outcome_labels(decision, question);
        let width = labels
            .iter()
            .map(|(_, l)| clip(l, CARD_LABEL_MAX).chars().count())
            .max()
            .unwrap_or(0);
        let answer = answer_label(decision);
        for (key, label) in &labels {
            let p = decision.probabilities.get(key).copied().unwrap_or(0.0);
            let is_answer = *label == answer || *key == answer;
            let marker = if is_answer { "▸ " } else { "  " };
            let label_style = if is_answer { bold } else { Style::default() };
            lines.push(Line::from(vec![
                Span::styled(format!("  {marker}"), Style::new().fg(ACCENT)),
                Span::styled(
                    format!("{:<width$}  ", clip(label, CARD_LABEL_MAX)),
                    label_style,
                ),
                Span::styled(probability_bar(p, CARD_BAR), Style::new().fg(ACCENT)),
                Span::raw(format!("  {}", format_p(p))),
            ]));
        }
        let mut tail = format!(
            "  {} · P {} · {}",
            answer,
            format_p(answer_probability(decision)),
            calibration_flag(decision)
        );
        if let Some(threshold) = threshold_line(decision, question) {
            tail.push_str(&format!(" · {threshold}"));
        }
        lines.push(Line::from(Span::styled(tail, DIM)));
    }
    lines
}

// ── the overlay ─────────────────────────────────────────────────────────────

/// One aggregate's last calls, shown in place of the aggregate list.
#[derive(Debug, Clone, PartialEq)]
pub struct CallsView {
    pub label: String,
    pub records: Vec<DecisionRecord>,
    pub total: u64,
}

/// `/decisions`: the chat's aggregates and recent turns, and on Enter one
/// aggregate's last calls.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionsOverlay {
    pub summary: SessionDecisionSummary,
    /// The selected aggregate.
    pub selected: usize,
    /// Rows scrolled from the top.
    pub scroll: u16,
    /// The last calls of the aggregate Enter picked, while open.
    pub calls: Option<CallsView>,
    /// The aggregate Enter picked, for the run loop to read its calls.
    pub pending_calls: Option<usize>,
    /// Why the last read failed, shown in the overlay.
    pub error: Option<String>,
}

impl DecisionsOverlay {
    pub fn new(summary: SessionDecisionSummary) -> Self {
        Self {
            summary,
            selected: 0,
            scroll: 0,
            calls: None,
            pending_calls: None,
            error: None,
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        let count = self.summary.aggregates.len();
        if count == 0 {
            return;
        }
        let next = (self.selected as i64 + i64::from(delta)).clamp(0, count as i64 - 1);
        self.selected = next as usize;
    }

    pub fn selected_aggregate(&self) -> Option<&DecisionAggregate> {
        self.summary.aggregates.get(self.selected)
    }

    /// The query for an aggregate's last calls.
    pub fn calls_query(aggregate: &DecisionAggregate) -> DecisionQuery {
        DecisionQuery {
            origin: Some(aggregate.origin),
            origin_id: aggregate.origin_id.clone(),
            limit: Some(LAST_CALLS),
            ..Default::default()
        }
    }

    /// The overlay's lines, in the order they are drawn.
    pub fn lines(&self) -> Vec<Line<'static>> {
        let head = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
        let mut lines = Vec::new();
        if let Some(error) = &self.error {
            lines.push(Line::from(Span::styled(
                error.clone(),
                Style::new().fg(Color::Red),
            )));
        }
        if let Some(calls) = &self.calls {
            lines.push(Line::from(vec![
                Span::styled(calls.label.clone(), head),
                Span::styled(
                    format!(
                        "  last {} of {} {}",
                        calls.records.len(),
                        calls.total,
                        if calls.total == 1 { "call" } else { "calls" }
                    ),
                    DIM,
                ),
            ]));
            if calls.records.is_empty() {
                lines.push(Line::from(Span::styled("  No calls yet.", DIM)));
            }
            for record in &calls.records {
                lines.push(Line::raw(record_line(record)));
            }
            return lines;
        }
        if self.summary.total == 0 {
            lines.push(Line::from(Span::styled(
                "This chat has made no decision-model calls.",
                DIM,
            )));
            return lines;
        }
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} in this chat",
                self.summary.total,
                if self.summary.total == 1 {
                    "call"
                } else {
                    "calls"
                }
            ),
            DIM,
        )));
        let label_w = self
            .summary
            .aggregates
            .iter()
            .map(|a| clip(&a.label, 32).chars().count())
            .chain(
                self.summary
                    .recent_turns
                    .iter()
                    .map(|t| clip(&t.label, 32).chars().count()),
            )
            .max()
            .unwrap_or(0);
        lines.push(Line::from(Span::styled("By source", head)));
        for (i, aggregate) in self.summary.aggregates.iter().enumerate() {
            let selected = i == self.selected;
            let text = aggregate_line(aggregate, label_w);
            let text = text.strip_prefix("  ").unwrap_or(&text).to_string();
            lines.push(Line::from(vec![
                Span::styled(if selected { "▸ " } else { "  " }, Style::new().fg(ACCENT)),
                Span::styled(
                    text,
                    if selected {
                        Style::new().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]));
        }
        if !self.summary.recent_turns.is_empty() {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled("Recent turns", head)));
            for turn in &self.summary.recent_turns {
                lines.push(Line::from(Span::styled(turn_line(turn, label_w), DIM)));
            }
        }
        lines
    }

    pub fn title(&self) -> String {
        match &self.calls {
            Some(_) => " Decisions · last calls · esc back ".to_string(),
            None => " Decisions · ↑↓ select · ↵ last calls · esc close ".to_string(),
        }
    }
}

/// The overlay over the history, centred.
pub fn draw_overlay(f: &mut Frame, overlay: &DecisionsOverlay, area: Rect) {
    let w = area.width.saturating_sub(4).clamp(20, 120);
    let lines = overlay.lines();
    let h = (lines.len() as u16 + 2).clamp(5, area.height.saturating_sub(2).max(5));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w.min(area.width),
        height: h.min(area.height),
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            overlay.title(),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect).inner(Margin::new(1, 0));
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(lines).scroll((overlay.scroll, 0)), inner);
}

/// What a key did to the overlay.
#[derive(Debug, PartialEq, Eq)]
pub enum OverlayKey {
    /// Used; nothing else runs.
    Consumed,
    /// Closed the overlay.
    Closed,
}

/// Keys while the overlay is open: ↑/↓ select, Enter asks for the selected
/// aggregate's calls, PgUp/PgDn scroll, Esc goes back from the calls or
/// closes.
pub fn on_key(overlay: &mut DecisionsOverlay, code: crossterm::event::KeyCode) -> OverlayKey {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => {
            if overlay.calls.take().is_some() {
                overlay.scroll = 0;
                return OverlayKey::Consumed;
            }
            return OverlayKey::Closed;
        }
        KeyCode::Up if overlay.calls.is_none() => overlay.move_selection(-1),
        KeyCode::Down if overlay.calls.is_none() => overlay.move_selection(1),
        KeyCode::Up | KeyCode::PageUp => overlay.scroll = overlay.scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::PageDown => overlay.scroll = overlay.scroll.saturating_add(1),
        KeyCode::Enter if overlay.calls.is_none() && overlay.selected_aggregate().is_some() => {
            overlay.pending_calls = Some(overlay.selected);
        }
        _ => {}
    }
    OverlayKey::Consumed
}

/// The label an aggregate's calls are shown under.
pub fn calls_label(aggregate: &DecisionAggregate) -> String {
    format!("{} · {}", origin_word(aggregate.origin), aggregate.label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::session_decisions::tests::{aggregate, at, record};
    use biorouter::decision::log::{DecisionTurnMarkerItem, DecisionTurnSummary};
    use biorouter::decision::{DecisionKind, DecisionOrigin, Threshold};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn marker() -> DecisionTurnMarker {
        DecisionTurnMarker {
            turn_id: "t1".into(),
            items: vec![
                DecisionTurnMarkerItem {
                    origin: DecisionOrigin::Monitor,
                    origin_id: Some("form-reached".into()),
                    label: "Form reached".into(),
                    count: 18,
                    passed: 0,
                    failed: 18,
                    errors: 0,
                    last_pass_probability: Some(0.31),
                    fired: false,
                },
                DecisionTurnMarkerItem {
                    origin: DecisionOrigin::Tool,
                    origin_id: Some("call_1".into()),
                    label: "decide".into(),
                    count: 600,
                    passed: 41,
                    failed: 557,
                    errors: 2,
                    last_pass_probability: None,
                    fired: false,
                },
                DecisionTurnMarkerItem {
                    origin: DecisionOrigin::Monitor,
                    origin_id: Some("done".into()),
                    label: "Task done".into(),
                    count: 3,
                    passed: 1,
                    failed: 2,
                    errors: 0,
                    last_pass_probability: Some(0.91),
                    fired: true,
                },
            ],
        }
    }

    #[test]
    fn a_marker_is_one_line_per_origin_never_one_per_call() {
        let lines = text(&marker_lines(&marker()));
        assert_eq!(lines.len(), 3, "three origins, 621 calls: three lines");
        assert_eq!(
            lines[0],
            "◇ Monitor Form reached: 18 checks this turn · 18 failed · last P 0.31"
        );
        assert_eq!(
            lines[1],
            "◇ Decide: 600 calls this turn · 41 passed · 557 failed · 2 errors"
        );
        assert!(lines[2].ends_with("last P 0.91 · fired"), "{}", lines[2]);
    }

    fn card_record() -> DecisionRecord {
        let question = Question {
            instructions: "Does this note state the patient has MS?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Yes(0.8)),
        };
        let mut r = record("d1", 0.93);
        r.origin = DecisionOrigin::Chat;
        r.questions = serde_json::json!({ "has_ms": question });
        r.decisions[0].passed = Some(true);
        r.decisions[0].pass_probability = Some(0.93);
        r
    }

    #[test]
    fn a_decision_chat_reply_is_a_card_with_bars_and_the_answer() {
        let lines = text(&card_lines(&card_record()));
        assert_eq!(lines[0], "◆ Decision · tev1:0.8b · private · 22 ms");
        assert_eq!(lines[1], "  Does this note state the patient has MS?");
        assert!(lines[2].starts_with("  ▸ yes  "), "{}", lines[2]);
        assert!(lines[2].ends_with("0.93"), "{}", lines[2]);
        assert!(lines[3].starts_with("    no   "), "{}", lines[3]);
        assert_eq!(
            lines[4],
            "  yes · P 0.93 · calibrated · P(yes) 0.93 ≥ 0.80, passed"
        );

        let mut failed = card_record();
        failed.decisions.clear();
        failed.error = Some("ollama_decision did not answer within 10 s.".into());
        let lines = text(&card_lines(&failed));
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("did not answer"));
    }

    fn summary() -> SessionDecisionSummary {
        let mut tool = aggregate(DecisionOrigin::Tool, "decide", 600);
        tool.last_answer = Some("yes".into());
        SessionDecisionSummary {
            total: 618,
            aggregates: vec![aggregate(DecisionOrigin::Monitor, "Form reached", 18), tool],
            recent_turns: vec![DecisionTurnSummary {
                turn_id: "0193a1b2c3d4".into(),
                origin: DecisionOrigin::Monitor,
                origin_id: Some("form-reached".into()),
                label: "Form reached".into(),
                count: 18,
                passed: 0,
                failed: 18,
                errors: 0,
                first_at: at(0),
                last_at: at(60),
                last_pass_probability: Some(0.31),
            }],
        }
    }

    fn render(overlay: &DecisionsOverlay, w: u16, h: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| draw_overlay(f, overlay, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn the_overlay_lists_aggregates_and_turns_and_enter_asks_for_calls() {
        let mut overlay = DecisionsOverlay::new(summary());
        let screen = render(&overlay, 120, 20).join("\n");
        assert!(
            screen.contains("Decisions · ↑↓ select · ↵ last calls · esc close"),
            "{screen}"
        );
        assert!(screen.contains("618 calls in this chat"), "{screen}");
        assert!(screen.contains("▸ monitor  Form reached"), "{screen}");
        assert!(screen.contains("18 checks · 18 failed"), "{screen}");
        assert!(screen.contains("tool     decide"), "{screen}");
        assert!(screen.contains("Recent turns"), "{screen}");
        assert!(screen.contains("0193a1b2  monitor"), "{screen}");

        use crossterm::event::KeyCode;
        assert_eq!(on_key(&mut overlay, KeyCode::Down), OverlayKey::Consumed);
        assert_eq!(overlay.selected, 1);
        assert_eq!(on_key(&mut overlay, KeyCode::Down), OverlayKey::Consumed);
        assert_eq!(overlay.selected, 1, "the selection stops at the last row");
        on_key(&mut overlay, KeyCode::Enter);
        assert_eq!(overlay.pending_calls, Some(1));
        let query = DecisionsOverlay::calls_query(overlay.selected_aggregate().unwrap());
        assert_eq!(query.origin, Some(DecisionOrigin::Tool));
        assert_eq!(query.origin_id.as_deref(), Some("decide-id"));
        assert_eq!(query.limit, Some(LAST_CALLS));

        overlay.pending_calls = None;
        overlay.calls = Some(CallsView {
            label: calls_label(overlay.selected_aggregate().unwrap()),
            records: vec![record("d2", 0.31), record("d1", 0.9)],
            total: 600,
        });
        let screen = render(&overlay, 140, 20).join("\n");
        assert!(
            screen.contains("Decisions · last calls · esc back"),
            "{screen}"
        );
        assert!(
            screen.contains("tool · decide  last 2 of 600 calls"),
            "{screen}"
        );
        assert!(screen.contains("no  P 0.31"), "{screen}");
        assert_eq!(on_key(&mut overlay, KeyCode::Esc), OverlayKey::Consumed);
        assert!(
            overlay.calls.is_none(),
            "Esc goes back to the aggregates first"
        );
        assert_eq!(on_key(&mut overlay, KeyCode::Esc), OverlayKey::Closed);
    }

    #[test]
    fn an_empty_chat_says_it_has_no_calls() {
        let overlay = DecisionsOverlay::new(SessionDecisionSummary {
            total: 0,
            aggregates: vec![],
            recent_turns: vec![],
        });
        let screen = render(&overlay, 80, 10).join("\n");
        assert!(screen.contains("This chat has made no decision-model calls."));
        let mut overlay = overlay;
        on_key(&mut overlay, crossterm::event::KeyCode::Enter);
        assert_eq!(overlay.pending_calls, None, "nothing to open");
    }
}
