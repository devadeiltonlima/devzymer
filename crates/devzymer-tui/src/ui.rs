//! Desenho da interface de seleção.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::brand::{self, Caps};
use crate::state::{Selection, Step};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

/// Desenha a interface no quadro atual.
pub fn render(frame: &mut Frame, selection: &Selection) {
    let plain = Caps {
        color: false,
        truecolor: false,
        unicode: true,
        animate: false,
    };
    render_animated(frame, selection, 0, &plain);
}

/// Desenha a interface com o banner animado quando o terminal comporta.
pub fn render_animated(frame: &mut Frame, selection: &Selection, tick: u64, caps: &Caps) {
    let area = frame.area();
    let wide = caps.color && usize::from(area.width) > brand::BANNER_WIDTH && area.height >= 24;
    let brand_height = if wide {
        (brand::BANNER_ROWS + 3) as u16
    } else {
        4
    };
    let blocks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(brand_height),
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);

    if wide {
        frame.render_widget(
            Paragraph::new(brand::banner_lines(caps, tick)).alignment(Alignment::Center),
            blocks[0],
        );
    } else {
        render_brand(frame, blocks[0]);
    }
    match selection.step {
        Step::Agents => render_agents(frame, blocks[1], selection),
        Step::Scope => render_scope(frame, blocks[1], selection),
    }
    render_footer(frame, blocks[2], selection);
    render_help(frame, blocks[3], selection);
}

fn render_brand(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT));
    let lines = vec![
        Line::from(Span::styled(
            "DEVZYMER",
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::ITALIC),
        )),
        Line::from(Span::styled(
            "Less Tokens. More Engineering.",
            Style::default().fg(DIM),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_agents(frame: &mut Frame, area: Rect, selection: &Selection) {
    let items: Vec<ListItem> = selection
        .rows
        .iter()
        .map(|row| {
            let mark = if row.selected { "[✓]" } else { "[ ]" };
            let status = row.status.display();
            let style = if row.available {
                Style::default()
            } else {
                Style::default().fg(DIM)
            };
            let label = if row.available {
                row.label.clone()
            } else {
                format!("{} (não encontrado)", row.label)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), style),
                Span::styled(format!("{label:<16}"), style),
                Span::styled(status, Style::default().fg(status_color(row))),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Selecione os agentes de código "),
        )
        .highlight_style(
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(28, 30, 34)),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    state.select(Some(selection.cursor));
    frame.render_stateful_widget(list, area, &mut state);
}

fn status_color(row: &crate::state::AgentRow) -> Color {
    match row.status {
        devzymer_integrations::AgentStatus::Connected => Color::Green,
        devzymer_integrations::AgentStatus::Configured => Color::Cyan,
        devzymer_integrations::AgentStatus::Detected => Color::Yellow,
        devzymer_integrations::AgentStatus::NotInstalled => DIM,
    }
}

fn render_scope(frame: &mut Frame, area: Rect, selection: &Selection) {
    let mut lines = vec![Line::from(Span::styled(
        "Escolha o escopo da instalação:",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for (index, scope) in selection.scopes.iter().enumerate() {
        let active = index == selection.scope_cursor;
        let mark = if active { ">" } else { " " };
        let style = if active {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} "), style),
            Span::styled(scope.display(), style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "O índice local continua isolado por repositório em qualquer escopo.",
        Style::default().fg(DIM),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Escopo da instalação "),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, selection: &Selection) {
    let text = match selection.step {
        Step::Agents => format!(
            "Selecionados: {} de {}    [ Enter: continuar ]    [ Esc: cancelar ]",
            selection.selected_count(),
            selection.rows.len()
        ),
        Step::Scope => {
            " [ Enter: instalar ]    [ Esc: voltar ]    [ ←/→: trocar escopo ]".to_string()
        }
    };
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn render_help(frame: &mut Frame, area: Rect, selection: &Selection) {
    let help = match selection.step {
        Step::Agents => {
            " ↑/↓ mover   espaço marcar   a marcar todos   enter continuar   esc cancelar"
        }
        Step::Scope => " ←/→ escopo   enter instalar   esc voltar",
    };
    frame.render_widget(
        Paragraph::new(Span::styled(help, Style::default().fg(DIM))),
        area,
    );
}
