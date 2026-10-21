//! Interface de terminal interativa do instalador do devzymer.

pub mod state;
pub mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

pub use state::{Action, AgentRow, Choice, Outcome, Selection, Step};

/// O escopo de instalação oferecido na interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallScopeChoice {
    Project,
    User,
}

impl InstallScopeChoice {
    pub fn display(self) -> &'static str {
        match self {
            InstallScopeChoice::Project => "Projeto (apenas este repositório)",
            InstallScopeChoice::User => "Usuário (todos os repositórios)",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            InstallScopeChoice::Project => "projeto",
            InstallScopeChoice::User => "usuário",
        }
    }
}

/// True when the process is attached to an interactive terminal.
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Executa a interface e devolve a escolha do usuário.
///
/// Sem terminal interativo a função falha na hora. Um `event::poll` sem
/// entrada seria uma espera infinita, e um instalador que trava é pior do
/// que um instalador que não abre a interface.
pub fn run(selection: &mut Selection) -> io::Result<Outcome> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a interface precisa de um terminal interativo",
        ));
    }
    let mut terminal = setup()?;
    let mut guard = TerminalGuard;
    let result = event_loop(&mut terminal, selection);
    guard.restore(&mut terminal)?;
    result
}

/// Devolve o terminal ao estado anterior, mesmo em caso de erro.
///
/// O terminal fica em modo cru e na tela alternativa enquanto a interface
/// está aberta. Uma falha no meio do caminho não pode deixar o shell do
/// usuário quebrado, então a restauração acontece em `Drop`.
struct TerminalGuard;

impl TerminalGuard {
    fn restore(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn setup() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    selection: &mut Selection,
) -> io::Result<Outcome> {
    loop {
        terminal.draw(|frame| ui::render(frame, selection))?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(Outcome::Cancelled);
        }
        match translate(key) {
            Some(Action::Cancel) => return Ok(Outcome::Cancelled),
            Some(Action::Confirm) => {
                if selection.step == Step::Scope {
                    return Ok(selection.finish());
                }
                selection.handle(Action::Confirm);
            }
            Some(action) => selection.handle(action),
            None => {}
        }
    }
}

fn translate(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::Left => Some(Action::Left),
        KeyCode::Right => Some(Action::Right),
        KeyCode::Char(' ') => Some(Action::Toggle),
        KeyCode::Char('a') => Some(Action::ToggleAll),
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Cancel),
        _ => None,
    }
}
