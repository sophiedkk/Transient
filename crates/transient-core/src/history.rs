//! Undo and redo.

use crate::project::Project;

/// How many steps the undo history keeps.
const HISTORY_LIMIT: usize = 100;

/// Undo and redo, as whole-project snapshots.
///
/// Before every change, call [`History::record`] with the project as it was.
/// Snapshots are cheap: a project is only numbers and paths; the audio is
/// stored elsewhere.
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl History {
    /// Remembers `before` (the project before a change). Clears the redo steps:
    /// after a new change, the undone changes cannot come back.
    pub fn record(&mut self, before: Project) {
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Steps back: returns the previous project, remembering `current` for redo.
    pub fn undo(&mut self, current: Project) -> Option<Project> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        Some(previous)
    }

    /// Steps forward again after an undo.
    pub fn redo(&mut self, current: Project) -> Option<Project> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        Some(next)
    }

    /// Whether there is something to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is something to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_walk_through_the_history() {
        let mut history = History::default();
        let mut project = Project::default();
        assert!(!history.can_undo());

        history.record(project.clone());
        project.name = "one".to_owned();
        history.record(project.clone());
        project.name = "two".to_owned();

        project = history.undo(project).unwrap();
        assert_eq!(project.name, "one");
        project = history.undo(project).unwrap();
        assert_eq!(project.name, "");
        assert!(history.undo(project.clone()).is_none());

        project = history.redo(project).unwrap();
        assert_eq!(project.name, "one");
        // A new change makes the undone "two" unreachable.
        history.record(project.clone());
        assert!(!history.can_redo());
    }
}
