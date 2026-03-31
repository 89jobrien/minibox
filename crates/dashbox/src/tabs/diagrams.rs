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
use crate::diagrams;

// ── Layout constants ─────────────────────────────────────────────────────────

/// Inner label width for each node box.
const NODE_INNER: usize = 10;
/// Total box width: ┌ + (NODE_INNER+2)×─ + ┐
#[allow(dead_code)]
const BOX_W: usize = NODE_INNER + 4;
/// Width of the connector string between two nodes in the same row.
const CONN_W: usize = 9;

// ── View cycling ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    CiFlow,
    DevLoop,
    ContainerLifecycle,
    ImagePull,
    AdapterSuite,
    WorkspaceDeps,
}

impl View {
    const ALL: [View; 6] = [
        View::CiFlow,
        View::DevLoop,
        View::ContainerLifecycle,
        View::ImagePull,
        View::AdapterSuite,
        View::WorkspaceDeps,
    ];

    fn name(self) -> &'static str {
        match self {
            View::CiFlow => "CI Flow",
            View::DevLoop => "Dev Loop",
            View::ContainerLifecycle => "Container Lifecycle",
            View::ImagePull => "Image Pull",
            View::AdapterSuite => "Adapter Suite",
            View::WorkspaceDeps => "Workspace Deps",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&v| v == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let n = Self::ALL.len();
        Self::ALL[(self.index() + n - 1) % n]
    }
}

// ── Tab state ────────────────────────────────────────────────────────────────

pub struct DiagramsTab {
    view: View,
    ci_flow: Diagram,
    dev_loop: Diagram,
    container_lifecycle: Diagram,
    image_pull: Diagram,
    adapter_suite: Diagram,
    workspace_deps: Diagram,
    selected: usize,
    ci_source: CachedSource<CiSource>,
}

impl DiagramsTab {
    pub fn new() -> Self {
        let ci_flow = diagrams::ci_flow();
        let first = ci_flow.first_node().unwrap_or(0);
        Self {
            view: View::CiFlow,
            ci_flow,
            dev_loop: diagrams::dev_loop(),
            container_lifecycle: diagrams::container_lifecycle(),
            image_pull: diagrams::image_pull(),
            adapter_suite: diagrams::adapter_suite(),
            workspace_deps: diagrams::workspace_deps(),
            selected: first,
            ci_source: CachedSource::new(CiSource::new(), 30),
        }
    }

    fn active_diagram(&self) -> &Diagram {
        match self.view {
            View::CiFlow => &self.ci_flow,
            View::DevLoop => &self.dev_loop,
            View::ContainerLifecycle => &self.container_lifecycle,
            View::ImagePull => &self.image_pull,
            View::AdapterSuite => &self.adapter_suite,
            View::WorkspaceDeps => &self.workspace_deps,
        }
    }

    fn go_to_view(&mut self, v: View) {
        self.view = v;
        self.selected = self.active_diagram().first_node().unwrap_or(0);
    }

    /// Live CI status overlay — only populated for the CI Flow diagram.
    fn build_statuses(&mut self) -> HashMap<usize, NodeStatus> {
        if self.view != View::CiFlow {
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
        // Node IDs in ci_flow: main=1, next=2, stable=3
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
fn connector_str(edge: &Edge, width: usize) -> String
where
    Edge: Sized,
{
    let fill = match edge.style {
        EdgeStyle::Solid => '─',
        EdgeStyle::Dashed => '-',
        EdgeStyle::Manual => '·',
    };
    let lbl = edge.label.unwrap_or("");
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
    diagram: &Diagram,
    selected: usize,
    statuses: &HashMap<usize, NodeStatus>,
    is_continuation: bool,
) -> [Line<'static>; 3] {
    let mut tops: Vec<Span<'static>> = Vec::new();
    let mut mids: Vec<Span<'static>> = Vec::new();
    let mut bots: Vec<Span<'static>> = Vec::new();

    // Indent: 2 spaces, with the ↳ marker on the mid line for continuations.
    tops.push(Span::raw("  "));
    mids.push(if is_continuation {
        Span::styled("↳ ", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw("  ")
    });
    bots.push(Span::raw("  "));

    let gap: String = " ".repeat(CONN_W);

    for (i, &nid) in row.iter().enumerate() {
        // Connector before each node after the first
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

        let label = center_label(node.label, NODE_INNER);
        // ┌──────────────┐  (NODE_INNER+2 dashes)
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
    diagram: &Diagram,
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
    diagram: &Diagram,
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

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        node.label.to_string(),
        Style::default().fg(kind_col).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled(
            node.kind.label().to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(status_str.to_string(), Style::default().fg(status_color)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::raw(node.detail.to_string())));
    lines.push(Line::raw(""));

    let outgoing = diagram.outgoing(selected);
    if !outgoing.is_empty() {
        lines.push(Line::from(Span::styled(
            "edges".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        for edge in outgoing {
            let target = diagram.node(edge.to).map(|n| n.label).unwrap_or("?");
            let arrow = match edge.style {
                EdgeStyle::Solid => "──►",
                EdgeStyle::Dashed => "─ ►",
                EdgeStyle::Manual => "··►",
            };
            let text = match edge.label {
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
        let view = self.view;

        let chunks = Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(area);

        // ── Left: diagram canvas ──────────────────────────────────────────
        let idx = view.index() + 1;
        let total = View::ALL.len();
        let title = format!(" {} ({idx}/{total})  d/D:cycle ", view.name());
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
        // capture navigate before the immutable borrow below
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
            KeyCode::Char('d') => self.go_to_view(self.view.next()),
            KeyCode::Char('D') => self.go_to_view(self.view.prev()),
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
