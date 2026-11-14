//! Reordering the stages of the Aggregate draft.
use super::aggregate::AggregateState;

impl AggregateState {
    /// `self` with stages `i` and `i + 1` swapped; the selection stays on the stage it was on.
    fn swapped(self, i: usize) -> Self {
        if i >= self.stages.len().saturating_sub(1) {
            return self;
        }
        let mut stages = self.stages.clone();
        stages.swap(i, i + 1);
        let mut keys = self.keys.clone();
        keys.swap(i, i + 1);
        let selected = match self.selected {
            s if s == i => i + 1,
            s if s == i + 1 => i,
            s => s,
        };
        Self {
            stages,
            keys,
            selected,
            ..self
        }
        .changed()
    }

    /// Moves stage `i` one place up; the first stage stays.
    pub fn move_up(self, i: usize) -> Self {
        match i {
            0 => self,
            _ => self.swapped(i - 1),
        }
    }

    /// Moves stage `i` one place down; the last stage stays.
    pub fn move_down(self, i: usize) -> Self {
        self.swapped(i)
    }
}
