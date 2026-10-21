//! Estado da seleção de agentes.
//!
//! A lógica fica separada da renderização para poder ser testada sem
//! terminal: as entradas e as saídas da interface são o que o agente
//! manipula.

use devzymer_integrations::{AgentDetection, AgentKind, AgentStatus};

use crate::InstallScopeChoice;

/// Uma ação vinda do teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Toggle,
    ToggleAll,
    Left,
    Right,
    Confirm,
    Cancel,
}

/// O que a interface produz.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// O usuário confirmou a escolha.
    Confirmed(Choice),
    /// O usuário cancelou.
    Cancelled,
}

/// A escolha final.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub agents: Vec<AgentKind>,
    pub scope: InstallScopeChoice,
}

/// A etapa em que a interface está.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Agents,
    Scope,
}

/// Uma linha da lista de agentes.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRow {
    pub kind: AgentKind,
    pub label: String,
    pub status: AgentStatus,
    pub selected: bool,
    pub available: bool,
}

/// O estado completo da interface.
#[derive(Debug, Clone)]
pub struct Selection {
    pub rows: Vec<AgentRow>,
    pub cursor: usize,
    pub step: Step,
    pub scope_cursor: usize,
    pub scopes: Vec<InstallScopeChoice>,
}

impl Selection {
    /// Cria a seleção a partir do que foi detectado.
    ///
    /// Os agentes já configurados entram marcados. Sem nenhum agente
    /// configurado, o Claude Code entra marcado, que é o comportamento
    /// histórico do `devzymer init`.
    pub fn new(detections: &[AgentDetection]) -> Selection {
        let configured: Vec<AgentKind> = detections
            .iter()
            .filter(|d| d.status == AgentStatus::Configured || d.status == AgentStatus::Connected)
            .map(|d| d.agent)
            .collect();
        let default = if configured.is_empty() {
            vec![AgentKind::Claude]
        } else {
            configured
        };
        let rows = detections
            .iter()
            .map(|d| AgentRow {
                kind: d.agent,
                label: d.agent.display().to_string(),
                status: d.status,
                selected: default.contains(&d.agent),
                available: d.is_available(),
            })
            .collect();
        Selection {
            rows,
            cursor: 0,
            step: Step::Agents,
            scope_cursor: 0,
            scopes: vec![InstallScopeChoice::Project, InstallScopeChoice::User],
        }
    }

    /// Aplica uma ação.
    pub fn handle(&mut self, action: Action) {
        match self.step {
            Step::Agents => self.handle_agents(action),
            Step::Scope => self.handle_scope(action),
        }
    }

    fn handle_agents(&mut self, action: Action) {
        match action {
            Action::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            Action::Down => {
                if self.cursor + 1 < self.rows.len() {
                    self.cursor += 1;
                }
            }
            Action::Toggle => {
                if let Some(row) = self.rows.get_mut(self.cursor) {
                    row.selected = !row.selected;
                }
            }
            Action::ToggleAll => {
                let all = self.rows.iter().all(|row| row.selected);
                for row in self.rows.iter_mut() {
                    row.selected = !all;
                }
            }
            Action::Confirm => self.step = Step::Scope,
            Action::Cancel => {}
            Action::Left | Action::Right => {}
        }
    }

    fn handle_scope(&mut self, action: Action) {
        match action {
            Action::Left => {
                if self.scope_cursor > 0 {
                    self.scope_cursor -= 1;
                }
            }
            Action::Right => {
                if self.scope_cursor + 1 < self.scopes.len() {
                    self.scope_cursor += 1;
                }
            }
            Action::Up | Action::Down => {
                if self.scope_cursor + 1 < self.scopes.len() {
                    self.scope_cursor += 1;
                } else {
                    self.scope_cursor = 0;
                }
            }
            Action::Toggle => {
                if self.scope_cursor + 1 < self.scopes.len() {
                    self.scope_cursor += 1;
                } else {
                    self.scope_cursor = 0;
                }
            }
            Action::ToggleAll => {}
            Action::Cancel => self.step = Step::Agents,
            Action::Confirm => {}
        }
    }

    /// Os agentes marcados.
    pub fn selected(&self) -> Vec<AgentKind> {
        self.rows
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.kind)
            .collect()
    }

    /// Quantos agentes estão marcados.
    pub fn selected_count(&self) -> usize {
        self.rows.iter().filter(|row| row.selected).count()
    }

    /// O escopo escolhido.
    pub fn scope(&self) -> InstallScopeChoice {
        self.scopes
            .get(self.scope_cursor)
            .copied()
            .unwrap_or(InstallScopeChoice::Project)
    }

    /// Produz o resultado quando a confirmação acontece.
    pub fn finish(&self) -> Outcome {
        let agents = self.selected();
        if agents.is_empty() {
            return Outcome::Cancelled;
        }
        Outcome::Confirmed(Choice {
            agents,
            scope: self.scope(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devzymer_integrations::{AgentDetection, AgentStatus};
    use std::path::PathBuf;

    fn detection(kind: AgentKind, status: AgentStatus) -> AgentDetection {
        AgentDetection {
            agent: kind,
            status,
            executable: Some(PathBuf::from("agente")),
            config_path: None,
            registered_at: None,
            notes: Vec::new(),
        }
    }

    fn sample() -> Selection {
        Selection::new(&[
            detection(AgentKind::Claude, AgentStatus::Detected),
            detection(AgentKind::Codex, AgentStatus::Detected),
            detection(AgentKind::OpenCode, AgentStatus::Detected),
        ])
    }

    #[test]
    fn the_claude_code_starts_selected() {
        let selection = sample();
        assert_eq!(selection.selected(), vec![AgentKind::Claude]);
        assert_eq!(selection.selected_count(), 1);
    }

    #[test]
    fn an_installed_agent_starts_selected() {
        let selection = Selection::new(&[
            detection(AgentKind::Claude, AgentStatus::Detected),
            detection(AgentKind::Codex, AgentStatus::Configured),
        ]);
        assert_eq!(selection.selected(), vec![AgentKind::Codex]);
    }

    #[test]
    fn space_toggles_the_agent_under_the_cursor() {
        let mut selection = sample();
        selection.handle(Action::Down);
        selection.handle(Action::Toggle);
        assert_eq!(selection.selected_count(), 2);
        selection.handle(Action::Toggle);
        assert_eq!(selection.selected_count(), 1);
    }

    #[test]
    fn select_all_marks_every_agent() {
        let mut selection = sample();
        selection.handle(Action::ToggleAll);
        assert_eq!(selection.selected_count(), 3);
        selection.handle(Action::ToggleAll);
        assert_eq!(selection.selected_count(), 0);
    }

    #[test]
    fn the_cursor_stays_inside_the_list() {
        let mut selection = sample();
        selection.handle(Action::Up);
        assert_eq!(selection.cursor, 0);
        for _ in 0..5 {
            selection.handle(Action::Down);
        }
        assert_eq!(selection.cursor, 2);
    }

    #[test]
    fn the_confirm_leads_to_the_scope_step() {
        let mut selection = sample();
        selection.handle(Action::Confirm);
        assert_eq!(selection.step, Step::Scope);
        assert_eq!(selection.scope(), InstallScopeChoice::Project);
    }

    #[test]
    fn the_scope_is_chosen_with_the_arrows() {
        let mut selection = sample();
        selection.handle(Action::Confirm);
        selection.handle(Action::Right);
        assert_eq!(selection.scope(), InstallScopeChoice::User);
    }

    #[test]
    fn a_selection_without_agents_is_cancelled() {
        let mut selection = sample();
        selection.handle(Action::ToggleAll);
        selection.handle(Action::ToggleAll);
        assert!(selection.selected().is_empty());
        assert_eq!(selection.finish(), Outcome::Cancelled);
    }

    #[test]
    fn the_choice_carries_the_agents_and_the_scope() {
        let mut selection = sample();
        selection.handle(Action::Down);
        selection.handle(Action::Toggle);
        selection.handle(Action::Confirm);
        selection.handle(Action::Right);
        let outcome = selection.finish();
        match outcome {
            Outcome::Confirmed(choice) => {
                assert_eq!(choice.agents.len(), 2);
                assert_eq!(choice.scope, InstallScopeChoice::User);
            }
            Outcome::Cancelled => panic!("a seleção deveria ter sido confirmada"),
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn selection() -> Selection {
        let rows = vec![
            AgentRow {
                kind: AgentKind::Claude,
                label: "Claude Code".to_string(),
                status: AgentStatus::Detected,
                selected: true,
                available: true,
            },
            AgentRow {
                kind: AgentKind::Codex,
                label: "Codex".to_string(),
                status: AgentStatus::NotInstalled,
                selected: false,
                available: false,
            },
        ];
        Selection {
            rows,
            cursor: 0,
            step: Step::Agents,
            scope_cursor: 0,
            scopes: vec![InstallScopeChoice::Project, InstallScopeChoice::User],
        }
    }

    fn draw(selection: &Selection) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, selection))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn the_brand_and_the_agents_are_drawn() {
        let text = draw(&selection());
        assert!(text.contains("DEVZYMER"), "a marca deve aparecer");
        assert!(text.contains("Less Tokens. More Engineering."));
        assert!(text.contains("Claude Code"));
        assert!(text.contains("Codex"));
    }

    #[test]
    fn the_selection_count_is_drawn() {
        let text = draw(&selection());
        assert!(
            text.contains("Selecionados: 1 de 2"),
            "o total deve aparecer"
        );
    }

    #[test]
    fn the_scope_step_is_drawn() {
        let mut selection = selection();
        selection.step = Step::Scope;
        let text = draw(&selection);
        assert!(text.contains("Escopo da instalação"));
        assert!(text.contains("Projeto"));
        assert!(text.contains("Usuário"));
    }

    #[test]
    fn a_terminal_pequeno_nao_quebra_o_desenho() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &selection()))
            .unwrap();
    }
}
