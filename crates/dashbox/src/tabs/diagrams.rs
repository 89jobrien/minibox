// dashbox/src/tabs/diagrams.rs

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::ci::CiSource;
use crate::diagram::{EdgeStyle, NavDir, NodeKind, NodeStatus, OwnedDiagram};

// ── Layout constants ─────────────────────────────────────────────────────────

/// Inner label width for each node box.
const NODE_INNER: usize = 10;
/// Total box width: ┌ + (NODE_INNER+2)×─ + ┐
#[allow(dead_code)]
const BOX_W: usize = NODE_INNER + 4;
/// Width of the connector string between two nodes in the same row.
const CONN_W: usize = 9;

// ── Tab state ────────────────────────────────────────────────────────────────

pub struct DiagramsTab {
    diagrams: Vec<OwnedDiagram>,
    active: usize,
    selected: usize,
    ci_source: CachedSource<CiSource>,
}

impl DiagramsTab {
    pub fn new(extra: Vec<OwnedDiagram>) -> Self {
        let mut diagrams = crate::diagrams::built_in_diagrams();
        diagrams.extend(extra);
        let first = diagrams.first().and_then(|d| d.first_node()).unwrap_or(0);
        Self {
            diagrams,
            active: 0,
            selected: first,
            ci_source: CachedSource::new(CiSource::new(), 30),
        }
    }

    fn active_diagram(&self) -> &OwnedDiagram {
        &self.diagrams[self.active]
    }

    fn go_to_index(&mut self, idx: usize) {
        self.active = idx;
        self.selected = self.active_diagram().first_node().unwrap_or(0);
    }

    fn next_diagram(&mut self) {
        let next = (self.active + 1) % self.diagrams.len();
        self.go_to_index(next);
    }

    fn prev_diagram(&mut self) {
        let n = self.diagrams.len();
        let prev = (self.active + n - 1) % n;
        self.go_to_index(prev);
    }

    /// Live CI status overlay — only populated for the first diagram (CI Flow, index 0).
    /// Node IDs in ci_flow.mmd: feature=0, main=1, next=2, stable=3, vtag=4
    fn build_statuses(&mut self) -> HashMap<usize, NodeStatus> {
        if self.active != 0 {
            return HashMap::new();
        }
        self.ci_source.ensure_fresh();
        let ci_data = match self.ci_source.get() {
            Some(Ok(d)) => d,
            _ => return HashMap::new(),
        };
        let mut branch_status: HashMap<&str, NodeStatus> = HashMap::new();
        for run in &ci_data.runs {
            let status = if run.status == "completed" {
                match run.conclusion.as_str() {
                    "success" => NodeStatus::Passing,
                    "failure" => NodeStatus::Failing,
                    _ => NodeStatus::Unknown,
                }
            } else if run.status == "in_progress" {
                NodeStatus::Running
            } else {
                NodeStatus::Unknown
            };
            branch_status
                .entry(run.head_branch.as_str())
                .or_insert(status);
        }
        // feature=0, main=1, next=2, stable=3, vtag=4
        [("main", 1usize), ("next", 2), ("stable", 3)]
            .iter()
            .filter_map(|&(branch, id)| branch_status.get(branch).map(|&s| (id, s)))
            .collect()
    }
}

// ── Render helpers ────────────────────────────────────────────────────────────

fn kind_color(kind: NodeKind) -> Color {
    match kind {
        NodeKind::Branch => Color::Cyan,
        NodeKind::Job => Color::Yellow,
        NodeKind::Command => Color::Green,
        NodeKind::Artifact => Color::Magenta,
    }
}

fn status_color(status: NodeStatus, kind: NodeKind) -> Color {
    match status {
        NodeStatus::Unknown => kind_color(kind),
        NodeStatus::Passing => Color::Green,
        NodeStatus::Failing => Color::Red,
        NodeStatus::Running => Color::Yellow,
    }
}

/// Center `text` within `width` display columns, truncating if too long.
fn center_label(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len().min(width);
    let label: String = chars[..len].iter().collect();
    let pad = width - len;
    let left = pad / 2;
    let right = pad - left;
    format!("{}{}{}", " ".repeat(left), label, " ".repeat(right))
}

/// Build a connector string of exactly `width` display columns.
fn connector_str(edge: &crate::diagram::OwnedEdge, width: usize) -> String {
    let fill = match edge.style {
        EdgeStyle::Solid => '─',
        EdgeStyle::Dashed => '-',
        EdgeStyle::Manual => '·',
    };
    let lbl = edge.label.as_deref().unwrap_or("");
    let lbl_len = lbl.chars().count();
    let fill_total = width.saturating_sub(1 + lbl_len);
    let left = fill_total / 2;
    let right = fill_total - left;
    let mut s = String::new();
    for _ in 0..left {
        s.push(fill);
    }
    s.push_str(lbl);
    for _ in 0..right {
        s.push(fill);
    }
    s.push('►');
    s
}

fn connector_color(style: EdgeStyle) -> Color {
    match style {
        EdgeStyle::Solid => Color::DarkGray,
        EdgeStyle::Dashed => Color::DarkGray,
        EdgeStyle::Manual => Color::Yellow,
    }
}

/// Render one row of nodes as three `Line`s: top borders, labels, bottom borders.
fn render_row_lines(
    row: &[usize],
    diagram: &OwnedDiagram,
    selected: usize,
    statuses: &HashMap<usize, NodeStatus>,
    is_continuation: bool,
) -> [Line<'static>; 3] {
    let mut tops: Vec<Span<'static>> = Vec::new();
    let mut mids: Vec<Span<'static>> = Vec::new();
    let mut bots: Vec<Span<'static>> = Vec::new();

    tops.push(Span::raw("  "));
    mids.push(if is_continuation {
        Span::styled("↳ ", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw("  ")
    });
    bots.push(Span::raw("  "));

    let gap: String = " ".repeat(CONN_W);

    for (i, &nid) in row.iter().enumerate() {
        if i > 0 {
            let prev = row[i - 1];
            if let Some(edge) = diagram.edge_between(prev, nid) {
                let conn = connector_str(edge, CONN_W);
                let color = connector_color(edge.style);
                tops.push(Span::raw(gap.clone()));
                mids.push(Span::styled(conn, Style::default().fg(color)));
                bots.push(Span::raw(gap.clone()));
            } else {
                tops.push(Span::raw(gap.clone()));
                mids.push(Span::raw(gap.clone()));
                bots.push(Span::raw(gap.clone()));
            }
        }

        let node = match diagram.node(nid) {
            Some(n) => n,
            None => continue,
        };
        let status = statuses.get(&nid).copied().unwrap_or_default();
        let color = status_color(status, node.kind);

        let style = if nid == selected {
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };

        let label = center_label(&node.label, NODE_INNER);
        let top = format!("┌{:─<w$}┐", "", w = NODE_INNER + 2);
        let mid = format!("│ {label} │");
        let bot = format!("└{:─<w$}┘", "", w = NODE_INNER + 2);

        tops.push(Span::styled(top, style));
        mids.push(Span::styled(mid, style));
        bots.push(Span::styled(bot, style));
    }

    [Line::from(tops), Line::from(mids), Line::from(bots)]
}

fn diagram_lines(
    diagram: &OwnedDiagram,
    selected: usize,
    statuses: &HashMap<usize, NodeStatus>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (row_idx, row) in diagram.layout.iter().enumerate() {
        if row_idx > 0 {
            let prev = &diagram.layout[row_idx - 1];
            let is_cont = diagram.has_cross_row_edge(prev, row);
            lines.push(Line::raw(""));
            let [t, m, b] = render_row_lines(row, diagram, selected, statuses, is_cont);
            lines.extend([t, m, b]);
        } else {
            let [t, m, b] = render_row_lines(row, diagram, selected, statuses, false);
            lines.extend([t, m, b]);
        }
    }

    lines
}

fn detail_lines(
    diagram: &OwnedDiagram,
    selected: usize,
    statuses: &HashMap<usize, NodeStatus>,
) -> Vec<Line<'static>> {
    let node = match diagram.node(selected) {
        Some(n) => n,
        None => return vec![],
    };
    let status = statuses.get(&selected).copied().unwrap_or_default();

    let (status_str, status_color) = match status {
        NodeStatus::Unknown => ("", Color::DarkGray),
        NodeStatus::Passing => (" ✓ passing", Color::Green),
        NodeStatus::Failing => (" ✗ failing", Color::Red),
        NodeStatus::Running => (" ⏳ running", Color::Yellow),
    };
    let kind_col = kind_color(node.kind);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            node.label.clone(),
            Style::default().fg(kind_col).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                node.kind.label().to_string(),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(status_str.to_string(), Style::default().fg(status_color)),
        ]),
        Line::raw(""),
        Line::from(Span::raw(node.detail.clone())),
        Line::raw(""),
    ];

    let outgoing = diagram.outgoing(selected);
    if !outgoing.is_empty() {
        lines.push(Line::from(Span::styled(
            "edges".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        for edge in outgoing {
            let target = diagram
                .node(edge.to)
                .map(|n| n.label.as_str())
                .unwrap_or("?");
            let arrow = match edge.style {
                EdgeStyle::Solid => "──►",
                EdgeStyle::Dashed => "─ ►",
                EdgeStyle::Manual => "··►",
            };
            let text = match &edge.label {
                Some(lbl) => format!(" {arrow} {target}  ({lbl})"),
                None => format!(" {arrow} {target}"),
            };
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(Color::Gray),
            )));
        }
    }

    lines
}

fn legend_line() -> Line<'static> {
    Line::from(vec![
        Span::styled("─────────► ", Style::default().fg(Color::DarkGray)),
        Span::styled("auto  ", Style::default().fg(Color::DarkGray)),
        Span::styled("─--------► ", Style::default().fg(Color::DarkGray)),
        Span::styled("release  ", Style::default().fg(Color::DarkGray)),
        Span::styled("·········► ", Style::default().fg(Color::Yellow)),
        Span::styled("manual  ", Style::default().fg(Color::DarkGray)),
        Span::styled("⎇ branch  ", Style::default().fg(Color::Cyan)),
        Span::styled("⚙ job  ", Style::default().fg(Color::Yellow)),
        Span::styled("$ cmd  ", Style::default().fg(Color::Green)),
        Span::styled("◈ artifact", Style::default().fg(Color::Magenta)),
    ])
}

// ── TabRenderer ──────────────────────────────────────────────────────────────

impl TabRenderer for DiagramsTab {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let statuses = self.build_statuses();
        let diagram = self.active_diagram();
        let selected = self.selected;
        let active = self.active;
        let total = self.diagrams.len();

        let chunks = Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(area);

        // ── Left: diagram canvas ──────────────────────────────────────────
        let title = format!(" {} ({}/{})  d/D:cycle ", diagram.name, active + 1, total);
        let left_block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        let inner_left = left_block.inner(chunks[0]);
        frame.render_widget(left_block, chunks[0]);

        let mut all_lines: Vec<Line<'static>> = Vec::new();
        all_lines.push(Line::raw(""));
        all_lines.extend(diagram_lines(diagram, selected, &statuses));
        all_lines.push(Line::raw(""));
        all_lines.push(legend_line());

        frame.render_widget(Paragraph::new(all_lines), inner_left);

        // ── Right: node detail ────────────────────────────────────────────
        let right_block = Block::default()
            .borders(Borders::ALL)
            .title(" Node ")
            .title_style(Style::default().fg(Color::DarkGray));
        let inner_right = right_block.inner(chunks[1]);
        frame.render_widget(right_block, chunks[1]);

        frame.render_widget(
            Paragraph::new(detail_lines(diagram, selected, &statuses)).wrap(Wrap { trim: true }),
            inner_right,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> TabAction {
        let nav_result = {
            let diagram = self.active_diagram();
            match key.code {
                KeyCode::Left | KeyCode::Char('h') => diagram.navigate(self.selected, NavDir::Left),
                KeyCode::Right | KeyCode::Char('l') => {
                    diagram.navigate(self.selected, NavDir::Right)
                }
                KeyCode::Up | KeyCode::Char('k') => diagram.navigate(self.selected, NavDir::Up),
                KeyCode::Down | KeyCode::Char('j') => diagram.navigate(self.selected, NavDir::Down),
                KeyCode::Tab => Some(diagram.next_node(self.selected)),
                KeyCode::BackTab => Some(diagram.prev_node(self.selected)),
                _ => None,
            }
        };

        match key.code {
            KeyCode::Char('d') => self.next_diagram(),
            KeyCode::Char('D') => self.prev_diagram(),
            _ => {
                if let Some(nid) = nav_result {
                    self.selected = nid;
                }
            }
        }

        TabAction::None
    }

    fn refresh(&mut self) {
        self.ci_source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "h/l:move  j/k:row  Tab:next  d/D:diagram"
    }
}
