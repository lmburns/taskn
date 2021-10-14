#![allow(unused)]
mod events;

use anyhow::{anyhow, Context, Result};
use std::{
    io::{self, Stdout, Write},
    process::Command,
};
use thiserror::Error;

use termion::{
    event::Key,
    input::MouseTerminal,
    raw::{IntoRawMode, RawTerminal},
    screen::AlternateScreen,
};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};

use crate::{opt::Opt, taskwarrior::Task};
use events::{Event, Events};

#[derive(Debug, Error)]
pub(crate) enum Error {
    /// Next item in iterator error
    #[error("error gaining next item in iterator: {0}")]
    NextIterator(#[source] anyhow::Error),
    /// Error running task subcommand displaying stdout and stderr
    #[error("`task {command}` for task `{uuid}` failed. {stdout} {stderr}")]
    TaskCmd {
        command: String,
        uuid:    String,
        stdout:  String,
        stderr:  String,
    },
    /// Error running task subcommand, general error
    #[error("`task {command}` for task `{uuid}` failed. {err}")]
    TaskCmdNoStdout {
        command: String,
        uuid:    String,
        #[source]
        err:     io::Error,
    },
    /// Error for Task only showing UUID
    #[error("`task {command}` for task `{uuid}` failed")]
    TaskUUID { command: String, uuid: String },
}

// type Term = Terminal<TermionBackend<RawTerminal<Stdout>>>;
type Term = Terminal<TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>;

pub(crate) fn execute(opt: &Opt) -> Result<()> {
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut taskwarrior_args = opt.args.clone();
    taskwarrior_args.push("(status:pending or status:waiting)".to_string());

    terminal.hide_cursor()?;
    terminal.clear()?;

    let events = Events::new();
    let mut common_state = CommonState::load_from_taskwarrior(opt)?;
    let mut mode: Box<dyn Mode> = Box::new(Normal);
    loop {
        mode.render(&mut common_state, &mut terminal)?;
        match events.next().map_err(Error::NextIterator)? {
            Event::Key(key) => match key {
                Key::Char('q') | Key::Esc | Key::Ctrl('c') => break,
                key => {
                    let result = mode.update(opt, &mut common_state, key)?;
                    if let Some(new_mode) = result.new_mode {
                        mode = new_mode;
                    }
                    if result.should_flush {
                        common_state = common_state.flush_to_taskwarrior(opt)?;
                    } else if result.should_load {
                        common_state = CommonState::load_from_taskwarrior(opt)?;
                    }
                },
            },
            Event::Resize => continue,
        }
    }

    terminal.show_cursor()?;
    drop(terminal);
    io::stdout().flush()?;

    Ok(())
}

struct CommonState {
    list_state:     ListState,
    tasks:          Vec<Task>,
    // TODO: right now we represent the contents of a task on this [CommonState]
    // but it seems like it ought to be on the task instead, since it's specifically
    // that task's contents
    // think about moving this onto the [Task].
    tasks_contents: Vec<(String, String)>,
}

impl CommonState {
    fn load_from_taskwarrior(opt: &Opt) -> Result<Self> {
        let mut tasks = {
            if opt.only_taskn {
                Task::get(["status:pending", "+taskn"].iter())
            } else {
                Task::get(["status:pending"].iter())
            }
        }
        .context("error with task output from arguments: status:pending")?;

        tasks.sort_by(|a, b| a.estimate.partial_cmp(&b.estimate).unwrap());

        let mut list_state = ListState::default();
        if !tasks.is_empty() {
            list_state.select(Some(0));
        }

        let mut tasks_contents = Vec::with_capacity(tasks.len());
        for task in &tasks {
            tasks_contents.push((task.uuid.clone(), task.load_contents(opt)?));
        }

        Ok(CommonState {
            list_state,
            tasks,
            tasks_contents,
        })
    }

    fn flush_to_taskwarrior(self, opt: &Opt) -> Result<Self> {
        // need to calculate new_selected before into_iter()
        // because otherwise it would partially move out of self
        // and cause a compiler error
        let mut new_selected = self.selected();
        for (order, mut task) in self.tasks.into_iter().enumerate() {
            task.estimate = Some(order.to_string());
            task.save()?;
        }
        let mut new_self =
            Self::load_from_taskwarrior(opt).context("error loading new data from task")?;

        if new_selected >= new_self.tasks.len() {
            new_selected = new_self.tasks.len() - 1;
        }
        new_self.list_state.select(Some(new_selected));
        Ok(new_self)
    }

    fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    fn selected_contents(&self) -> String {
        let selected = self.selected();
        let selected_uuid = &self.tasks[selected].uuid;
        for (uuid, contents) in self.tasks_contents.clone() {
            if *selected_uuid == uuid {
                return contents;
            }
        }
        panic!("selected invariant violated");
    }
}

#[derive(Default)]
struct ActionResult {
    new_mode:     Option<Box<dyn Mode>>,
    should_load:  bool,
    should_flush: bool,
}

trait Mode {
    fn render(&self, common_state: &mut CommonState, terminal: &mut Term) -> Result<()>;

    fn update(
        &mut self,
        opt: &Opt,
        common_state: &mut CommonState,
        key: Key,
    ) -> Result<ActionResult>;
}

/// The default interactive mode. Does not modify any data. Allows users to look
/// through their tasks alongside their associated taskn notes.
#[derive(Copy, Clone)]
struct Normal;

impl Mode for Normal {
    fn render(&self, common_state: &mut CommonState, terminal: &mut Term) -> Result<()> {
        terminal
            .draw(|frame| common_render(frame, common_state, &[Modifier::DIM]))
            .context("error drawing terminal")
    }

    fn update(
        &mut self,
        _opt: &Opt,
        common_state: &mut CommonState,
        key: Key,
    ) -> Result<ActionResult> {
        let selected = common_state.selected();
        match key {
            Key::Up | Key::Char('k' | 'K') =>
                if selected > 0 {
                    common_state.list_state.select(Some(selected - 1));
                },
            Key::Down | Key::Char('j' | 'J') =>
                if selected < common_state.tasks.len() - 1 {
                    common_state.list_state.select(Some(selected + 1));
                },
            Key::Char('g') => common_state.list_state.select(Some(0)),
            Key::Char('G') => common_state
                .list_state
                .select(Some(common_state.tasks.len() - 1)),
            Key::Char('d') =>
                return Ok(ActionResult {
                    new_mode:     Some(Box::new(Done)),
                    should_flush: false,
                    should_load:  false,
                }),
            Key::Char('s') =>
                return Ok(ActionResult {
                    new_mode:     Some(Box::new(Shift::new(selected))),
                    should_flush: false,
                    should_load:  false,
                }),
            Key::Char('X') => {
                self.task_edit(common_state);
            },
            _ => {},
        }
        Ok(ActionResult {
            new_mode:     None,
            should_flush: false,
            should_load:  false,
        })
    }
}

impl Normal {
    #[allow(clippy::unused_self)]
    pub(crate) fn task_edit(self, common_state: &mut CommonState) -> Result<(), Error> {
        let selected = common_state.selected();

        let task_id = common_state.tasks[selected].id;
        let task_uuid = &common_state.tasks[selected].uuid;
        let r = Command::new("task").arg(task_uuid).arg("edit").spawn();

        let r = match r {
            Ok(child) => {
                let output = child.wait_with_output();
                match output {
                    Ok(output) =>
                        if output.status.success() {
                            String::from_utf8_lossy(&output.stdout);
                            Ok(())
                        } else {
                            Err(Error::TaskCmd {
                                command: String::from("edit"),
                                uuid:    task_uuid.to_string(),
                                stdout:  String::from_utf8_lossy(&output.stdout).to_string(),
                                stderr:  String::from_utf8_lossy(&output.stderr).to_string(),
                            })
                        },
                    Err(err) => Err(Error::TaskCmdNoStdout {
                        command: String::from("edit"),
                        uuid: task_uuid.to_string(),
                        err,
                    }),
                }
            },
            _ => Err(Error::TaskUUID {
                command: String::from("edit"),
                uuid:    task_uuid.to_string(),
            }),
        };

        r
    }
}

/// Allows users to move a selected task (as selected in [Normal] mode) to a
/// different ordering. Used to modifying the order in which tasks appear in the
/// default TaskWarrior report.
struct Shift {
    original_pos: usize,
}

impl Shift {
    fn new(current_pos: usize) -> Self {
        Self {
            original_pos: current_pos,
        }
    }
}

impl Mode for Shift {
    fn render(&self, common_state: &mut CommonState, terminal: &mut Term) -> Result<()> {
        terminal
            .draw(|frame| {
                common_render(frame, common_state, &[Modifier::DIM, Modifier::UNDERLINED]);
            })
            .context("error drawing terminal")
    }

    fn update(
        &mut self,
        _opt: &Opt,
        common_state: &mut CommonState,
        key: Key,
    ) -> Result<ActionResult> {
        match key {
            Key::Up | Key::Char('k' | 'K') => {
                let selected = common_state.selected();
                if selected > 0 {
                    common_state.tasks.swap(selected, selected - 1);
                    common_state.list_state.select(Some(selected - 1));
                }
            },
            Key::Down | Key::Char('j' | 'J') => {
                let selected = common_state.selected();
                if selected < common_state.tasks.len() - 1 {
                    common_state.tasks.swap(selected, selected + 1);
                    common_state.list_state.select(Some(selected + 1));
                }
            },
            Key::Char('\n' | 's') =>
                return Ok(ActionResult {
                    new_mode:     Some(Box::new(Normal)),
                    should_flush: true,
                    should_load:  false,
                }),
            Key::Esc | Key::Ctrl('f') => {
                let selected = common_state.selected();
                let task = common_state.tasks.remove(selected);
                common_state.tasks.insert(self.original_pos, task);
                common_state.list_state.select(Some(self.original_pos));
                return Ok(ActionResult {
                    new_mode:     Some(Box::new(Normal)),
                    should_flush: false,
                    should_load:  false,
                });
            },
            _ => {},
        }

        Ok(ActionResult {
            new_mode:     None,
            should_flush: false,
            should_load:  false,
        })
    }
}

/// Marks a task done as
struct Done;

impl Mode for Done {
    fn render(&self, common_state: &mut CommonState, terminal: &mut Term) -> Result<()> {
        terminal
            .draw(|frame| {
                let layout = default_layout(frame);
                render_tasks(
                    frame,
                    common_state,
                    &[Modifier::DIM, Modifier::CROSSED_OUT],
                    layout[0],
                );

                let text = "CONFIRM (ENTER) or CANCEL (ESC)";
                let paragraph = Paragraph::new(text).block(
                    Block::default()
                        .title("Mark Done?")
                        .style(
                            Style::default()
                                .fg(Color::LightMagenta)
                                .add_modifier(Modifier::BOLD),
                        )
                        .borders(Borders::ALL),
                );

                frame.render_widget(paragraph, layout[1]);
            })
            .context("error drawing terminal")
    }

    fn update(
        &mut self,
        _opt: &Opt,
        common_state: &mut CommonState,
        key: Key,
    ) -> Result<ActionResult> {
        match key {
            Key::Esc | Key::Ctrl('f') =>
                return Ok(ActionResult {
                    new_mode:     Some(Box::new(Normal)),
                    should_flush: false,
                    should_load:  false,
                }),
            Key::Char('\n') => {
                let selected = common_state.selected();
                common_state.tasks[selected].status = "done".to_string();
                return Ok(ActionResult {
                    new_mode:     Some(Box::new(Normal)),
                    should_flush: true,
                    should_load:  false,
                });
            },
            _ => {},
        }
        Ok(ActionResult::default())
    }
}

// type Frame<'a> = tui::Frame<'a, TermionBackend<RawTerminal<Stdout>>>;
type Frame<'a> =
    tui::Frame<'a, TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>;

#[allow(single_use_lifetimes)]
fn common_render<'a>(
    frame: &mut Frame<'a>,
    common_state: &mut CommonState,
    selected_modifiers: &[Modifier],
) {
    let layout = default_layout(frame);
    render_tasks(frame, common_state, selected_modifiers, layout[0]);
    render_contents(frame, common_state, layout[1]);
}

fn default_layout(frame: &mut Frame<'_>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(frame.size())
}

#[allow(single_use_lifetimes)]
fn render_tasks<'a>(
    frame: &mut Frame<'a>,
    common_state: &mut CommonState,
    _selected_modifiers: &[Modifier],
    area: Rect,
) {
    let items: Vec<ListItem> = common_state
        .tasks
        .iter()
        .map(|task| ListItem::new(task.description.as_str()))
        .collect();

    // let mut highlight_style = Style::default();
    // for modifier in selected_modifiers.iter() {
    //     highlight_style = highlight_style.add_modifier(*modifier);
    // }

    let list = List::new(items)
        .block(Block::default().title("Tasks").borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    // frame.render_widget(header, chunks[0]);
    frame.render_stateful_widget(list, area, &mut common_state.list_state);
}

#[allow(single_use_lifetimes)]
fn render_contents<'a>(frame: &mut Frame<'a>, common_state: &mut CommonState, area: Rect) {
    // preview the current highlighted task's notes
    let contents = common_state.selected_contents();
    let paragraph = Paragraph::new(contents).block(
        Block::default()
            .title("Preview")
            .style(Style::default().fg(Color::Red))
            .borders(Borders::ALL),
    );

    frame.render_widget(paragraph, area);
}
