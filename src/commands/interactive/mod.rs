mod events;

use std::fs::File;
use std::io::{self, Read, Stdout};
use std::path::PathBuf;

use termion::event::Key;
use termion::raw::{IntoRawMode, RawTerminal};
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Modifier, Style};
use tui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use tui::Terminal;

use crate::opt::Opt;
use crate::taskwarrior::Task;
use events::{Event, Events};

// NOTE: if you're here looking at this code
// and you're thinking to yourself
// "hey this is awful"
// well congratulations, you're in good company.
//
// this is in that early early early stage
// of programming where you're trying to explore
// whatever it is that you want to make
//
// bear with me as it continues to be ugly (for now)

// !!!feature brainstorm!!!
//
// - modal UI; fewer keystrokes = faster interaction.
//   after using a UI for a while, you can learn how to interact
//   so you no longer need to type out full commands
//   - normal mode
//     - up + down to nagivate between notes
//     - enter to open up $EDITOR on the taskn note
//     - "m" or "e" to enter modify/edit mode (which one?)
//     - "a" to enter add mode
//   - edit mode
//     - ESC / Ctrl-F to exit edit more
//     - r to toggle +remindme tag
//     - e change the estimate
//     - u to change urgency
//     - p to change project
//     - t to add/remove tag (normal +/- taskwarrior syntax)
//
// - try to build out a joint estimate + urgency ordering system
//   so that tasks have a consistent order and i can capture
//   top-to-bottom
//
// - preview taskn notes when you select a task
//
// let's think about state transitions a little bit more:
//   - some central state concept (CommonState) which things can mutate
//   - a way to build a CommonState from TaskWarrior
//   - and a way to save CommonState to TaskWarrior
//   - sub-state that represents the current mode + additional state associated with that mode
//   - each action produces:
//     - a sub-state (so we can transition)
//     - whether we need to reload entirely
//     - whether we need to flush state

type Term = Terminal<TermionBackend<RawTerminal<Stdout>>>;

pub fn execute(opt: Opt) -> io::Result<()> {
    let stdout = io::stdout().into_raw_mode()?;
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut taskwarrior_args = opt.args.clone();
    taskwarrior_args.push("(status:pending or status:waiting)".to_string());

    // clear screen
    println!("\0{}[2J", 27 as char);

    let events = Events::new();
    let mut common_state = CommonState::load_from_taskwarrior()?;
    let mut mode = Mode::Normal(NormalState::default());
    loop {
        mode.render(&opt, &mut terminal, &common_state);
        match events.next()? {
            Event::Key(key) => match key {
                Key::Ctrl('c') => break,
                key => {
                    let result = mode.handle_key(&opt, &mut common_state, key)?;
                    mode = result.new_mode;
                    if result.should_flush {
                        common_state = common_state.flush_to_taskwarrior()?;
                    } else if result.should_load {
                        common_state = CommonState::load_from_taskwarrior()?;
                    }
                }
            },
            Event::Resize => continue,
        }
    }

    Ok(())
}

struct CommonState {
    tasks: Vec<Task>,
}

impl CommonState {
    fn load_from_taskwarrior() -> io::Result<Self> {
        let mut tasks = Task::get(["status:pending"].iter())?;
        tasks.sort_by(|a, b| a.estimate.partial_cmp(&b.estimate).unwrap());
        Ok(CommonState { tasks })
    }

    fn flush_to_taskwarrior(self) -> io::Result<Self> {
        for (order, mut task) in self.tasks.into_iter().enumerate() {
            task.estimate = Some(order as i32);
            task.save()?;
        }
        Self::load_from_taskwarrior()
    }
}

struct ActionResult {
    new_mode: Mode,
    should_load: bool,
    should_flush: bool,
}

enum Mode {
    Normal(NormalState),
}

impl Mode {
    fn render(
        &mut self,
        opt: &Opt,
        terminal: &mut Term,
        common_state: &CommonState,
    ) -> io::Result<()> {
        match self {
            Mode::Normal(state) => state.render(opt, terminal, &common_state.tasks),
        }
    }

    fn handle_key(
        self,
        opt: &Opt,
        common_state: &mut CommonState,
        key: Key,
    ) -> io::Result<ActionResult> {
        Ok(match self {
            Mode::Normal(mut state) => {
                state.handle_key(opt, key, &common_state.tasks)?;
                ActionResult {
                    new_mode: Mode::Normal(state),
                    should_load: false,
                    should_flush: false,
                }
            }
        })
    }
}

struct NormalState {
    list_state: ListState,
}

impl NormalState {
    fn render(&mut self, opt: &Opt, terminal: &mut Term, tasks: &[Task]) -> io::Result<()> {
        let selected = self.selected();
        let contents = {
            let path = PathBuf::new()
                .join(&opt.root_dir)
                .join(&tasks[selected].uuid)
                .with_extension(&opt.file_format);

            match File::open(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => "".to_string(),
                Err(e) => return Err(e),
                Ok(mut file) => {
                    let mut buffer = String::new();
                    file.read_to_string(&mut buffer)?;
                    buffer
                }
            }
        };

        terminal.draw(|frame| {
            let layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
                .split(frame.size());

            let items: Vec<ListItem> = tasks
                .iter()
                .map(|task| ListItem::new(task.description.as_str()))
                .collect();

            // show all of the tasks
            let list = List::new(items)
                .block(Block::default().title("Tasks").borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::UNDERLINED));

            frame.render_stateful_widget(list, layout[0], &mut self.list_state);

            // preview the current highlighted task's notes
            let paragraph = Paragraph::new(contents)
                .block(Block::default().title("Preview").borders(Borders::ALL));
            frame.render_widget(paragraph, layout[1])
        })
    }

    fn handle_key(&mut self, opt: &Opt, key: Key, tasks: &[Task]) -> io::Result<()> {
        match key {
            Key::Up => {
                let mut selected = self.selected();
                if selected == 0 {
                    selected = tasks.len();
                }
                self.list_state.select(Some(selected - 1));
            }
            Key::Down => {
                let selected = self.selected();
                self.list_state.select(Some((selected + 1) % tasks.len()));
            }
            Key::Char('\n') => {
                // TODO: integrate this with the existing edit command so that the behavior is
                // shared
                //
                // TODO: make it so this can peacefully coexist alongside the stdin thread
                // right now the stdin thread either panics, if we don't lock, or buffers
                // all of the input if we do lock
                //
                // so just figure that out :)
                // let path = PathBuf::new()
                //     .join(&opt.root_dir)
                //     .join(&tasks[self.selected()].uuid)
                //     .with_extension(&opt.file_format);

                // let stdin = io::stdin();
                // let handle = stdin.lock();
                // Command::new(&opt.editor).arg(path).status()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn selected(&self) -> usize {
        match self.list_state.selected() {
            None => 0,
            Some(selected) => selected,
        }
    }
}

impl Default for NormalState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self { list_state }
    }
}
