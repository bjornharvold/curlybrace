//! A one-slot memo of an answer that depends on some inputs and is asked on every repaint: the
//! answer is computed again only when the inputs differ from the ones it was computed from, so no
//! change of the inputs can leave a stale answer behind. Interior mutability, so a view that only
//! holds `&` can use it. Two memos are always equal: a memo says nothing the inputs do not.
use std::cell::{Cell, RefCell};

#[derive(Debug)]
pub struct InputMemo<K, V> {
    cached: RefCell<Option<(K, V)>>,
    evaluations: Cell<u32>,
}

impl<K, V> Default for InputMemo<K, V> {
    fn default() -> Self {
        Self {
            cached: RefCell::new(None),
            evaluations: Cell::new(0),
        }
    }
}

impl<K: Clone, V: Clone> Clone for InputMemo<K, V> {
    fn clone(&self) -> Self {
        Self {
            cached: RefCell::new(self.cached.borrow().clone()),
            evaluations: Cell::new(self.evaluations.get()),
        }
    }
}

impl<K, V> PartialEq for InputMemo<K, V> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<K, V> Eq for InputMemo<K, V> {}

impl<K, V: Clone> InputMemo<K, V> {
    /// The answer for the inputs `is_current` accepts the remembered key of; else `compute` is
    /// run on a key made by `key_of`, and its answer is remembered. A hit copies no input.
    pub fn answer(
        &self,
        is_current: impl Fn(&K) -> bool,
        key_of: impl FnOnce() -> K,
        compute: impl FnOnce(&K) -> V,
    ) -> V {
        if let Some((key, value)) = self.cached.borrow().as_ref() {
            if is_current(key) {
                return value.clone();
            }
        }
        let key = key_of();
        let value = compute(&key);
        self.evaluations.set(self.evaluations.get().wrapping_add(1));
        *self.cached.borrow_mut() = Some((key, value.clone()));
        value
    }

    /// How many times the answer was really computed (the seam that shows a repaint does not).
    pub fn evaluations(&self) -> u32 {
        self.evaluations.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_input_is_not_computed_again_and_a_changed_one_is() {
        let memo: InputMemo<String, usize> = InputMemo::default();
        let ask = |text: &str| memo.answer(|k| k == text, || text.to_string(), |k| k.len());
        assert_eq!(ask("abc"), 3);
        assert_eq!(ask("abc"), 3);
        assert_eq!(memo.evaluations(), 1);
        assert_eq!(ask("abcd"), 4);
        assert_eq!(memo.evaluations(), 2);
        assert_eq!(ask("abc"), 3);
        assert_eq!(memo.evaluations(), 3);
    }

    #[test]
    fn memos_are_always_equal_and_a_clone_keeps_the_answer() {
        let memo: InputMemo<u8, u8> = InputMemo::default();
        memo.answer(|k| *k == 1, || 1, |k| *k + 1);
        let copy = memo.clone();
        assert_eq!(memo, InputMemo::default());
        assert_eq!(copy.answer(|k| *k == 1, || 1, |k| *k + 1), 2);
        assert_eq!(copy.evaluations(), 1);
    }
}
