// NEEDS TO BE COMPLETELY REWRITTEN
//
// Unsure of the goal of this subcommand, but it just rewrites the names of
// every task with 'estimate:n'. Better documentation would help

use anyhow::{Context, Result};
use std::cmp::Ordering;

use crate::{opt::Opt, taskwarrior::Task};

pub(crate) fn execute(opt: &Opt) -> Result<()> {
    let mut tasks = tasks_ordered()?;
    if !opt.args.is_empty() {
        // args.len() > 0 -> we want to reorder a specific task
        assert!(opt.args.len() == 2);
        let target_id: usize = opt.args[0].parse().unwrap();
        let target_order: usize = opt.args[1].parse().unwrap();
        assert!(target_order < tasks.len());

        let mut target_index = None;
        for (i, task) in tasks.iter().enumerate() {
            if task.id == target_id {
                target_index = Some(i);
                break;
            }
        }
        let target_index = target_index.unwrap();

        let task = tasks.remove(target_index);
        tasks.insert(target_order, task);
    }

    for (i, task) in tasks.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        task.set_estimate(Some(i as i32))?;
    }
    Ok(())
}

fn tasks_ordered() -> Result<Vec<Task>> {
    let args = &["status:pending"];
    let mut tasks = Task::get(args.iter()).context("error getting taskwarrior output")?;
    tasks.sort_by(estimate_order);
    Ok(tasks)
}

fn estimate_order(task1: &Task, task2: &Task) -> Ordering {
    let order = task1.estimate.partial_cmp(&task2.estimate);
    if let Some(order) = order {
        order
    } else {
        Ordering::Greater
    }
}
