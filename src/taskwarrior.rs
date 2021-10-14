#![allow(unused)]
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::Command,
    str,
};

use anyhow::Result;
use chrono::{offset::Local, DateTime, NaiveDateTime, TimeZone};
use colored::Colorize;
use serde::{de, Deserialize};
use shellexpand::tilde;
use thiserror::Error;

/// Errors used within this file
#[derive(Debug, Error)]
pub(crate) enum Error {
    /// Error when converting to UTF8
    #[error("there was invalid UTF-8 characters in the string output")]
    UTF8Conversion,
    /// General IO error
    #[error("IO error: {0}")]
    IO(#[source] io::Error),
    /// Serde error
    #[error("invalid data for converting task output to serde_json: {0}")]
    InvalidData(#[source] serde_json::Error),
}

use crate::opt::Opt;

// use task_hookrs::{import::import, task::Task as TaskData};

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Task {
    pub(crate) id:                  usize,
    pub(crate) description:         String,
    pub(crate) uuid:                String,
    pub(crate) status:              String,
    pub(crate) estimate:            Option<String>,
    pub(crate) tags:                Option<Vec<String>>,
    pub(crate) wait:                Option<ParsableDateTime>,
    #[cfg(target = "macos")]
    pub(crate) taskn_reminder_uuid: Option<String>,
}

impl Task {
    /// Saves anything stored inside this Task to taskwarrior.
    pub(crate) fn save(&self) -> io::Result<()> {
        let mut command = Command::new("task");

        command
            .arg("rc.bulk=0")
            .arg("rc.confirmation=off")
            .arg("rc.dependency.confirmation=off")
            .arg("rc.recurrence.confirmation=off")
            .arg(&self.uuid)
            .arg("modify")
            .arg(&self.description)
            .arg(format!("status:{}", self.status));

        // TODO: WTF is this for?
        // It just rewrites the name of every task

        // if let Some(estimate) = self.estimate {
        //     command.arg(format!("estimate:{}", estimate));
        // } else {
        //     command.arg("estimate:");
        // }
        //
        // if let Some(_wait) = &self.wait {
        //     // TODO: update wait when it exists
        //     // command.arg(format!("wait:{}", wait));
        // } else {
        //     command.arg("wait:");
        // }
        //
        // if let Some(taskn_reminder_uuid) = &self.taskn_reminder_uuid {
        //     command.arg(format!("taskn_reminder_uuid:{}", taskn_reminder_uuid));
        // } else {
        //     command.arg("taskn_reminder_uuid:");
        // }

        let _drop = command.output()?;
        Ok(())
    }

    /// Loads the contents of the note associated with a particular Task. Note
    /// that this requires the [Opt] parameter because it determines where
    /// the tasks are saved.
    pub(crate) fn load_contents(&self, opt: &Opt) -> io::Result<String> {
        let path = PathBuf::new()
            .join(&opt.root_dir)
            .join(&self.uuid)
            .with_extension(&opt.file_format);
        match File::open(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok("".to_string()),
            Err(e) => Err(e),
            Ok(mut file) => {
                let mut buffer = String::new();
                file.read_to_string(&mut buffer)?;
                Ok(buffer)
            },
        }
    }

    #[allow(unused_lifetimes)]
    pub(crate) fn get<S, I>(taskwarrior_args: I) -> Result<Vec<Self>, Error>
    where
        S: ToString,
        I: Iterator<Item = S>,
    {
        let output = Command::new("task")
            .arg("rc.json.array=on")
            .arg("rc.confirmation=off")
            .args(
                taskwarrior_args
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>(),
            )
            .arg("export")
            .output()
            .map_err(Error::IO)?;

        let output = match String::from_utf8(output.stdout) {
            Err(_) => return Err(Error::UTF8Conversion),
            Ok(output) => output,
        };

        match serde_json::from_str::<Vec<Self>>(&output) {
            Err(e) => {
                super::taskn_error!("{}", e);
                Err(Error::InvalidData(e))
            },
            Ok(tasks) => Ok(tasks),
        }
    }

    pub(crate) fn set_estimate(&mut self, estimate: Option<i32>) -> io::Result<()> {
        let estimate_arg;
        if let Some(estimate) = estimate {
            estimate_arg = format!("estimate:{}", estimate);
        } else {
            estimate_arg = "estimate:".to_string();
        }

        Command::new("task")
            .arg(&self.uuid)
            .arg("modify")
            .arg(estimate_arg)
            .output()?;

        Ok(())
    }

    /// Defines a user defined attribute (UDA) that stores the UUID of an
    /// operating system reminder onto the taskwarrior task.
    pub(crate) fn define_reminder_uda() -> io::Result<()> {
        let conf_line = "uda.taskn_reminder_uuid.type=string";
        let taskrc_path = tilde("~/.taskrc");

        let mut has_reminder_uda = false;
        {
            let taskrc = BufReader::new(File::open(taskrc_path.as_ref())?);
            for line in taskrc.lines() {
                let line = line?;
                if line == conf_line {
                    has_reminder_uda = true;
                    break;
                }
            }
        }

        if !has_reminder_uda {
            let mut taskrc = OpenOptions::new().append(true).open(taskrc_path.as_ref())?;
            writeln!(taskrc, "{}", conf_line)?;
        }

        Ok(())
    }

    /// Determines whether or not the [Task] contains a tag with the provided
    /// value.
    pub(crate) fn has_tag<S: AsRef<str>>(&self, s: S) -> bool {
        match &self.tags {
            None => false,
            Some(tags) => {
                let s = s.as_ref();
                for tag in tags {
                    if tag == s {
                        return true;
                    }
                }
                false
            },
        }
    }

    pub(crate) fn set_reminder_uuid(&mut self, uuid: &str) -> io::Result<()> {
        Command::new("task")
            .arg(&self.uuid)
            .arg("modify")
            .arg(format!("taskn_reminder_uuid:{}", uuid))
            .output()?;

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub(crate) struct ParsableDateTime(pub(crate) DateTime<Local>);

impl<'de> Deserialize<'de> for ParsableDateTime {
    fn deserialize<D: de::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ParsableDateTime, D::Error> {
        Ok(ParsableDateTime(
            deserializer.deserialize_str(DateTimeVisitor)?,
        ))
    }
}

struct DateTimeVisitor;

#[allow(single_use_lifetimes)]
impl<'de> de::Visitor<'de> for DateTimeVisitor {
    type Value = DateTime<Local>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string encoded in %Y%m%dT%H%M%SZ")
    }

    fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
        // this is a little cursed, but for good reason
        // chrono isn't happy parsing a DateTime without an associated timezone
        // so we parse a DateTime first
        // and then we know it's always in UTC so we make a DateTime<Local> from it
        // and finally convert that back into the DateTime, which is what we want
        NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%SZ")
            .map(|naive_date_time| Local.from_utc_datetime(&naive_date_time))
            .map_err(|_| de::Error::invalid_value(de::Unexpected::Str(s), &self))
    }
}
