use clap::{crate_description, crate_name, AppSettings, Clap};
use std::env;

use crate::commands::Command;

#[derive(Debug, Clap)]
#[clap(
    name = crate_name!(),
    about = crate_description!(),
    global_setting = AppSettings::ColorAuto,
    global_setting = AppSettings::ColoredHelp,
    global_setting = AppSettings::InferSubcommands,
    global_setting = AppSettings::DisableHelpSubcommand,
    global_setting = AppSettings::HidePossibleValuesInHelp,
)]
struct ProtoOpt {
    /// The editor used to open task notes. Uses `$EDITOR` or `vi`
    #[clap(long, short = 'e', next_line_help = true, env = "EDITOR")]
    editor: Option<String>,

    /// The file format used for task notes.
    #[clap(long, short = 'f', default_value = "md", next_line_help = true)]
    file_format: String,

    /// The directory in which task notes are placed. If the directory does not
    /// already exist, taskn will create it.
    #[clap(long, short = 'r', default_value = "~/.taskn", next_line_help = true)]
    root_dir: String,

    /// Only workon tasks with the `taskn` tag (only works with interactive, for now)
    #[clap(short, long = "only")]
    only_taskn: bool,

    /// Subcommand to run
    #[clap(subcommand)]
    command: Option<Command>,

    /// Any remaining arguments are passed along to taskwarrior while selecting
    /// tasks.
    args: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct Opt {
    pub(crate) editor:      String,
    pub(crate) only_taskn:  bool,
    pub(crate) file_format: String,
    pub(crate) root_dir:    String,
    pub(crate) command:     Command,
    pub(crate) args:        Vec<String>,
}

impl Opt {
    fn from_proto_opt(proto_opt: ProtoOpt) -> Self {
        // match Command::from_str(&proto_opt.command) {
        //     Ok(cmd) => {
        //         command = cmd;
        //         args = proto_opt.args;
        //     },
        //     Err(_) => {
        //         command = Command::Edit;
        //         args = [&[proto_opt.command], &proto_opt.args[..]].concat();
        //     },
        // }

        Opt {
            editor:      if let Some(editor) = proto_opt.editor {
                editor
            } else if let Ok(editor) = env::var("EDITOR") {
                editor
            } else {
                "vi".to_string()
            },
            only_taskn:  proto_opt.only_taskn,
            file_format: proto_opt.file_format,
            root_dir:    shellexpand::tilde(&proto_opt.root_dir).to_string(),
            command:     proto_opt.command.unwrap_or_default(),
            args:        proto_opt.args,
        }
    }

    pub(crate) fn from_args() -> Self {
        Self::from_proto_opt(ProtoOpt::parse())
    }
}
