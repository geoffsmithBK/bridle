//! Wiring: flags, the agent thread, and the terminal event loop.

use bridle::agent::{Agent, AgentOptions, DEFAULT_MAX_STEPS};
use bridle::config;
use bridle::events::{AgentEvent, Command};
use bridle::llm::client::LlamaClient;
use bridle::tools::{ToolContext, guard};
use bridle::ui::{app::App, input, render};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use ratatui::crossterm::execute;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let config = match config::parse(std::env::args().skip(1)) {
        Ok(Some(c)) => c,
        Ok(None) => {
            println!("{}", config::USAGE);
            return Ok(());
        }
        Err(e) => {
            eprintln!("bridle: {e}\n\n{}", config::USAGE);
            std::process::exit(2);
        }
    };

    let root = std::env::current_dir()?;
    // Check --continue / --resume before taking over the terminal, so a
    // missing session is a plain error message.
    let resume = match &config.resume {
        None => None,
        Some(r) => match bridle::session::resolve(&root, r) {
            Ok(stamp) => Some(stamp),
            Err(e) => {
                eprintln!("bridle: {e}");
                std::process::exit(1);
            }
        },
    };
    let shim_dir = guard::make_shim_dir()?;
    let ctx = ToolContext { cwd: root.clone(), project_root: root.clone(), shim_dir: shim_dir.clone() };

    let (cmd_tx, cmd_rx) = channel();
    let (ev_tx, ev_rx) = channel();
    let llm = LlamaClient::new(&config.url, config.wire_raw);
    let options = AgentOptions { model: config.model, log: true, max_steps: DEFAULT_MAX_STEPS, resume };
    let agent = Agent::new(llm, options, ctx, ev_tx, cmd_rx);
    let agent_thread = std::thread::spawn(move || agent.run());

    let mut app = App::new(root);
    let mut terminal = ratatui::init(); // also installs a panic hook that restores the terminal
    execute!(std::io::stdout(), EnableBracketedPaste)?;
    let result = run_ui(&mut terminal, &mut app, &cmd_tx, &ev_rx);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();

    // Stop whatever is running (e.g. kill a bash process group), then quit.
    let _ = cmd_tx.send(Command::Interrupt);
    let _ = cmd_tx.send(Command::Quit);
    let deadline = Instant::now() + Duration::from_secs(1);
    while !agent_thread.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = std::fs::remove_dir_all(shim_dir);
    result
}

fn run_ui(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    commands: &Sender<Command>,
    events: &Receiver<AgentEvent>,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| render::draw(f, app))?;
        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(command) = input::handle_key(app, key) {
                        let _ = commands.send(command);
                    }
                }
                Event::Paste(text) => input::handle_paste(app, &text),
                _ => {}
            }
        }
        while let Ok(event) = events.try_recv() {
            app.on_agent_event(event);
        }
        if app.quit {
            return Ok(());
        }
    }
}
