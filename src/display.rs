use copypasta::{ClipboardContext, ClipboardProvider};

use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Style},
    widgets::{Block, Paragraph},
};

use crate::logger::Logger;
use crate::{api, error, info};

#[derive(Eq, PartialEq)]
enum InputMode {
    Normal,
    Insert,
}

#[derive(Debug)]
struct WrappedText {
    content: String,
    line_lengths: Vec<usize>,
    page: usize,
    window_size: (usize, usize),
}

impl WrappedText {
    fn is_byte_boundary(&self, offset: usize) -> bool {
        offset == 0
            || offset == self.content.len()
            || (self.content.as_bytes()[offset] & 0xC0) != 0x80
    }

    fn find_prev_byte_boundary(&self, mut offset: usize) -> usize {
        while offset > 0 && !self.is_byte_boundary(offset) {
            offset -= 1;
        }

        offset
    }

    fn find_next_byte_boundary(&self, mut offset: usize) -> usize {
        while offset < self.content.len() && !self.is_byte_boundary(offset) {
            offset += 1;
        }

        offset
    }

    fn get_flat(&self, line: usize, mut offset: usize, previous_byte_boundary: bool) -> usize {
        for l in 0..line {
            offset += self.line_lengths[l];
        }

        if previous_byte_boundary {
            self.find_prev_byte_boundary(offset)
        } else {
            self.find_next_byte_boundary(offset)
        }
    }

    pub fn insert(&mut self, substring: &str, line: usize, column: usize) {
        let offset = self.get_flat(line, column + 1, false);

        if offset >= self.content.len() {
            self.content.push_str(substring);
        } else {
            self.content.insert_str(offset, substring);
        }
    }

    // clamping for these two paging functions is done in the main render loop
    // with setting cursor.row >= window_height as the requirement for triggering it
    // done in the KeyEvent handling down below

    pub fn page_up(&mut self) {
        if self.page >= self.window_size.1 {
            self.page -= self.window_size.1;
        } else {
            self.page = 0;
        }
    }

    pub fn page_down(&mut self) {
        self.page += self.window_size.1;
    }

    pub fn delete_word(&mut self, line: usize, column: usize) -> String {
        let mut end = self.get_flat(line, column, true);
        if end == 0 {
            return String::new();
        }

        if column < self.line_lengths[line] {
            end += 1;
        }

        let mut begin = end - 1;
        let bytes = self.content.as_bytes();
        while begin > 0 && (!self.is_byte_boundary(begin) || !bytes[begin].is_ascii_whitespace()) {
            begin -= 1;
        }

        self.content.drain(begin..end).as_str().to_string()
    }

    pub fn display(&self) -> &str {
        if self.line_lengths.len() == 0 {
            return "";
        }

        let mut begin = 0;
        for p in 0..self.page {
            begin += self.line_lengths[p];
        }

        let mut end = begin;
        for p in self.page..self.line_lengths.len() {
            end += self.line_lengths[p];
        }

        begin = self.find_prev_byte_boundary(begin);
        end = self.find_next_byte_boundary(end);

        &self.content[begin..end].trim()
    }

    pub fn len(&self) -> usize {
        self.content.len()
    }

    pub fn clear(&mut self) {
        self.content = String::new();
        self.line_lengths = Vec::new();
    }
}

struct State {
    input_wrapped: WrappedText,
    chat_wrapped: WrappedText,
    input_mode: InputMode,
    pending_changes: bool,
    chat_messages: Vec<api::Message>,
    input_cursor: (usize, usize),
    chat_cursor: (usize, usize),
    pending_page_up: bool,
    pending_chat_update: String,
    pending_deletions: usize,
}

// there's probably a better abstraction for these interactive boxes
impl State {
    pub fn rewrap(&mut self, window: Rect) {
        let new_wrapped = match self.input_mode {
            InputMode::Normal => &mut self.chat_wrapped,
            InputMode::Insert => &mut self.input_wrapped,
        };

        let mut wrapped_content = String::new();
        let mut lengths = Vec::new();
        let mut column: usize = 0;
        for c in new_wrapped.content.chars() {
            if c == '\n' || column as u16 >= window.width - 2 {
                wrapped_content.push('\n');
                lengths.push(column as usize);
                column = 0;
            }

            if c != '\n' {
                wrapped_content.push(c);
            }

            column += c.len_utf8();
        }

        lengths.push(column);
        new_wrapped.line_lengths = lengths;
        new_wrapped.content = wrapped_content;
    }

    // the output clamped cursor of this should refer to
    // the bounds of the container in which it resides
    pub fn clamp_cursor(&mut self) {
        let (cursor, wrapped) = match self.input_mode {
            InputMode::Normal => (&mut self.chat_cursor, &self.chat_wrapped),
            InputMode::Insert => (&mut self.input_cursor, &self.input_wrapped),
        };

        if wrapped.line_lengths.len() == 0 {
            *cursor = (0, 0);
            return;
        }

        let new_row = std::cmp::min(cursor.0, wrapped.line_lengths.len() - 1);
        let new_row = std::cmp::min(new_row, wrapped.window_size.1 - 2);
        let line_index = new_row + wrapped.page;
        let col_bound = if wrapped.line_lengths[line_index] > 0 {
            wrapped.line_lengths[line_index]
                - (if cursor.0 > 0 && self.input_mode == InputMode::Insert {
                    1
                } else {
                    0
                })
        } else {
            0
        };

        cursor.0 = new_row;
        cursor.1 = std::cmp::min(std::cmp::max(cursor.1, 0), col_bound);
    }
}

pub fn display(
    system_prompt: &str,
    api: &str,
    conversation: &Vec<api::Message>,
) -> Result<Vec<api::Message>, Box<dyn std::error::Error>> {
    let mut terminal = ratatui::init();

    let mut state = State {
        input_wrapped: WrappedText {
            content: String::new(),
            line_lengths: Vec::new(),
            page: 0,
            window_size: (0, 0),
        },
        chat_wrapped: WrappedText {
            content: String::new(),
            line_lengths: Vec::new(),
            page: 0,
            window_size: (0, 0),
        },
        chat_messages: conversation.clone(),
        pending_changes: false,
        input_mode: InputMode::Normal,
        input_cursor: (0, 0),
        chat_cursor: (0, 0),
        pending_page_up: false,
        pending_chat_update: String::new(),
        pending_deletions: 0,
    };

    for (i, chat) in state.chat_messages.iter().enumerate() {
        if i > 0 {
            state.pending_chat_update.push_str("\n───\n");
        }

        state.pending_chat_update.push_str(&chat.content);
        state.pending_chat_update.push_str("\n───\n");
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();

    loop {
        terminal.draw(|frame| {
            let [chat_box, input_box, status_bar] = Layout::vertical([
                Constraint::Percentage(66),
                Constraint::Min(3),
                Constraint::Max(1),
            ])
            .areas(frame.area());

            if state.pending_changes {
                state.rewrap(input_box);
                state.pending_changes = false;
            }

            state.chat_wrapped.window_size = (chat_box.width.into(), chat_box.height.into());
            state.input_wrapped.window_size = (input_box.width.into(), input_box.height.into());

            {
                let (cursor, page_check) = match state.input_mode {
                    InputMode::Normal => (&mut state.chat_cursor, &mut state.chat_wrapped),
                    InputMode::Insert => (&mut state.input_cursor, &mut state.input_wrapped),
                };

                if cursor.0 >= page_check.window_size.1 - 2 {
                    let diff = cursor.0 - (page_check.window_size.1 - 2) + 1;
                    page_check.page = std::cmp::min(
                        page_check.page + diff,
                        page_check.line_lengths.len() - (page_check.window_size.1 - 2),
                    );
                    cursor.0 = page_check.window_size.1 - 3;
                } else if cursor.0 == 0 && state.pending_page_up && page_check.page > 0 {
                    page_check.page -= 1;
                }

                state.pending_page_up = false;
            }

            if state.pending_chat_update.len() > 0 {
                state
                    .chat_wrapped
                    .content
                    .push_str(&state.pending_chat_update);
                state.rewrap(chat_box);
                state.pending_chat_update = String::new();
            }

            // if we have pending deletions, the cursors should already be valid
            if state.pending_deletions > 0 {
                info!(
                    "calling delete with cursor: {:?} on {:?}",
                    (
                        state.input_cursor.0 + state.input_wrapped.page,
                        state.input_cursor.1,
                    ),
                    state.input_wrapped
                );
                let offset = state.input_wrapped.get_flat(
                    state.input_cursor.0 + state.input_wrapped.page,
                    state.input_cursor.1 + (if state.input_cursor.0 > 0 { 1 } else { 0 }),
                    true,
                );

                if offset < state.pending_deletions {
                    state.pending_deletions = offset;
                }

                if state.pending_deletions > 0 {
                    let lower = std::cmp::max(offset - state.pending_deletions, 0);
                    let upper = offset;

                    let deleted = state.input_wrapped.content.drain(lower..upper);
                    // setting it to the max width
                    // with the assumption that it'll be placed within bounds on the next clamp
                    if deleted.as_str() == "\n" {
                        state.input_cursor.1 = input_box.width as usize;
                        if state.input_wrapped.page > 0 {
                            state.input_wrapped.page -= 1;
                        }
                    }

                    drop(deleted);

                    state.pending_deletions = 0;
                    state.rewrap(input_box);
                }
            }

            state.clamp_cursor();

            // the cursor should always be clamped before reaching here
            let (display_cursor, focused_area) = match state.input_mode {
                InputMode::Normal => (state.chat_cursor, chat_box),
                InputMode::Insert => (state.input_cursor, input_box),
            };

            frame.render_widget(
                Paragraph::new(state.chat_wrapped.display()).block(Block::bordered().title("Chat")),
                chat_box,
            );

            frame.render_widget(
                Paragraph::new(state.input_wrapped.display())
                    .block(Block::bordered().title("Input")),
                input_box,
            );

            frame.render_widget(
                Paragraph::new(match state.input_mode {
                    InputMode::Insert => "Insert",
                    InputMode::Normal => "Command",
                })
                .style(Style::default().fg(Color::Black).bg(
                    match state.input_mode {
                        InputMode::Insert => Color::LightYellow,
                        InputMode::Normal => Color::LightCyan,
                    },
                )),
                status_bar,
            );

            frame.set_cursor_position(Position::new(
                focused_area.x + display_cursor.1 as u16 + 1,
                focused_area.y + display_cursor.0 as u16 + 1,
            ));
        })?;

        match rx.try_recv() {
            Ok(message) => {
                let last_message = state.chat_messages.last_mut().unwrap();
                last_message.content.push_str(&message.clone());

                state.pending_chat_update = message;
                state.chat_cursor.0 = state.chat_wrapped.line_lengths.len();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(e) => panic!("{}", e),
        };

        if event::poll(std::time::Duration::from_millis(25))? {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        if state.input_mode == InputMode::Normal {
                            match key.code {
                                KeyCode::Tab => {
                                    info!("{:?}", state.input_wrapped);
                                }
                                KeyCode::Char('q') => {
                                    break;
                                }
                                KeyCode::Char('i') | KeyCode::Char('a') => {
                                    state.input_mode = InputMode::Insert;
                                }
                                KeyCode::Enter => {
                                    if state.input_wrapped.len() > 0 {
                                        state.chat_messages.push(api::Message {
                                            message_type: api::MessageType::User,
                                            content: state.input_wrapped.content.clone(),
                                        });

                                        state.pending_chat_update =
                                            (if state.chat_messages.len() > 1 {
                                                "\n───\n".to_string()
                                            } else {
                                                "".to_string()
                                            }) + &state.input_wrapped.content.clone()
                                                + "\n───\n";

                                        let messages = state.chat_messages.clone();
                                        let prompt = system_prompt.to_string();
                                        let api = api.to_string();
                                        let tx = tx.clone();
                                        std::thread::spawn(move || {
                                            match api::prompt_stream(prompt, &messages, api, tx) {
                                                Ok(_) => {}
                                                Err(e) => {
                                                    error!(
                                                        "error sending message to GPT endpoint: {}",
                                                        e
                                                    );
                                                    std::process::exit(1);
                                                }
                                            }
                                        });
                                    }

                                    state.input_wrapped.clear();
                                }
                                KeyCode::Left => {
                                    // underflow
                                    if state.chat_cursor.1 > 0 {
                                        state.chat_cursor.1 -= 1;
                                    }
                                }
                                KeyCode::Right => {
                                    state.chat_cursor.1 += 1;
                                }
                                KeyCode::Up => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        state.chat_wrapped.page_up();
                                    } else {
                                        // underflow
                                        if state.chat_cursor.0 > 0 {
                                            state.chat_cursor.0 -= 1;
                                        } else {
                                            state.pending_page_up = true;
                                        }
                                    }
                                }
                                KeyCode::Down => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        state.chat_wrapped.page_down();
                                        state.chat_cursor.0 = state.chat_wrapped.window_size.1;
                                    } else {
                                        state.chat_cursor.0 += 1;
                                    }
                                }
                                _ => {}
                            }
                        } else if state.input_mode == InputMode::Insert {
                            match key.code {
                                KeyCode::Esc => {
                                    state.input_mode = InputMode::Normal;
                                }
                                KeyCode::Char(c) => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        match c {
                                            'w' => {
                                                let deleted = state.input_wrapped.delete_word(
                                                    state.input_cursor.0,
                                                    state.input_cursor.1,
                                                );

                                                if deleted.contains('\n') {
                                                    state.input_cursor.1 =
                                                        state.input_wrapped.window_size.0;
                                                }
                                            }
                                            'v' => {
                                                let mut ctx = ClipboardContext::new().unwrap();
                                                let clip_contents = ctx.get_contents().unwrap();

                                                let lines = clip_contents
                                                    .chars()
                                                    .filter(|c| *c == '\n')
                                                    .count();

                                                state.input_wrapped.insert(
                                                    &clip_contents,
                                                    state.input_cursor.0 + state.input_wrapped.page,
                                                    state.input_cursor.1,
                                                );

                                                state.input_cursor.0 += lines;
                                                state.input_cursor.1 =
                                                    state.input_wrapped.window_size.0;
                                            }
                                            _ => {}
                                        }
                                    } else {
                                        state.input_wrapped.insert(
                                            &c.to_string(),
                                            state.input_cursor.0 + state.input_wrapped.page,
                                            state.input_cursor.1,
                                        );

                                        state.input_cursor.1 += 1;
                                    }

                                    state.pending_changes = true;
                                }
                                KeyCode::Enter => {
                                    state.input_wrapped.insert(
                                        &'\n'.to_string(),
                                        state.input_cursor.0 + state.input_wrapped.page,
                                        state.input_cursor.1,
                                    );

                                    state.input_cursor.0 += 1;
                                    state.input_cursor.1 = 0;
                                    state.pending_changes = true;
                                }
                                KeyCode::Backspace => {
                                    if state.input_wrapped.len() > 0 {
                                        state.pending_deletions += 1;
                                    }
                                }
                                KeyCode::Left => {
                                    // underflow
                                    if state.input_cursor.1 > 0 {
                                        state.input_cursor.1 -= 1;
                                    }
                                }
                                KeyCode::Right => {
                                    state.input_cursor.1 += 1;
                                }
                                KeyCode::Up => {
                                    // underflow
                                    if state.input_cursor.0 > 0 {
                                        state.input_cursor.0 -= 1;
                                    } else {
                                        state.pending_page_up = true;
                                    }
                                }
                                KeyCode::Down => {
                                    state.input_cursor.0 += 1;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(e) => {
                    panic!("error reading event: {}", e);
                }
                _ => {}
            }
        }
    }

    ratatui::restore();

    Ok(state.chat_messages)
}
