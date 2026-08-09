use crossterm::event::Event as CtEvent;
use tui_input::backend::crossterm::EventHandler;
use tui_input::{Input, InputRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VimMode {
    Normal,
    Insert,
    Visual,
}

#[derive(Debug, Clone)]
pub(crate) struct VimInput {
    input: Input,
    mode: VimMode,
    visual_anchor: Option<usize>,
}

impl Default for VimInput {
    fn default() -> Self {
        Self {
            input: Input::default(),
            mode: VimMode::Normal,
            visual_anchor: None,
        }
    }
}

impl VimInput {
    #[cfg(test)]
    pub(crate) fn new(value: String) -> Self {
        let input = Input::new(value);
        let mut this = Self {
            input,
            ..Self::default()
        };
        this.clamp_normal_cursor();
        this
    }

    pub(crate) fn value(&self) -> &str {
        self.input.value()
    }

    pub(crate) fn cursor(&self) -> usize {
        self.input.cursor()
    }

    pub(crate) fn visual_cursor(&self) -> usize {
        self.input.visual_cursor()
    }

    pub(crate) fn visual_scroll(&self, width: usize) -> usize {
        self.input.visual_scroll(width)
    }

    pub(crate) fn mode(&self) -> VimMode {
        self.mode
    }

    pub(crate) fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.visual_anchor?;
        if self.value().is_empty() {
            return None;
        }
        Some((anchor.min(self.cursor()), anchor.max(self.cursor())))
    }

    pub(crate) fn enter_normal(&mut self) {
        if self.mode == VimMode::Insert && self.cursor() > 0 {
            self.input.handle(InputRequest::GoToPrevChar);
        }
        self.mode = VimMode::Normal;
        self.visual_anchor = None;
        self.clamp_normal_cursor();
    }

    pub(crate) fn enter_insert_before(&mut self) {
        self.mode = VimMode::Insert;
        self.visual_anchor = None;
    }

    pub(crate) fn enter_insert_after(&mut self) {
        if !self.value().is_empty() {
            self.input.handle(InputRequest::GoToNextChar);
        }
        self.enter_insert_before();
    }

    pub(crate) fn enter_insert_start(&mut self) {
        self.input.handle(InputRequest::GoToStart);
        self.enter_insert_before();
    }

    pub(crate) fn enter_insert_end(&mut self) {
        self.input.handle(InputRequest::GoToEnd);
        self.enter_insert_before();
    }

    pub(crate) fn enter_visual(&mut self) {
        if !self.value().is_empty() {
            self.visual_anchor = Some(self.cursor());
            self.mode = VimMode::Visual;
        }
    }

    pub(crate) fn handle_insert_event(&mut self, event: &CtEvent) -> bool {
        let before = self.value().to_string();
        self.input.handle_event(event);
        self.value() != before
    }

    pub(crate) fn move_left(&mut self) {
        self.input.handle(InputRequest::GoToPrevChar);
    }

    pub(crate) fn move_right(&mut self) {
        self.input.handle(InputRequest::GoToNextChar);
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_prev_word(&mut self) {
        self.input.handle(InputRequest::GoToPrevWord);
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_next_word(&mut self) {
        self.input.handle(InputRequest::GoToNextWord);
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_start(&mut self) {
        self.input.handle(InputRequest::GoToStart);
    }

    pub(crate) fn move_end(&mut self) {
        self.input.handle(InputRequest::GoToEnd);
        self.clamp_normal_cursor();
    }

    pub(crate) fn delete_char(&mut self) -> bool {
        if self.value().is_empty() {
            return false;
        }
        let before = self.value().to_string();
        self.input.handle(InputRequest::DeleteNextChar);
        self.clamp_normal_cursor();
        self.value() != before
    }

    pub(crate) fn delete_to_end(&mut self) -> bool {
        if self.value().is_empty() {
            return false;
        }
        let before = self.value().to_string();
        self.input.handle(InputRequest::DeleteTillEnd);
        self.clamp_normal_cursor();
        self.value() != before
    }

    pub(crate) fn delete_selection(&mut self, enter_insert: bool) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        let value: String = self
            .value()
            .chars()
            .enumerate()
            .filter_map(|(index, ch)| (!(start..=end).contains(&index)).then_some(ch))
            .collect();
        self.input = Input::new(value).with_cursor(start);
        self.visual_anchor = None;
        if enter_insert {
            self.mode = VimMode::Insert;
        } else {
            self.mode = VimMode::Normal;
            self.clamp_normal_cursor();
        }
        true
    }

    fn clamp_normal_cursor(&mut self) {
        if self.mode == VimMode::Insert {
            return;
        }
        let len = self.value().chars().count();
        if len > 0 && self.cursor() >= len {
            self.input.handle(InputRequest::SetCursor(len - 1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_mode_keeps_cursor_on_a_character() {
        let mut input = VimInput::new("abc".into());
        assert_eq!(input.cursor(), 2);

        input.move_right();
        assert_eq!(input.cursor(), 2);
        input.move_start();
        input.move_left();
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn visual_delete_removes_the_inclusive_selection() {
        let mut input = VimInput::new("abcdef".into());
        input.move_start();
        input.move_right();
        input.enter_visual();
        input.move_right();
        input.move_right();

        assert_eq!(input.selection_range(), Some((1, 3)));
        assert!(input.delete_selection(false));
        assert_eq!(input.value(), "aef");
        assert_eq!(input.cursor(), 1);
        assert_eq!(input.mode(), VimMode::Normal);
    }

    #[test]
    fn append_and_escape_follow_vim_cursor_semantics() {
        let mut input = VimInput::new("ab".into());
        input.enter_insert_after();
        input.handle_insert_event(&CtEvent::Key(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('c'),
        )));
        input.enter_normal();

        assert_eq!(input.value(), "abc");
        assert_eq!(input.cursor(), 2);
    }
}
