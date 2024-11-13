use copypasta::{ClipboardContext, ClipboardProvider};
use std::io::Read;

use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, List, ListState, Paragraph, Wrap},
};

use crate::logger::Logger;
use crate::{error, info, network};

#[derive(Eq, PartialEq)]
enum ChatInputMode {
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

    // no idea what these +1s are for lol
    pub fn next_whitespace_distance(&self, line: usize, column: usize) -> usize {
        let offset = self.get_flat(line, column + 1, false);
        if let Some(rest) = self.content.get(offset..) {
            match rest.find(char::is_whitespace) {
                Some(d) => d,
                None => rest.len(),
            }
        } else {
            0
        }
    }

    pub fn prev_whitespace_distance(&self, line: usize, column: usize) -> usize {
        let offset = self.get_flat(line, column + 1, false);
        if let Some(rest) = self.content.get(..offset) {
            match rest.rfind(char::is_whitespace) {
                Some(d) => offset - d,
                None => rest.len(),
            }
        } else {
            0
        }
    }

    pub fn insert(&mut self, substring: &str, line: usize, column: usize) {
        let offset = self.get_flat(line, column + if line > 0 { 1 } else { 0 }, false);

        let sanitized = substring.replace("\t", "    ");

        if offset >= self.content.len() {
            self.content.push_str(&sanitized);
        } else {
            self.content.insert_str(offset, &sanitized);
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

pub struct ChatState {
    input_wrapped: WrappedText,
    chat_wrapped: WrappedText,
    input_mode: ChatInputMode,
    pending_changes: bool,
    pub chat_messages: Vec<network::Message>,
    input_cursor: (usize, usize),
    chat_cursor: (usize, usize),
    pending_page_up: bool,
    pending_chat_update: String,
    pending_deletions: usize,
    last_message_instant: std::time::Instant,
    next_window: WindowView,
}

// there's probably a better abstraction for these interactive boxes
impl ChatState {
    pub fn rewrap(&mut self, window: Rect) {
        let new_wrapped = match self.input_mode {
            ChatInputMode::Normal => &mut self.chat_wrapped,
            ChatInputMode::Insert => &mut self.input_wrapped,
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

        if column > 0 {
            lengths.push(column);
        }

        new_wrapped.line_lengths = lengths;
        new_wrapped.content = wrapped_content;
    }

    // the output clamped cursor of this should refer to
    // the bounds of the container in which it resides
    pub fn clamp_cursor(&mut self) {
        let (cursor, wrapped) = match self.input_mode {
            ChatInputMode::Normal => (&mut self.chat_cursor, &self.chat_wrapped),
            ChatInputMode::Insert => (&mut self.input_cursor, &self.input_wrapped),
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
                - (if cursor.0 > 0 && self.input_mode == ChatInputMode::Insert {
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

pub fn chat(
    terminal: &mut ratatui::DefaultTerminal,
    system_prompt: &str,
    api: &str,
    conversation_path: &str,
) -> Result<WindowView, Box<dyn std::error::Error>> {
    let conversation = match std::path::Path::new(conversation_path).exists() {
        true => {
            let contents = std::fs::read_to_string(conversation_path)?;
            serde_json::from_str(&contents)?
        }
        false => Vec::new(),
    };

    let mut state = ChatState {
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
        input_mode: ChatInputMode::Normal,
        input_cursor: (0, 0),
        chat_cursor: (0, 0),
        pending_page_up: false,
        pending_chat_update: String::new(),
        pending_deletions: 0,
        last_message_instant: std::time::Instant::now() - std::time::Duration::from_secs(60),
        next_window: WindowView::Chat,
    };

    for chat in state.chat_messages.iter() {
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
                    ChatInputMode::Normal => (&mut state.chat_cursor, &mut state.chat_wrapped),
                    ChatInputMode::Insert => (&mut state.input_cursor, &mut state.input_wrapped),
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
                } else if page_check.line_lengths.len() < page_check.window_size.1 - 2 {
                    page_check.page = 0;
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

                    state.input_wrapped.content.drain(lower..upper);

                    if state.input_cursor.0 > 0 && state.input_cursor.1 == 0 {
                        state.input_cursor.0 -= 1;
                        state.input_cursor.1 = usize::MAX;
                    } else {
                        state.input_cursor.1 -= 1;
                    }

                    state.pending_deletions = 0;
                }

                state.rewrap(input_box);
            }

            state.clamp_cursor();

            // the cursor should always be clamped before reaching here
            let (display_cursor, focused_area) = match state.input_mode {
                ChatInputMode::Normal => (state.chat_cursor, chat_box),
                ChatInputMode::Insert => (state.input_cursor, input_box),
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
                    ChatInputMode::Insert => "Insert",
                    ChatInputMode::Normal => "Command",
                })
                .style(Style::default().fg(Color::Black).bg(
                    match state.input_mode {
                        ChatInputMode::Insert => Color::LightYellow,
                        ChatInputMode::Normal => Color::LightCyan,
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
                state.last_message_instant = std::time::Instant::now();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(e) => panic!("{}", e),
        };

        if {
            if (std::time::Instant::now() - state.last_message_instant).as_millis() < 5000 {
                event::poll(std::time::Duration::from_millis(5))?
            } else {
                true
            }
        } {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        if state.input_mode == ChatInputMode::Normal {
                            match key.code {
                                KeyCode::Tab => {
                                    state.next_window = state.next_window.next();
                                    break;
                                }
                                KeyCode::Char('q') => {
                                    state.next_window = WindowView::Exit;
                                    break;
                                }
                                KeyCode::Char('i') | KeyCode::Char('a') => {
                                    state.input_mode = ChatInputMode::Insert;
                                }
                                KeyCode::Char('l') => {
                                    state.next_window = WindowView::Load;
                                    break;
                                }
                                KeyCode::Enter => {
                                    if state.input_wrapped.len() > 0 {
                                        state.chat_messages.push(network::Message {
                                            message_type: network::MessageType::User,
                                            content: state.input_wrapped.content.clone(),
                                        });

                                        state.pending_chat_update =
                                            (if state.chat_messages.len() > 1 {
                                                "\n───\n".to_string()
                                            } else {
                                                "".to_string()
                                            }) + &state.input_wrapped.content.clone()
                                                + "\n───\n";

                                        state.chat_messages.push(network::Message {
                                            message_type: network::MessageType::Assistant,
                                            content: String::new(),
                                        });

                                        state.last_message_instant = std::time::Instant::now();

                                        let messages = state.chat_messages.clone();
                                        let prompt = system_prompt.to_string();
                                        let api = api.to_string();
                                        let tx = tx.clone();
                                        std::thread::spawn(move || {
                                            match network::prompt_stream(prompt, &messages, api, tx)
                                            {
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
                        } else if state.input_mode == ChatInputMode::Insert {
                            match key.code {
                                KeyCode::Esc => {
                                    state.input_mode = ChatInputMode::Normal;
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

                                        // TODO: structs with row/col/width/height values instead
                                        //       of these stupid fucking tuples
                                        if state.input_cursor.1
                                            >= state.input_wrapped.window_size.0 - 3
                                        {
                                            state.input_wrapped.insert(
                                                &"\n".to_string(),
                                                state.input_cursor.0 + state.input_wrapped.page,
                                                state.input_cursor.1,
                                            );

                                            state.input_cursor.0 += 1;
                                            state.input_cursor.1 = 1;
                                        } else {
                                            state.input_cursor.1 += 1;
                                        }
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
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        state.input_cursor.1 -= std::cmp::min(
                                            state.input_cursor.1,
                                            state.input_wrapped.prev_whitespace_distance(
                                                state.input_cursor.0,
                                                state.input_cursor.1,
                                            ),
                                        );

                                        if state.input_cursor.0 > 0 && state.input_cursor.1 == 0 {
                                            state.input_cursor.0 -= 1;

                                            // this gets caught by the cursor clamping
                                            state.input_cursor.1 = usize::MAX;
                                        }
                                    }

                                    // underflow
                                    if state.input_cursor.1 > 0 {
                                        state.input_cursor.1 -= 1;
                                    }
                                }
                                KeyCode::Right => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        state.input_cursor.1 +=
                                            state.input_wrapped.next_whitespace_distance(
                                                state.input_cursor.0,
                                                state.input_cursor.1,
                                            );

                                        if state.input_cursor.0 + state.input_wrapped.page
                                            < state.input_wrapped.line_lengths.len()
                                            && state.input_cursor.1
                                                >= state.input_wrapped.line_lengths[state
                                                    .input_cursor
                                                    .0
                                                    + state.input_wrapped.page]
                                        {
                                            state.input_cursor.0 += 1;
                                            state.input_cursor.1 = 1;
                                        }
                                    }

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

    let messages_json = serde_json::to_string(&state.chat_messages).unwrap();
    match std::fs::write(conversation_path, messages_json) {
        Ok(_) => {
            info!("Conversation saved to {}", conversation_path);
        }
        Err(e) => {
            info!("Error saving messages: {}", e);
        }
    }

    Ok(state.next_window)
}

#[derive(PartialEq)]
enum DirectoryInputMode {
    Search,
    Files,
}

struct DirectoryState {
    input_mode: DirectoryInputMode,
    search_max_width: usize,
    search_content: String,
    // the search bar will only ever be one line
    search_cursor: usize,
    search_results: Vec<network::DeweyResponseItem>,
    results_state: ListState,
    next_window: WindowView,
}

pub fn directory(
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<WindowView, Box<dyn std::error::Error>> {
    let mut state = DirectoryState {
        input_mode: DirectoryInputMode::Search,
        search_max_width: 0,
        search_content: String::new(),
        search_cursor: 0,
        search_results: Vec::new(),
        results_state: ListState::default(),
        next_window: WindowView::Directory,
    };

    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();

    loop {
        terminal.draw(|frame| {
            let main_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Max(3), Constraint::Min(3)])
                .split(frame.area());

            let results_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(main_layout[1]);

            state.search_max_width = main_layout[0].width as usize;

            let list = List::new(
                state
                    .search_results
                    .iter()
                    .map(|response| response.filepath.clone()),
            )
            .block(Block::bordered().title("File Results"))
            .highlight_style(Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD))
            .highlight_symbol(">")
            .repeat_highlight_symbol(true);

            frame.render_widget(
                Paragraph::new(state.search_content.clone())
                    .block(Block::bordered().title("Search")),
                main_layout[0],
            );

            frame.render_stateful_widget(list, results_layout[0], &mut state.results_state);

            frame.render_widget(
                Paragraph::new(match state.results_state.selected() {
                    Some(i) => {
                        let selected = state.search_results[i].clone();
                        let contents = match std::fs::read_to_string(selected.filepath.clone()) {
                            Ok(c) => c,
                            Err(e) => {
                                error!("error reading file {}: {}", selected.filepath.clone(), e);

                                format!("error reading file {}: {}", selected.filepath, e)
                            }
                        };

                        contents[selected.subset.0 as usize..selected.subset.1 as usize].to_string()
                    }
                    None => String::new(),
                })
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title("Contents")),
                results_layout[1],
            );

            let (display_cursor, focused_area) = match state.input_mode {
                DirectoryInputMode::Search => ((0, state.search_cursor), main_layout[0]),
                DirectoryInputMode::Files => ((0, 0), results_layout[0]),
            };

            if state.input_mode == DirectoryInputMode::Search {
                frame.set_cursor_position(Position::new(
                    focused_area.x + display_cursor.1 as u16 + 1,
                    focused_area.y + display_cursor.0 as u16 + 1,
                ));
            }
        })?;

        match rx.try_recv() {
            Ok(buffer) => {
                let buffer = String::from_utf8_lossy(&buffer);
                let response: network::DeweyResponse = match serde_json::from_str(&buffer) {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!("Failed to parse response: {}", e);
                        error!("buffer: {:?}", buffer);
                        return Err(e.into());
                    }
                };

                state.search_results = response.results.iter().map(|f| f.clone()).collect();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(e) => panic!("{}", e),
        };

        if event::poll(std::time::Duration::from_millis(25))? {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        if state.input_mode == DirectoryInputMode::Files {
                            match key.code {
                                KeyCode::Tab => {
                                    state.next_window = state.next_window.next();
                                    break;
                                }
                                KeyCode::Char('q') => {
                                    state.next_window = WindowView::Exit;
                                    break;
                                }
                                KeyCode::Char('s') => {
                                    state.input_mode = DirectoryInputMode::Search;
                                }
                                KeyCode::Up => {
                                    state.results_state.select_previous();
                                }
                                KeyCode::Down => {
                                    state.results_state.select_next();
                                }
                                _ => {}
                            }
                        } else if state.input_mode == DirectoryInputMode::Search {
                            match key.code {
                                KeyCode::Esc => {
                                    state.input_mode = DirectoryInputMode::Files;
                                }
                                KeyCode::Char(c) => {
                                    if state.search_content.len() < state.search_max_width {
                                        state.search_content.push(c);

                                        if state.search_cursor < state.search_max_width {
                                            state.search_cursor += 1;
                                        }
                                    }
                                }
                                KeyCode::Enter => {
                                    let request = serde_json::to_string(&network::DeweyRequest {
                                        k: 10,
                                        query: state.search_content.clone(),
                                        filters: Vec::new(),
                                    })?
                                    .into_bytes();

                                    let mut payload = Vec::new();
                                    payload
                                        .extend_from_slice(&(request.len() as u32).to_be_bytes());
                                    payload.extend_from_slice(&request);

                                    let tx = tx.clone();
                                    std::thread::spawn(move || {
                                        match network::tcp_request("5051", payload, tx) {
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
                                KeyCode::Backspace => {
                                    if state.search_cursor > 0 {
                                        state
                                            .search_content
                                            .drain(state.search_cursor - 1..state.search_cursor);

                                        state.search_cursor -= 1;
                                    }
                                }
                                KeyCode::Left => {
                                    if state.search_cursor > 0 {
                                        state.search_cursor -= 1;
                                    }
                                }
                                KeyCode::Right => {
                                    if state.search_cursor < state.search_max_width
                                        && state.search_cursor < state.search_content.len()
                                    {
                                        state.search_cursor += 1;
                                    }
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

    Ok(state.next_window)
}

pub fn conversation_search(
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<(WindowView, String), Box<dyn std::error::Error>> {
    let conversation_path = crate::config::get_conversations_dir();

    let mut conversations = Vec::new();
    for file in std::fs::read_dir(conversation_path.clone())? {
        let file = file?;
        if file.path().is_file() {
            conversations.push(network::DeweyResponseItem {
                filepath: file.file_name().to_string_lossy().into_owned(),
                subset: (0, 0),
            });
        }
    }

    conversations.sort_unstable_by(|a, b| b.filepath.cmp(&a.filepath));

    let mut state = DirectoryState {
        input_mode: DirectoryInputMode::Search,
        search_max_width: 0,
        search_content: String::new(),
        search_cursor: 0,
        search_results: conversations.clone(),
        results_state: ListState::default(),
        next_window: WindowView::Directory,
    };

    let mut last_call = std::time::Instant::now();
    let mut pending_search = false;
    let mut filtered_results = Vec::new();

    let mut chosen_conversation = String::new();

    loop {
        terminal.draw(|frame| {
            let main_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Max(3), Constraint::Min(3)])
                .split(frame.area());

            let results_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(main_layout[1]);

            state.search_max_width = main_layout[0].width as usize;

            if state.search_content.len() == 0 {
                filtered_results = Vec::new();
            }

            // this _needs_ to be multithreaded
            let results = if pending_search
                && std::time::Instant::now() - last_call > std::time::Duration::from_millis(500)
            {
                pending_search = false;
                last_call = std::time::Instant::now();

                filtered_results = state
                    .search_results
                    .iter()
                    .filter_map(|response| {
                        let path = conversation_path.join(response.filepath.clone());
                        let file = match std::fs::File::open(path.clone()) {
                            Ok(f) => Some(f),
                            Err(e) => {
                                error!("error opening file {}: {}", path.to_str().unwrap(), e);
                                None
                            }
                        };

                        if file.is_none() {
                            return None;
                        }

                        let file = file.unwrap();
                        let mut reader = std::io::BufReader::new(file);
                        let mut buffer = Vec::new();

                        loop {
                            buffer.resize(state.search_content.len(), 0);
                            let bytes_read = reader.read(&mut buffer).unwrap();
                            if bytes_read == 0 {
                                break;
                            }

                            buffer.truncate(bytes_read);

                            if let Ok(text) = String::from_utf8(buffer.clone()) {
                                if text.contains(&state.search_content) {
                                    return Some(response.filepath.clone());
                                }
                            }
                        }

                        None
                    })
                    .collect::<Vec<String>>();

                filtered_results.clone()
            } else if state.search_content.len() > 0 {
                filtered_results.clone()
            } else {
                state
                    .search_results
                    .iter()
                    .map(|response| response.filepath.clone())
                    .collect::<Vec<String>>()
            };

            let list = List::new(results.clone())
                .block(Block::bordered().title("Conversations"))
                .highlight_style(Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD))
                .highlight_symbol(">")
                .repeat_highlight_symbol(true);

            frame.render_widget(
                Paragraph::new(state.search_content.clone())
                    .block(Block::bordered().title("Search")),
                main_layout[0],
            );

            frame.render_stateful_widget(list, results_layout[0], &mut state.results_state);

            let mut lines = Vec::new();

            match state.results_state.selected() {
                Some(i) => {
                    let selected = conversation_path.join(results[i].clone());
                    let file = std::fs::File::open(selected.clone());
                    if file.is_err() {
                        lines.push(Line::from(Span::styled(
                            format!(
                                "error reading file {}: {}",
                                selected.to_str().unwrap(),
                                file.err().unwrap()
                            ),
                            Style::new().red().bold(),
                        )));
                    } else {
                        let mut file = file.unwrap();

                        // limiting file reads to arbitrarily be 2kB
                        let mut buffer = vec![0; 1024 * 2];
                        let bytes_read = file.read(&mut buffer).unwrap();
                        buffer.truncate(bytes_read);

                        let mut contents = match String::from_utf8(buffer.clone()) {
                            Ok(valid_string) => valid_string,
                            Err(e) => {
                                let e = e.utf8_error();
                                let mut valid_length = e.valid_up_to();

                                if valid_length > bytes_read {
                                    valid_length = bytes_read;
                                }

                                let valid_bytes = &buffer[..valid_length];
                                String::from_utf8(valid_bytes.to_vec()).unwrap()
                            }
                        };

                        if !contents.ends_with("\"}]") {
                            contents.push_str("\"}]");
                        }

                        contents = contents.replace("\t", "    ");

                        let messages: Vec<network::Message> = match serde_json::from_str(&contents)
                        {
                            Ok(v) => v,
                            Err(e) => {
                                lines.push(Line::from(Span::styled(
                                    format!(
                                        "error parsing conversation json {}: {}",
                                        selected.to_str().unwrap(),
                                        e
                                    ),
                                    Style::new().red(),
                                )));

                                Vec::new()
                            }
                        };

                        for message in messages.iter() {
                            let mut line = Vec::new();
                            line.push(match message.message_type {
                                network::MessageType::User => {
                                    Span::styled("User: ", Style::new().blue().bold())
                                }
                                network::MessageType::Assistant => {
                                    Span::styled("Assistant: ", Style::new().green().bold())
                                }
                                _ => Span::raw(""),
                            });

                            line.push(Span::raw(message.content.clone()));

                            lines.push(Line::from(line));
                            lines.push(Line::raw("───"));
                        }
                    }
                }
                None => {}
            };

            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .block(Block::bordered().title("Contents")),
                results_layout[1],
            );

            let (display_cursor, focused_area) = match state.input_mode {
                DirectoryInputMode::Search => ((0, state.search_cursor), main_layout[0]),
                DirectoryInputMode::Files => ((0, 0), results_layout[0]),
            };

            if state.input_mode == DirectoryInputMode::Search {
                frame.set_cursor_position(Position::new(
                    focused_area.x + display_cursor.1 as u16 + 1,
                    focused_area.y + display_cursor.0 as u16 + 1,
                ));
            }
        })?;

        if event::poll(std::time::Duration::from_millis(25))? {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        if state.input_mode == DirectoryInputMode::Files {
                            match key.code {
                                KeyCode::Tab => {
                                    state.next_window = state.next_window.next();
                                    break;
                                }
                                KeyCode::Char('q') => {
                                    state.next_window = WindowView::Exit;
                                    break;
                                }
                                KeyCode::Char('s') => {
                                    state.input_mode = DirectoryInputMode::Search;
                                }
                                KeyCode::Enter => {
                                    match state.results_state.selected() {
                                        Some(i) => {
                                            let selected = conversation_path
                                                .join(state.search_results[i].filepath.clone());

                                            chosen_conversation =
                                                selected.to_string_lossy().to_string();
                                            state.next_window = WindowView::Chat;

                                            break;
                                        }
                                        None => {}
                                    };
                                }
                                KeyCode::Up => {
                                    state.results_state.select_previous();
                                }
                                KeyCode::Down => {
                                    state.results_state.select_next();
                                }
                                _ => {}
                            }
                        } else if state.input_mode == DirectoryInputMode::Search {
                            match key.code {
                                KeyCode::Esc => {
                                    state.input_mode = DirectoryInputMode::Files;
                                }
                                KeyCode::Char(c) => {
                                    if state.search_content.len() < state.search_max_width {
                                        state.search_content.push(c);

                                        if state.search_cursor < state.search_max_width {
                                            state.search_cursor += 1;
                                        }

                                        pending_search = true;
                                    }
                                }
                                KeyCode::Backspace => {
                                    if state.search_cursor > 0 {
                                        state
                                            .search_content
                                            .drain(state.search_cursor - 1..state.search_cursor);

                                        state.search_cursor -= 1;
                                        pending_search = true;
                                    }
                                }
                                KeyCode::Left => {
                                    if state.search_cursor > 0 {
                                        state.search_cursor -= 1;
                                    }
                                }
                                KeyCode::Right => {
                                    if state.search_cursor < state.search_max_width
                                        && state.search_cursor < state.search_content.len()
                                    {
                                        state.search_cursor += 1;
                                    }
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

    Ok((state.next_window, chosen_conversation))
}

pub enum WindowView {
    Chat,
    Directory,
    Load,
    Exit,
}

impl WindowView {
    fn next(&self) -> Self {
        match self {
            WindowView::Chat => WindowView::Directory,
            WindowView::Directory => WindowView::Chat,
            WindowView::Load => WindowView::Exit,
            WindowView::Exit => WindowView::Exit,
        }
    }
}

pub fn display_manager(
    window: WindowView,
    system_prompt: &str,
    api: &str,
    mut conversation_path: String,
) -> Result<(), std::io::Error> {
    let mut terminal = ratatui::init();

    let mut window = window;
    loop {
        match window {
            WindowView::Chat => {
                match chat(&mut terminal, system_prompt, api, &conversation_path) {
                    Ok(w) => {
                        window = w;
                    }
                    Err(e) => panic!("error leaving chat: {}", e),
                };
            }
            WindowView::Directory => {
                match directory(&mut terminal) {
                    Ok(w) => window = w,
                    Err(e) => panic!("error leaving directory: {}", e),
                };
            }
            WindowView::Load => {
                match conversation_search(&mut terminal) {
                    Ok(wc) => {
                        window = wc.0;
                        if wc.1.len() > 0 {
                            conversation_path = wc.1;
                        }
                    }
                    Err(e) => panic!("error leaving conversation search: {}", e),
                };
            }
            _ => break,
        };
    }

    ratatui::restore();

    Ok(())
}
