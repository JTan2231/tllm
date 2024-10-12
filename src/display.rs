use copypasta::{ClipboardContext, ClipboardProvider};
use std::io::Write;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

use crate::api;
use crate::logger::Logger;
use crate::{error, info};

const PROMPT: &str = "  ";
const MESSAGE_SEPARATOR: &str = "───";
const CHAT_BOX_HEIGHT: u16 = 10;
const POLL_TIMEOUT: u64 = 5; // ms

// wraps text around a given width
// line breaks on spaces
pub fn wrap(text: &String, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let chars = text.chars().collect::<Vec<char>>();

    let mut i = 0;
    while i < chars.len() {
        // for repeated newlines
        // feels hacky
        if chars[i] == '\n' {
            lines.push(current_line);
            current_line = String::new();
            i += 1;
            continue;
        }

        let mut newline = false;
        let mut j = i + 1;
        // retrieve the next word
        while j < chars.len() {
            if chars[j] == ' ' {
                break;
            }

            if chars[j] == '\n' {
                newline = true;
                break;
            }

            j += 1;
        }

        if newline {
            current_line.push_str(&chars[i..j].iter().collect::<String>());
            lines.push(current_line);
            current_line = String::new();
            i = j + 1;
            continue;
        }

        let word = &chars[i..j].iter().collect::<String>();
        if current_line.len() + j - i > width {
            lines.push(current_line);
            current_line = String::from(word.to_string());
        } else {
            current_line.push_str(word);
        }

        i += std::cmp::max(1, j - i);
    }

    lines.push(current_line);
    lines
}

// TODO: I think there's a lot of work to be done
//       on standardizing the way different
//       coordinate planes/origins are handled
//
// TODO: error handling in this file is laughable
//       maybe replace all the raw indexing with `.get()`?
//
// TODO: need a better way of dealing with
//       the different coordinate planes--it's getting confusing
//       differentiating between input + chat display
//       (pending refactor)
//       ..
//       should we just define a bunch of different common getters?
//       and include transformations in each?
//       and assume outside functions just deal with coordinates
//       without a basis?
//       much to ponder

#[derive(PartialEq, Clone, Debug, serde::Serialize, serde::Deserialize)]
enum InputMode {
    Edit,
    Command,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct State {
    input_history: Vec<String>,
    messages: Vec<api::Message>,
    chat_display: Vec<String>,
    input: Vec<String>,
    input_mode: InputMode,
    paging_index: u16,
    chat_paging_index: u16,
    // these 2 are (x, y) coordinates
    // and will _always_ be relative to the terminal window
    //
    // NOTE: these should _never_ be in the margins
    //       e.g., (col < PROMPT.len()) should always be false
    input_cursor_position: (u16, u16),
    highlight_cursor_position: (u16, u16),
}

// NOTE: all of these currently only account for the input paging index
//       really, the chat_paging index et al. should be accounted for
//       in an entirely different struct

impl State {
    // changing origin from terminal window to input box
    // this is still relative to the terminal window,
    // just with the origin shifted to account for padding
    // and whatnot
    //
    // as such it can be used for, e.g., indexing the characters on a line
    // (i think)
    //
    // this _DOES NOT_ adjust for paging
    fn get_input_position(&self) -> (u16, u16) {
        let origin = input_origin();

        (
            self.input_cursor_position.0 - origin.0,
            self.input_cursor_position.1 - origin.1,
        )
    }

    // mostly just for consistency with get_input_position
    //
    // this is still relative to the terminal window,
    // just with the origin shifted to account for padding
    // and whatnot
    //
    // as such it can be used for, e.g., indexing the characters on a line
    // (i think)
    //
    // this _DOES NOT_ adjust for paging
    fn get_chat_position(&self) -> (u16, u16) {
        (
            self.highlight_cursor_position.0 - PROMPT.len() as u16,
            self.highlight_cursor_position.1,
        )
    }

    // mapping from the cursor's position on the screen
    // to the index of the string it's on
    fn cursor_position_to_string_position(&self, position: (u16, u16)) -> (u16, u16) {
        match self.input_mode {
            InputMode::Edit => {
                let row = position.1 + self.paging_index;
                let col = std::cmp::max(position.0, PROMPT.len() as u16) - PROMPT.len() as u16;

                (col, row)
            }
            InputMode::Command => {
                let row = position.1 + self.chat_paging_index;
                let col = std::cmp::max(position.0, PROMPT.len() as u16) - PROMPT.len() as u16;

                (col, row)
            }
        }
    }

    fn get_current_line(&self) -> String {
        match self.input_mode {
            InputMode::Edit => {
                self.input[(self.paging_index + self.get_input_position().1) as usize].clone()
            }
            InputMode::Command => self.chat_display
                [(self.chat_paging_index + self.get_chat_position().1) as usize]
                .clone(),
        }
    }

    fn get_current_line_length(&self) -> usize {
        let line = self.get_current_line();
        if line == MESSAGE_SEPARATOR {
            3
        } else {
            line.len()
        }
    }

    // horrific function name
    // this is a mutable reference to the cursor position
    // depending on the current input mode
    fn get_mut_mode_position(&mut self) -> &mut (u16, u16) {
        match self.input_mode {
            InputMode::Edit => &mut self.input_cursor_position,
            InputMode::Command => &mut self.highlight_cursor_position,
        }
    }

    fn push_message(&mut self, message: api::Message) {
        self.messages.push(message.clone());
        let lines = wrap(&message.content, window_width() as usize - PROMPT.len() - 1);

        if message.message_type == api::MessageType::User {
            self.chat_display.push(MESSAGE_SEPARATOR.to_string());
        }
        self.chat_display.extend(lines);
        if message.message_type == api::MessageType::User {
            self.chat_display.push(MESSAGE_SEPARATOR.to_string());
        }

        // move to the bottom on updates
        if self.chat_display.len() > window_height() as usize - CHAT_BOX_HEIGHT as usize - 2 {
            self.chat_paging_index =
                self.chat_display.len() as u16 - (window_height() - CHAT_BOX_HEIGHT - 2);
        }
    }

    // looks hideous in terms of performance
    // there must be a better way
    fn push_delta(&mut self, delta: String) {
        let mut last_message = self.chat_display.pop().unwrap().clone();
        last_message.push_str(&delta);
        let new_lines = last_message.split('\n').collect::<Vec<&str>>();

        let mut wrapped_lines = Vec::new();
        for line in new_lines.iter() {
            let mut wrapped = vec![line.to_string().clone()];
            if line.len() > window_width() as usize - PROMPT.len() - 1 as usize {
                wrapped = wrap(
                    &line.to_string(),
                    window_width() as usize - PROMPT.len() as usize,
                );
            }

            wrapped_lines.extend(wrapped);
        }

        self.chat_display.extend(wrapped_lines);

        if self.chat_display.len() > window_height() as usize - CHAT_BOX_HEIGHT as usize - 1 {
            self.chat_paging_index =
                self.chat_display.len() as u16 - (window_height() - CHAT_BOX_HEIGHT - 1);
        }

        self.messages.last_mut().unwrap().content.push_str(&delta);
    }

    // indices based on a (0, 0) origin
    // mapped to their respective origins
    // dependent on input mode
    fn map_index_and_move(&mut self, col: u16, row: u16) {
        let mut col = col;
        let mut row = row;
        match self.input_mode {
            InputMode::Edit => {
                let origin = input_origin();
                col += origin.0;
                row += origin.1;

                self.input_cursor_position.0 = col as u16;
                self.input_cursor_position.1 = row as u16;

                self.input_cursor_position.0 = clamp(
                    self.input_cursor_position.0,
                    self.get_current_line_length() as u16,
                );

                move_cursor(self.input_cursor_position);
            }
            InputMode::Command => {
                self.highlight_cursor_position.0 = col + PROMPT.len() as u16;
                self.highlight_cursor_position.1 = row as u16;

                self.highlight_cursor_position.0 = clamp(
                    self.highlight_cursor_position.0,
                    self.get_current_line_length() as u16,
                );

                move_cursor(self.highlight_cursor_position);
            }
        }
    }
}

fn cleanup(message: String) {
    move_cursor((0, 0));
    execute!(std::io::stdout(), Clear(ClearType::All)).unwrap();
    disable_raw_mode().unwrap();

    println!("{}", message);
    cursor_to_block();
}

fn log_state(state: &mut State, c: &str, debug: bool) {
    state.input_history.push(format!("Char('{}')", c));
    if debug {
        info!("{}", serde_json::to_string_pretty(&state).unwrap());
        info!(
            "mapped positions: {:?}, {:?}",
            state.get_input_position(),
            state.get_chat_position()
        );
    }
}

pub fn terminal_app(
    system_prompt: String,
    api: String,
    conversation_path: String,
    debug: bool,
) -> Vec<api::Message> {
    enable_raw_mode().unwrap();
    execute!(std::io::stdout(), Clear(ClearType::All)).unwrap();
    print!("{}", PROMPT);
    std::io::stdout().flush().unwrap();

    let mut init = true;

    let height = crossterm::terminal::size().unwrap().1;

    let mut state = State {
        input_history: Vec::new(),
        messages: Vec::new(),
        chat_display: Vec::new(),
        input: Vec::from([String::new()]),
        input_mode: InputMode::Command,
        paging_index: 0,
        chat_paging_index: 0,
        input_cursor_position: (PROMPT.len() as u16, height - CHAT_BOX_HEIGHT),
        highlight_cursor_position: (PROMPT.len() as u16, 0),
    };

    let messages: Vec<api::Message> = match std::fs::read_to_string(conversation_path.clone()) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(messages) => messages,
            Err(e) => {
                error!("Failed to deserialize conversation: {}", e);
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    };

    for message in messages.iter() {
        state.push_message(message.clone());
    }

    let mut state_queue = state.clone();

    std::panic::set_hook(Box::new(|panic_info| {
        error!("{}", panic_info);
        cleanup("An error occurred!".to_string());
    }));

    let (tx, rx) = std::sync::mpsc::channel();

    let mut running = true;
    while running {
        match event::poll(std::time::Duration::from_millis(POLL_TIMEOUT)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key_event)) => match key_event.code {
                    KeyCode::Enter => {
                        log_state(&mut state_queue, "Enter", debug);
                        // KeyModifiers don't work on Enter??
                        /*if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        {*/
                        if state.input_mode == InputMode::Command && state.input.len() > 0 {
                            chat_submit(&mut state_queue);
                        } else {
                            running = enter(&mut state_queue);
                        }
                    }

                    // why does this take so long?
                    KeyCode::Esc => {
                        log_state(&mut state_queue, "Esc", debug);
                        state_queue.input_mode = InputMode::Command;
                        cursor_to_block();
                        move_cursor(state_queue.highlight_cursor_position);
                    }

                    KeyCode::Left => {
                        log_state(&mut state_queue, "Left", debug);
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            let (col, row) = previous_word(&mut state_queue);
                            state_queue.map_index_and_move(col, row);
                        } else {
                            left(state_queue.get_mut_mode_position(), PROMPT.len() as u16);
                        }
                    }

                    KeyCode::Right => {
                        log_state(&mut state_queue, "Right", debug);
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            let (col, row) = next_word(&mut state_queue);
                            state_queue.map_index_and_move(col, row);
                        } else {
                            right(
                                state_queue.get_mut_mode_position(),
                                state.get_current_line_length() as u16 + PROMPT.len() as u16,
                            );
                        }
                    }

                    KeyCode::Up => {
                        log_state(&mut state_queue, "Up", debug);
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::SHIFT)
                        {
                            page_up(&mut state_queue);
                        } else {
                            up(&mut state_queue);
                        }
                    }

                    KeyCode::Down => {
                        log_state(&mut state_queue, "Down", debug);
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::SHIFT)
                        {
                            page_down(&mut state_queue);
                        } else {
                            down(&mut state_queue);
                        }
                    }

                    KeyCode::Backspace => {
                        log_state(&mut state_queue, "Backspace", debug);
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                        } else {
                            backspace(&mut state_queue);
                        }
                    }

                    KeyCode::Char(c) => {
                        log_state(&mut state_queue, &c.to_string(), debug);
                        running = input(&mut state_queue, c, key_event.modifiers);
                    }
                    _ => {}
                },
                Err(e) => {
                    panic!("Error reading event: {}", e);
                }
                _ => {}
            },
            Ok(false) => {
                // do we have a pending message for gpt?
                match state.messages.last() {
                    Some(last_message) => {
                        if last_message.message_type == api::MessageType::User {
                            state_queue.push_message(api::Message::new(
                                api::MessageType::Assistant,
                                String::new(),
                            ));

                            let messages = state.messages.clone();
                            let prompt = system_prompt.clone();
                            let tx = tx.clone();
                            let api = api.clone();
                            std::thread::spawn(move || {
                                match api::prompt_stream(prompt, &messages, &api, tx) {
                                    Ok(_) => {}
                                    Err(e) => {
                                        error!("Error sending message to GPT: {}", e);

                                        std::process::exit(1);
                                    }
                                }
                            });
                        }
                    }
                    _ => {}
                };

                let mut new_delta = false;
                if let Ok(delta) = rx.try_recv() {
                    new_delta = true;
                    state_queue.push_delta(delta);
                }

                draw(
                    &state_queue,
                    state.messages.len() != state_queue.messages.len()
                        || init
                        || state.chat_paging_index != state_queue.chat_paging_index
                        || new_delta,
                    state.input != state_queue.input
                        || init
                        || state.paging_index != state_queue.paging_index,
                );

                // is this worth optimizing?
                state = state_queue.clone();
                init = false;
            }
            Err(e) => {
                panic!("Error polling events: {}", e);
            }
        }
    }

    cleanup("Goodbye!".to_string());

    state.messages
}

fn draw_lines(lines: &[String], current_row: u16) {
    move_cursor((PROMPT.len() as u16, current_row));
    let mut current_row = current_row;
    for line in lines.iter() {
        execute!(std::io::stdout(), Clear(ClearType::CurrentLine)).unwrap();
        let display_line = if line.ends_with('\n') {
            line[0..line.len() - 1 as usize].to_string()
        } else {
            line.clone()
        };
        print!("{}", display_line);
        current_row += 1;
        move_cursor((PROMPT.len() as u16, current_row));
    }
}

fn get_current_view(lines: &Vec<String>, paging_index: usize, height: usize) -> &[String] {
    if lines.len() > height {
        &lines[paging_index..std::cmp::min(paging_index + height, lines.len())]
    } else {
        &lines
    }
}

fn draw(state: &State, redraw_messages: bool, redraw_input: bool) {
    move_cursor((0, 0));

    if redraw_messages {
        execute!(std::io::stdout(), Clear(ClearType::All)).unwrap();

        // minus 1 for the separator
        let message_box_height =
            (crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT - 1) as usize;
        let mut message_lines = state.chat_display.clone();
        while message_lines.len() < message_box_height {
            message_lines.push("".to_string());
        }

        let display_lines = get_current_view(
            &message_lines,
            state.chat_paging_index as usize,
            message_box_height,
        );

        draw_lines(display_lines, 0);

        move_cursor((
            PROMPT.len() as u16,
            crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT - 1,
        ));
        print!("──────");
    }

    let current_row = crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT;
    move_cursor((PROMPT.len() as u16, current_row));

    if redraw_input {
        execute!(std::io::stdout(), Clear(ClearType::CurrentLine)).unwrap();
        let lines = get_current_view(
            &state.input,
            state.paging_index as usize,
            CHAT_BOX_HEIGHT as usize,
        );
        draw_lines(lines, current_row);
    }

    if state.input_mode == InputMode::Edit {
        cursor_to_line();
        move_cursor(state.input_cursor_position);
    } else {
        cursor_to_block();
        move_cursor(state.highlight_cursor_position);
    }
}

fn input_origin() -> (u16, u16) {
    (
        PROMPT.len() as u16,
        crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT,
    )
}

pub fn window_width() -> u16 {
    crossterm::terminal::size().unwrap().0
}

fn window_height() -> u16 {
    crossterm::terminal::size().unwrap().1
}

fn move_cursor(position: (u16, u16)) {
    execute!(
        std::io::stdout(),
        crossterm::cursor::MoveTo(position.0, position.1)
    )
    .unwrap();
}

// ideally, this is used every time the cursor is moved
fn clamp(col: u16, line_length: u16) -> u16 {
    return std::cmp::max(std::cmp::min(col, line_length), PROMPT.len() as u16);
}

fn left(position: &mut (u16, u16), bound: u16) {
    if position.0 > bound {
        position.0 -= 1;
        move_cursor(*position);
    }
}

fn right(position: &mut (u16, u16), bound: u16) {
    if position.0 < bound {
        position.0 += 1;
        move_cursor(*position);
    }
}

// delicious wet code
//
// i'm sure there are much better ways to structure the code around here
// but unfortunately i am an inexperienced, filthy individual
//
// updates the state's cursor position (input_mode dependent)
// to find the next word
fn next_word(state: &mut State) -> (u16, u16) {
    let (pos, mut paging_index, row_bound, paging_index_bound, display_lines) =
        match state.input_mode {
            InputMode::Edit => {
                let pos = state.get_input_position();
                let paging_index = state.paging_index as usize;
                let row_bound = window_height() - 1;
                let paging_index_bound = state.paging_index as usize;
                let display_lines = &state.input;

                (
                    pos,
                    paging_index,
                    row_bound,
                    paging_index_bound,
                    display_lines,
                )
            }
            InputMode::Command => {
                let pos = state.get_chat_position();
                let paging_index = state.chat_paging_index as usize;
                let row_bound = window_height() - CHAT_BOX_HEIGHT - 2;
                let paging_index_bound = state.chat_display.len()
                    - (window_height() as usize - CHAT_BOX_HEIGHT as usize - 1);
                let display_lines = &state.chat_display;

                (
                    pos,
                    paging_index,
                    row_bound,
                    paging_index_bound,
                    display_lines,
                )
            }
        };

    let mut col = pos.0 as usize;
    let mut row = pos.1 as usize;

    let mut chars = state.get_current_line().chars().collect::<Vec<char>>();

    // if we're already in whitespace, find the next word
    while chars.len() == 0 || (col < chars.len() && chars[col as usize] == ' ') {
        col += 1;

        if col >= chars.len() {
            let bounds = state.cursor_position_to_string_position((col as u16, row as u16));

            if bounds.1 + 1 < display_lines.len() as u16 {
                col = 0;

                if row as u16 == row_bound && paging_index < paging_index_bound {
                    paging_index += 1;
                } else {
                    row += 1;
                }

                chars = display_lines[(bounds.1 + 1) as usize]
                    .chars()
                    .collect::<Vec<char>>();
            } else {
                break;
            }
        }
    }

    while col < chars.len() && chars[col as usize] != ' ' {
        col += 1;

        if col == chars.len() {
            let bounds = state.cursor_position_to_string_position((col as u16, row as u16));

            if bounds.1 + 1 < display_lines.len() as u16 {
                col = 0;

                if row as u16 == row_bound && paging_index < paging_index_bound {
                    paging_index += 1;
                } else {
                    row += 1;
                }

                chars = display_lines[(bounds.1 + 1) as usize]
                    .chars()
                    .collect::<Vec<char>>();
            } else {
                break;
            }
        }
    }

    (col as u16, row as u16)
}

fn previous_word(state: &mut State) -> (u16, u16) {
    let (pos, mut paging_index, row_bound, paging_index_bound, display_lines) =
        match state.input_mode {
            InputMode::Edit => {
                let pos = state.get_input_position();
                let paging_index = state.paging_index as usize;
                let row_bound = 0;
                let paging_index_bound = state.paging_index as usize;
                let display_lines = &state.input;

                (
                    pos,
                    paging_index,
                    row_bound,
                    paging_index_bound,
                    display_lines,
                )
            }
            InputMode::Command => {
                let pos = state.get_chat_position();
                let paging_index = state.chat_paging_index as usize;
                let row_bound = 0;
                let paging_index_bound = state.chat_display.len()
                    - (window_height() as usize - CHAT_BOX_HEIGHT as usize - 1);
                let display_lines = &state.chat_display;

                (
                    pos,
                    paging_index,
                    row_bound,
                    paging_index_bound,
                    display_lines,
                )
            }
        };

    let mut col = pos.0 as i16;
    let mut row = pos.1 as i16;

    let mut chars = state.get_current_line().chars().collect::<Vec<char>>();

    // doing this feels like band-aid patch
    // is there a better place to address the bounds?
    col = std::cmp::min(col, chars.len() as i16 - 1);

    // if we're already in whitespace, find the next word
    while chars.len() == 0 || (col > -1 && chars[col as usize] == ' ') {
        col -= 1;

        if col <= 0 {
            let bounds = state.cursor_position_to_string_position((col as u16, row as u16));

            if bounds.1 > 0 {
                if row == row_bound && paging_index > paging_index_bound {
                    paging_index -= 1;
                } else {
                    row -= 1;
                }

                chars = display_lines[(bounds.1 - 1) as usize]
                    .chars()
                    .collect::<Vec<char>>();

                col = chars.len() as i16 - 1;
            } else {
                break;
            }
        }
    }

    while col > -1 && chars[col as usize] != ' ' {
        col -= 1;

        if col <= 0 {
            let bounds = state.cursor_position_to_string_position((col as u16, row as u16));

            if bounds.1 > 0 {
                if row == row_bound && paging_index > paging_index_bound {
                    paging_index -= 1;
                } else {
                    row -= 1;
                }

                chars = display_lines[(bounds.1 - 1) as usize]
                    .chars()
                    .collect::<Vec<char>>();

                col = chars.len() as i16 - 1;
            } else {
                break;
            }
        }
    }

    col = std::cmp::max(col, 0);

    (col as u16, row as u16)
}

// wet code because the borrow checker is horrible
// moving to zig after this project
fn up(state: &mut State) {
    match state.input_mode {
        InputMode::Edit => {
            let row = state.get_input_position().1;
            if row == 0 && state.paging_index > 0 {
                state.paging_index -= 1;
            } else if row > 0 {
                state.input_cursor_position.1 -= 1;

                let line = state.get_current_line();
                state.input_cursor_position.0 = clamp(
                    state.input_cursor_position.0,
                    (line.len() + PROMPT.len()) as u16,
                );

                move_cursor(state.input_cursor_position);
            }
        }
        InputMode::Command => {
            let row = state.get_chat_position().1;
            if row == 0 && state.chat_paging_index > 0 {
                state.chat_paging_index -= 1;
            } else if row > 0 {
                state.highlight_cursor_position.1 -= 1;

                state.highlight_cursor_position.0 = clamp(
                    state.highlight_cursor_position.0,
                    state.get_current_line_length() as u16,
                );

                move_cursor(state.highlight_cursor_position);
            }
        }
    }
}

// wet code because the borrow checker is horrible
// moving to zig after this project
fn down(state: &mut State) {
    match state.input_mode {
        InputMode::Command => {
            let row = state.get_chat_position().1;
            if row + state.chat_paging_index + 1 < state.chat_display.len() as u16 {
                // one above the separator makes minus 2
                if row == window_height() - CHAT_BOX_HEIGHT - 2
                    && row + state.chat_paging_index + 1 < state.chat_display.len() as u16
                {
                    state.chat_paging_index += 1;
                } else {
                    state.highlight_cursor_position.1 += 1;

                    state.highlight_cursor_position.0 = clamp(
                        state.highlight_cursor_position.0,
                        state.get_current_line_length() as u16,
                    );

                    move_cursor(state.highlight_cursor_position);
                }
            }
        }
        InputMode::Edit => {
            let row = state.get_input_position().1;
            if row + state.paging_index < state.input.len() as u16 - 1 {
                if row == CHAT_BOX_HEIGHT - 1
                    && row + state.paging_index + 1 < state.input.len() as u16
                {
                    state.paging_index += 1;
                } else {
                    state.input_cursor_position.1 += 1;

                    let line = state.get_current_line();
                    state.input_cursor_position.0 = clamp(
                        state.input_cursor_position.0,
                        (line.len() + PROMPT.len()) as u16,
                    );

                    move_cursor(state.input_cursor_position);
                }
            }
        }
    }
}

fn page_up(state: &mut State) {
    match state.input_mode {
        InputMode::Command => {
            state.chat_paging_index = std::cmp::max(
                0,
                state.chat_paging_index as i16
                    - (window_height() as i16 - CHAT_BOX_HEIGHT as i16 - 1 as i16),
            ) as u16;

            state.highlight_cursor_position.0 = clamp(
                state.highlight_cursor_position.0,
                state.get_current_line_length() as u16,
            );
        }
        _ => {}
    }
}

fn page_down(state: &mut State) {
    let lower_bound = std::cmp::max(
        state.chat_display.len() as i16 - window_height() as i16 + CHAT_BOX_HEIGHT as i16 + 1,
        0,
    ) as u16;

    match state.input_mode {
        InputMode::Command => {
            state.chat_paging_index = std::cmp::min(
                lower_bound,
                state.chat_paging_index + window_height() - CHAT_BOX_HEIGHT - 1,
            );

            state.highlight_cursor_position.0 = clamp(
                state.highlight_cursor_position.0,
                state.get_current_line_length() as u16,
            );
        }
        _ => {}
    }
}

fn cursor_to_line() {
    print!("\x1B[6 q");
    std::io::stdout().flush().unwrap();
}

fn cursor_to_block() {
    print!("\x1B[2 q");
    std::io::stdout().flush().unwrap();
}

fn chat_submit(state: &mut State) {
    let message = state.input.join("\n").trim().to_string();
    state.push_message(api::Message::new(api::MessageType::User, message));

    state.input = vec![String::new()];

    let origin = input_origin();
    state.input_cursor_position.0 = origin.0;
    state.input_cursor_position.1 = origin.1;
    state.paging_index = 0;

    move_cursor(state.input_cursor_position);
}

fn enter(state: &mut State) -> bool {
    if state.input_mode == InputMode::Edit {
        let pos = state.get_input_position();
        let pos = (pos.0, pos.1 + state.paging_index);
        let remainder = state.input[pos.1 as usize][pos.0 as usize..].to_string();
        state.input[pos.1 as usize] = state.input[pos.1 as usize][..pos.0 as usize].to_string();

        state.input.push(remainder);
        state.input_cursor_position.0 = PROMPT.len() as u16;

        let row = state.get_input_position().1;
        if row == CHAT_BOX_HEIGHT - 1 {
            state.paging_index += 1;
        } else {
            state.input_cursor_position.1 += 1;
        }
    }

    true
}

fn get_clipboard_text() -> String {
    let mut ctx = ClipboardContext::new().unwrap();
    ctx.get_contents().unwrap()
}

// the return signifies whether the program should continue running
fn input(state: &mut State, c: char, key_modifiers: crossterm::event::KeyModifiers) -> bool {
    if state.input_mode == InputMode::Command {
        if c == 'q' {
            return false;
        }

        if c == 'a' {
            state.input_mode = InputMode::Edit;
            cursor_to_line();
            move_cursor(state.input_cursor_position);
        }

        if c == 'y' {
            let (col, row) = state.get_chat_position();
            let line_number = row as usize + state.chat_paging_index as usize;
            let message = state.messages[line_number].content.clone();
            let mut ctx = ClipboardContext::new().unwrap();
            ctx.set_contents(message).unwrap();
        }
    } else {
        if c == 'v' && key_modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
            let clipboard_text = get_clipboard_text();

            if clipboard_text.is_empty() {
                return true;
            }

            let (col, row) = state.get_input_position();

            let mut current_line = state.get_current_line();
            current_line.insert_str(col as usize, &clipboard_text);

            let new_lines = wrap(&current_line, window_width() as usize - 1);
            let new_col = new_lines.last().unwrap().len() as u16 + PROMPT.len() as u16;
            let mut line_number = row as usize + state.paging_index as usize;
            state.input.remove(line_number);

            for line in new_lines.iter() {
                state.input.insert(line_number, line.clone());
                line_number += 1;
            }

            let total_height = window_height();
            state.input_cursor_position.0 = new_col;
            state.input_cursor_position.1 += new_lines.len() as u16 - 1;
            if state.input_cursor_position.1 >= total_height {
                state.paging_index += state.input_cursor_position.1 - total_height + 1;
            }

            state.input_cursor_position.1 =
                std::cmp::min(state.input_cursor_position.1, total_height - 1);

            move_cursor(state.input_cursor_position);
        } else {
            let (col, row) = state.get_input_position();
            let line_number = row as usize + state.paging_index as usize;
            if state.input[line_number].len() == window_width() as usize - 1 {
                state
                    .input
                    .insert(line_number + 1, String::from(format!("{}", c)));
                state.input_cursor_position.0 = PROMPT.len() as u16 + 1;
                state.input_cursor_position.1 += 1;
            } else {
                state.input[line_number].insert(col as usize, c);
                state.input_cursor_position.0 += 1;
            }
        }
    }

    return true;
}

fn backspace(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        let (col, row) = state.get_input_position();
        let line_number = (row + state.paging_index) as usize;
        if col > 0 {
            state.input[line_number].remove(col as usize - 1);
            state.input_cursor_position.0 -= 1;
            move_cursor(state.input_cursor_position);
        } else if line_number > 0 {
            let removed_line = state.input.remove(line_number);
            state.input_cursor_position.0 =
                state.input[line_number - 1].len() as u16 + PROMPT.len() as u16;

            if row > 0 && state.paging_index == 0 {
                state.input_cursor_position.1 -= 1;
            } else if state.paging_index > 0 {
                state.paging_index -= 1;
            }

            move_cursor(state.input_cursor_position);
            state.input[line_number - 1].push_str(&removed_line);
        }
    }
}
