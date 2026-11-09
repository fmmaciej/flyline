use crate::app::App;
use flybuffer::WordDelim;
use termina::event::{KeyCode, KeyEvent, Modifiers as KeyModifiers};

pub struct VimHistorySearch {
    saved_buffer: flybuffer::TextBuffer,
    forward: bool,
}

/// The active mode when Vim keybinding mode is enabled.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, strum::Display,
)]
pub enum VimMode {
    #[default]
    Insert,
    Normal,
    Visual,
    VisualLine,
}

/// Pending operator in Normal mode awaiting a motion or text object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimOperator {
    Delete,
    Change,
    Yank,
}

/// Pending single-character command awaiting the target character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimPendingChar {
    Replace,
    FindForward,
    FindBackward,
    TillForward,
    TillBackward,
}

/// Tracks the full runtime state of Vim modal editing.
#[derive(Debug, Clone, Default)]
pub struct VimState {
    pub mode: VimMode,
    pub repeat: VimRepeat,
    pub pending_operator: Option<VimOperator>,
    pub pending_char_cmd: Option<VimPendingChar>,
    pub pending_text_object: Option<char>, // 'i' (inner) or 'a' (around)
    pub count_accumulator: Option<usize>,
    pub operator_count: Option<usize>,
    pub last_find: Option<(VimPendingChar, char)>,
    pub yank_register: Option<(String, bool)>, // (text, is_linewise)
    pub last_history_search: Option<(String, bool)>,
}

/// Record one completed editing command, including an insert-mode continuation.
/// Motions and cancelled/no-op commands leave the previous change intact.
#[derive(Debug, Clone, Default)]
pub struct VimRepeat {
    pending: Vec<KeyEvent>,
    last: Vec<KeyEvent>,
    changed: bool,
    pub replaying: bool,
}

impl VimRepeat {
    pub fn begin(&mut self, key: KeyEvent, normal: bool, pending: bool) {
        if self.replaying {
            return;
        }
        if normal && !pending {
            self.pending.clear();
            self.changed = false;
        }
        if normal || !self.pending.is_empty() {
            self.pending.push(key);
        }
    }

    pub fn tracking(&self) -> bool {
        !self.replaying && !self.pending.is_empty()
    }

    pub fn finish(&mut self, normal: bool, pending: bool, changed: bool) {
        if self.replaying {
            return;
        }
        self.changed |= changed;
        if normal && !pending {
            if self.changed {
                self.last = std::mem::take(&mut self.pending);
            }
            self.pending.clear();
            self.changed = false;
        }
    }

    pub fn keys(&self) -> Vec<KeyEvent> {
        self.last.clone()
    }
}

impl VimState {
    pub fn has_pending(&self) -> bool {
        self.pending_operator.is_some()
            || self.pending_char_cmd.is_some()
            || self.pending_text_object.is_some()
    }

    pub fn get_effective_count(&self) -> usize {
        self.count_accumulator
            .unwrap_or(1)
            .saturating_mul(self.operator_count.unwrap_or(1))
            .min(10000)
    }

    pub fn reset_counts(&mut self) {
        self.count_accumulator = None;
        self.operator_count = None;
    }

    pub fn clear_pending(&mut self) {
        self.pending_operator = None;
        self.pending_char_cmd = None;
        self.pending_text_object = None;
        self.reset_counts();
    }

    pub fn enter_insert_mode(&mut self) {
        self.mode = VimMode::Insert;
        self.clear_pending();
    }

    pub fn enter_normal_mode(&mut self, buffer: &mut flybuffer::TextBuffer) {
        if self.mode == VimMode::Insert && buffer.cursor_byte_pos() > 0 {
            buffer.move_left();
        }
        buffer.clear_selection();
        self.mode = VimMode::Normal;
        self.clear_pending();
    }

    pub fn enter_visual_mode(&mut self, buffer: &mut flybuffer::TextBuffer, linewise: bool) {
        buffer.start_selection_if_none();
        self.mode = if linewise {
            VimMode::VisualLine
        } else {
            VimMode::Visual
        };
        self.clear_pending();
    }
}

impl App<'_> {
    pub fn start_vim_history_search(&mut self, forward: bool) {
        self.buffer.end_undo_group();
        let saved_buffer = std::mem::replace(
            &mut self.buffer,
            flybuffer::TextBuffer::new(if forward { "?" } else { "/" }),
        );
        self.vim_history_search = Some(VimHistorySearch {
            saved_buffer,
            forward,
        });
        self.vim_state.clear_pending();
    }

    /// Search query editing has its own buffer. Cancel restores the entire
    /// command and undo stack; Enter accepts a match and never submits it.
    pub fn handle_vim_history_search(&mut self, key: KeyEvent) -> bool {
        if self.vim_history_search.is_none() {
            return false;
        }
        let cancel = key.code == KeyCode::Escape
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'g' | '[')));
        if cancel || key.code == KeyCode::Enter {
            let search = self.vim_history_search.take().unwrap();
            let query = self.buffer.buffer()[1..].to_string();
            self.buffer = search.saved_buffer;
            if !cancel {
                let query = if query.is_empty() {
                    self.vim_state
                        .last_history_search
                        .as_ref()
                        .map(|(q, _)| q.clone())
                        .unwrap_or_default()
                } else {
                    query
                };
                if !query.is_empty() {
                    if let Some(entry) = self
                        .long_lived
                        .history_manager
                        .vim_search(&query, search.forward)
                    {
                        self.buffer_before_history_navigation
                            .get_or_insert_with(|| self.buffer.buffer().to_string());
                        self.buffer.replace_buffer(&entry.command);
                    }
                    self.vim_state.last_history_search = Some((query, search.forward));
                }
            }
            self.vim_state.clear_pending();
            if self.buffer.cursor_byte_pos() == self.buffer.line_end_pos()
                && self.buffer.cursor_byte_pos() > self.buffer.line_start_pos()
            {
                self.buffer.move_left();
            }
        } else {
            match key.code {
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                    self.buffer = flybuffer::TextBuffer::new(
                        if self.vim_history_search.as_ref().unwrap().forward {
                            "?"
                        } else {
                            "/"
                        },
                    );
                }
                KeyCode::Char(c)
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.buffer.insert_char(c)
                }
                KeyCode::Backspace if self.buffer.cursor_byte_pos() > 1 => {
                    self.buffer.delete_left()
                }
                KeyCode::Delete if !self.buffer.is_cursor_at_end() => self.buffer.delete_right(),
                KeyCode::Left if self.buffer.cursor_byte_pos() > 1 => self.buffer.move_left(),
                KeyCode::Right => self.buffer.move_right(),
                KeyCode::Home => {
                    self.buffer.try_move_cursor_to_byte_pos(1, false);
                }
                KeyCode::End => self.buffer.move_to_end(),
                _ => {}
            }
        }
        self.on_possible_buffer_change();
        true
    }

    pub fn vim_navigate_history(&mut self, forward: bool) {
        self.buffer_before_history_navigation
            .get_or_insert_with(|| self.buffer.buffer().to_string());
        match self
            .long_lived
            .history_manager
            .vim_navigate(forward, self.vim_state.get_effective_count())
        {
            Some(entry) => self.buffer.replace_buffer(&entry.command),
            None if forward => {
                if let Some(buffer) = self.buffer_before_history_navigation.take() {
                    self.buffer.replace_buffer(&buffer);
                }
            }
            None => {}
        }
        self.vim_state.clear_pending();
    }

    pub fn vim_mode(&self) -> VimMode {
        self.vim_state.mode
    }

    pub fn vim_pending_operator(&self) -> Option<VimOperator> {
        self.vim_state.pending_operator
    }

    pub fn vim_pending_char_cmd(&self) -> Option<VimPendingChar> {
        self.vim_state.pending_char_cmd
    }

    pub fn vim_pending_text_object(&self) -> Option<char> {
        self.vim_state.pending_text_object
    }

    pub fn vim_has_pending(&self) -> bool {
        self.vim_state.has_pending()
    }

    pub fn sync_yank_to_clipboard(&self, text: &str) {
        let _ = crate::flush_stdout!(
            "{}",
            termina::escape::osc::Osc::SetSelection(
                termina::escape::osc::Selection::CLIPBOARD,
                text
            )
        );
    }

    pub fn apply_operator_to_range(&mut self, range: Option<std::ops::Range<usize>>) {
        let Some(range) = range else {
            self.vim_state.clear_pending();
            return;
        };
        let op = self.vim_state.pending_operator;
        let is_visual = matches!(self.vim_state.mode, VimMode::Visual | VimMode::VisualLine);

        match op {
            Some(VimOperator::Delete) => {
                let deleted = self.buffer.delete_range(range);
                self.sync_yank_to_clipboard(&deleted);
                self.vim_state.yank_register = Some((deleted, false));
                self.vim_state.clear_pending();
                if is_visual {
                    self.vim_state.enter_normal_mode(&mut self.buffer);
                }
            }
            Some(VimOperator::Change) => {
                let deleted = self.buffer.delete_range(range);
                self.sync_yank_to_clipboard(&deleted);
                self.vim_state.yank_register = Some((deleted, false));
                self.vim_state.enter_insert_mode();
            }
            Some(VimOperator::Yank) => {
                let yanked = self.buffer.yank_range(range);
                self.sync_yank_to_clipboard(&yanked);
                self.vim_state.yank_register = Some((yanked, false));
                self.vim_state.clear_pending();
                if is_visual {
                    self.vim_state.enter_normal_mode(&mut self.buffer);
                }
            }
            None => {
                if is_visual {
                    self.buffer.set_selection_range(range, false);
                    self.vim_state.pending_text_object = None;
                } else {
                    self.vim_state.clear_pending();
                }
            }
        }
    }

    pub fn execute_vim_find(&mut self, kind: VimPendingChar, c: char, reverse: bool) {
        let effective_kind = if reverse {
            match kind {
                VimPendingChar::FindForward => VimPendingChar::FindBackward,
                VimPendingChar::FindBackward => VimPendingChar::FindForward,
                VimPendingChar::TillForward => VimPendingChar::TillBackward,
                VimPendingChar::TillBackward => VimPendingChar::TillForward,
                VimPendingChar::Replace => VimPendingChar::Replace,
            }
        } else {
            kind
        };

        match effective_kind {
            VimPendingChar::FindForward => {
                self.buffer.find_char_forward(c);
            }
            VimPendingChar::FindBackward => {
                self.buffer.find_char_backward(c);
            }
            VimPendingChar::TillForward => {
                self.buffer.till_char_forward(c);
            }
            VimPendingChar::TillBackward => {
                self.buffer.till_char_backward(c);
            }
            VimPendingChar::Replace => {
                self.buffer.replace_char_at_cursor(c);
            }
        }
    }

    pub fn execute_vim_motion_operator(&mut self, motion: VimMotion) {
        let count = self.vim_state.get_effective_count();
        let start = self.buffer.cursor_byte_pos();
        match motion {
            VimMotion::Left => {
                for _ in 0..count {
                    self.buffer.move_left();
                }
            }
            VimMotion::Right => {
                for _ in 0..count {
                    self.buffer.move_right();
                }
            }
            VimMotion::WordForward => {
                if self.vim_state.pending_operator == Some(VimOperator::Change) {
                    for _ in 0..count {
                        self.buffer.move_word_end(WordDelim::FineGrained);
                    }
                    let end = self.buffer.right_move_pos();
                    self.buffer.try_move_cursor_to_byte_pos(start, false);
                    self.apply_operator_to_range(Some(start..end));
                    return;
                } else {
                    for _ in 0..count {
                        self.buffer.move_next_word_start(WordDelim::FineGrained);
                    }
                }
            }
            VimMotion::BigWordForward => {
                if self.vim_state.pending_operator == Some(VimOperator::Change) {
                    for _ in 0..count {
                        self.buffer.move_word_end(WordDelim::WhiteSpace);
                    }
                    let end = self.buffer.right_move_pos();
                    self.buffer.try_move_cursor_to_byte_pos(start, false);
                    self.apply_operator_to_range(Some(start..end));
                    return;
                } else {
                    for _ in 0..count {
                        self.buffer.move_next_word_start(WordDelim::WhiteSpace);
                    }
                }
            }
            VimMotion::WordBackward => {
                for _ in 0..count {
                    self.buffer.move_prev_word_start(WordDelim::FineGrained);
                }
            }
            VimMotion::BigWordBackward => {
                for _ in 0..count {
                    self.buffer.move_prev_word_start(WordDelim::WhiteSpace);
                }
            }
            VimMotion::WordEnd => {
                for _ in 0..count {
                    self.buffer.move_word_end(WordDelim::FineGrained);
                }
                let end = self.buffer.right_move_pos();
                self.buffer.try_move_cursor_to_byte_pos(start, false);
                self.apply_operator_to_range(Some(start..end));
                return;
            }
            VimMotion::BigWordEnd => {
                for _ in 0..count {
                    self.buffer.move_word_end(WordDelim::WhiteSpace);
                }
                let end = self.buffer.right_move_pos();
                self.buffer.try_move_cursor_to_byte_pos(start, false);
                self.apply_operator_to_range(Some(start..end));
                return;
            }
            VimMotion::StartOfLine => {
                self.buffer.move_start_of_line();
            }
            VimMotion::FirstNonBlank => {
                self.buffer.move_first_non_whitespace();
            }
            VimMotion::EndOfLine => {
                self.buffer.move_end_of_line();
            }
            VimMotion::MatchingPair => {
                self.buffer.move_matching_pair();
                let end = self.buffer.right_move_pos();
                self.buffer.try_move_cursor_to_byte_pos(start, false);
                self.apply_operator_to_range(Some(start.min(end)..start.max(end)));
                return;
            }
            VimMotion::FindForward(c) => {
                self.vim_state.last_find = Some((VimPendingChar::FindForward, c));
                self.buffer.find_char_forward(c);
                let end = self.buffer.right_move_pos();
                self.buffer.try_move_cursor_to_byte_pos(start, false);
                self.apply_operator_to_range(Some(start..end));
                return;
            }
            VimMotion::FindBackward(c) => {
                self.vim_state.last_find = Some((VimPendingChar::FindBackward, c));
                self.buffer.find_char_backward(c);
            }
            VimMotion::TillForward(c) => {
                self.vim_state.last_find = Some((VimPendingChar::TillForward, c));
                self.buffer.till_char_forward(c);
                let end = self.buffer.right_move_pos();
                self.buffer.try_move_cursor_to_byte_pos(start, false);
                self.apply_operator_to_range(Some(start..end));
                return;
            }
            VimMotion::TillBackward(c) => {
                self.vim_state.last_find = Some((VimPendingChar::TillBackward, c));
                self.buffer.till_char_backward(c);
            }
        }
        let end = self.buffer.cursor_byte_pos();
        self.buffer.try_move_cursor_to_byte_pos(start, false);
        let range = if start <= end { start..end } else { end..start };
        self.apply_operator_to_range(Some(range));
    }
}

/// Standard Vim motions for operators.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMotion {
    Left,
    Right,
    WordForward,
    BigWordForward,
    WordBackward,
    BigWordBackward,
    WordEnd,
    BigWordEnd,
    StartOfLine,
    FirstNonBlank,
    EndOfLine,
    MatchingPair,
    FindForward(char),
    FindBackward(char),
    TillForward(char),
    TillBackward(char),
}

#[cfg(test)]
mod repeat_tests {
    use super::*;
    use termina::event::{KeyCode, Modifiers};
    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), Modifiers::empty())
    }

    #[test]
    fn motion_and_cancel_preserve_last_change() {
        let mut repeat = VimRepeat::default();
        repeat.begin(key('x'), true, false);
        repeat.finish(true, false, true);
        repeat.begin(key('h'), true, false);
        repeat.finish(true, false, false);
        assert_eq!(repeat.keys(), vec![key('x')]);
        repeat.begin(key('d'), true, false);
        repeat.finish(true, true, false);
        repeat.begin(key('q'), true, true);
        repeat.finish(true, false, false);
        assert_eq!(repeat.keys(), vec![key('x')]);
    }

    #[test]
    fn insert_continuation_finishes_only_on_normal_mode() {
        let mut repeat = VimRepeat::default();
        repeat.begin(key('c'), true, false);
        repeat.finish(true, true, false);
        repeat.begin(key('w'), true, true);
        repeat.finish(false, false, true);
        repeat.begin(key('X'), false, false);
        repeat.finish(false, false, true);
        repeat.begin(key('!'), false, false);
        repeat.finish(true, false, false);
        assert_eq!(repeat.keys(), vec![key('c'), key('w'), key('X'), key('!')]);
        repeat.replaying = true;
        repeat.begin(key('.'), true, false);
        repeat.finish(true, false, true);
        assert_eq!(repeat.keys().len(), 4);
    }
}
