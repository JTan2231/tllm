use copypasta::{ClipboardContext, ClipboardProvider};
use std::io::Write;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

mod openai;

// fairly sure this whole program doesn't play nice with unicode
//
// TODO: I think the paging logic needs abstracted
//       to each individual window, the state struct is
//       starting to feel a little bloated

#[derive(Debug)]
struct Flags {
    system_prompt: String,
    system_prompt_file: String,
    user_prompt: String,
    user_prompt_file: String,
}

impl Flags {
    fn new() -> Flags {
        Flags {
            system_prompt: String::new(),
            system_prompt_file: String::new(),
            user_prompt: String::new(),
            user_prompt_file: String::new(),
        }
    }
}

enum Flag {
    SystemPrompt,
    SystemPromptFile,
    UserPrompt,
    UserPromptFile,
}

impl Flag {
    fn from_str(s: &str) -> Option<Flag> {
        match s {
            "-S" => Some(Flag::SystemPrompt),
            "-s" => Some(Flag::SystemPromptFile),
            "-U" => Some(Flag::UserPrompt),
            "-u" => Some(Flag::UserPromptFile),
            _ => None,
        }
    }
}

const PROMPT: &str = "  ";
const CHAT_BOX_HEIGHT: u16 = 10;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    fn index_check(i: usize, args: &Vec<String>, flag: &str) {
        if i + 1 >= args.len() {
            panic!("Missing argument for flag: {}", flag);
        }
    }

    macro_rules! read_input_flag {
        ($flag:expr, $index:expr, $args:expr, $arg:expr) => {
            index_check($index, $args, $arg);
            $flag = $args[$index + 1].clone();

            $index += 2;
        };
    }

    if args.len() > 1 {
        let mut flags = Flags::new();
        let mut i = 1;
        while i < args.len() {
            let arg = &args[i];
            match Flag::from_str(arg) {
                Some(Flag::SystemPrompt) => {
                    if !flags.system_prompt_file.is_empty() {
                        panic!("Use either file or inline system prompt, not both");
                    }

                    read_input_flag!(flags.system_prompt, i, &args, arg);
                }
                Some(Flag::SystemPromptFile) => {
                    if !flags.system_prompt.is_empty() {
                        panic!("Use either file or inline system prompt, not both");
                    }

                    flags.system_prompt_file = std::fs::read_to_string(&args[i + 1]).unwrap();

                    i += 2;
                }
                Some(Flag::UserPrompt) => {
                    if !flags.user_prompt_file.is_empty() {
                        panic!("Use either file or inline user prompt, not both");
                    }

                    read_input_flag!(flags.user_prompt, i, &args, arg);
                }
                Some(Flag::UserPromptFile) => {
                    if !flags.user_prompt.is_empty() {
                        panic!("Use either file or inline user prompt, not both");
                    }

                    flags.user_prompt_file = std::fs::read_to_string(&args[i + 1]).unwrap();

                    i += 2;
                }
                None => {
                    if !flags.user_prompt_file.is_empty() {
                        panic!("Use either file or inline user prompt, not both");
                    }

                    if flags.user_prompt.is_empty() {
                        flags.user_prompt = arg.clone();
                        i += 1;
                    } else {
                        panic!("Unknown argument: {}", arg);
                    }
                }
            }
        }

        let system_prompt = if flags.system_prompt.is_empty() {
            flags.system_prompt_file
        } else {
            flags.system_prompt
        };

        let user_prompt = if flags.user_prompt.is_empty() {
            flags.user_prompt_file
        } else {
            flags.user_prompt
        };

        let messages = vec![
            openai::Message::new(openai::MessageType::System, system_prompt),
            openai::Message::new(openai::MessageType::User, user_prompt),
        ];

        let response = openai::prompt(&messages);
        println!("{}", wrap(&response, 20).join("\n"));
    } else {
        let mut config = Flags::new();
        if let Ok(system_prompt) = std::fs::read_to_string("~/.config/tgpt/system_prompt") {
            config.system_prompt = system_prompt;
        }

        terminal_app();
    }
}

const LOG_LOCATION: &str = "./dev";

fn log(message: &String) {
    let log_message = format!("{}: {}\n", chrono::Local::now(), message);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_LOCATION)
        .unwrap();

    file.write_all(log_message.as_bytes()).unwrap();
}

// wraps text around a given width
// line breaks on spaces
fn wrap(text: &String, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let chars = text.chars().collect::<Vec<char>>();

    let mut i = 0;
    while i < chars.len() {
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

#[derive(PartialEq, Clone, Debug)]
enum InputMode {
    Edit,
    Command,
}

#[derive(Clone, Debug)]
struct State {
    messages: Vec<openai::Message>,
    input: Vec<String>,
    input_mode: InputMode,
    paging_index: u16,
    chat_paging_index: u16,
    // these 2 are (x, y) coordinates
    // and will _always_ be relative to the terminal window
    input_cursor_position: (u16, u16),
    highlight_cursor_position: (u16, u16),
}

// NOTE: all of these currently only account for the input paging index
//       really, the chat_paging index et al. should be accounted for 
//       in an entirely different struct
impl State {
    // changing origin from terminal window to input box
    //
    // this _DOES NOT_ adjust for paging
    fn get_input_position(&self) -> (u16, u16) {
        let origin = input_origin();
        (
            self.input_cursor_position.0 - origin.0,
            self.input_cursor_position.1 - origin.1,
        )
    }

    fn get_current_line_number(&self) -> usize {
        self.get_input_position().1 + self.paging_index
    }

    fn get_current_line(&self) -> String {
        let row = self.get_input_position().1;
        self.input[(self.paging_index + row) as usize].clone()
    }
}

fn terminal_app() {
    enable_raw_mode().unwrap();
    execute!(std::io::stdout(), Clear(ClearType::All)).unwrap();
    print!("{}", PROMPT);
    std::io::stdout().flush().unwrap();

    let mut init = true;

    let height = crossterm::terminal::size().unwrap().1;

    let mut state = State {
        messages: Vec::new(),
        input: Vec::from([String::new()]),
        input_mode: InputMode::Command,
        paging_index: 0,
        chat_paging_index: 0,
        input_cursor_position: (PROMPT.len() as u16, height - CHAT_BOX_HEIGHT),
        highlight_cursor_position: (PROMPT.len() as u16, height - CHAT_BOX_HEIGHT),
    };

    let mut state_queue = state.clone();

    let mut running = true;
    while running {
        match event::poll(std::time::Duration::from_millis(5)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key_event)) => match key_event.code {
                    KeyCode::Enter => {
                        // KeyModifiers don't work on Enter??
                        /*if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        {*/
                        if state.input_mode == InputMode::Command {
                            chat_submit(&mut state_queue);
                        } else {
                            running = enter(&mut state_queue);
                        }
                    }

                    // why does this take so long?
                    KeyCode::Esc => {
                        state_queue.input_mode = InputMode::Command;
                        cursor_to_block();
                        move_cursor(state_queue.highlight_cursor_position);
                    }

                    KeyCode::Left => {
                        left(&mut state_queue);
                    }

                    KeyCode::Right => {
                        right(&mut state_queue);
                    }

                    KeyCode::Up => {
                        up(&mut state_queue);
                    }

                    KeyCode::Down => {
                        down(&mut state_queue);
                    }

                    KeyCode::Backspace => {
                        backspace(&mut state_queue);
                    }

                    KeyCode::Char(c) => {
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
                draw(
                    &state_queue,
                    state.messages.len() != state_queue.messages.len() || init,
                    state.input != state_queue.input
                        || init
                        || state.paging_index != state_queue.paging_index,
                );

                state = state_queue.clone();
                init = false;
            }
            Err(e) => {
                panic!("Error polling events: {}", e);
            }
        }
    }

    println!("\nExit");
    cursor_to_block();
    disable_raw_mode().unwrap();
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

fn draw(state: &State, redraw_messages: bool, redraw_input: bool) {
    move_cursor((0, 0));

    if redraw_messages {
        execute!(std::io::stdout(), Clear(ClearType::All)).unwrap();

        // minus 1 for the separator
        let message_box_height =
            (crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT - 1) as usize;
        let mut message_lines = Vec::new();
        for message in &state.messages {
            message_lines.extend(wrap(
                &message.content,
                crossterm::terminal::size().unwrap().0 as usize - 2,
            ));
            message_lines.push("---".to_string());
        }

        while message_lines.len() < message_box_height {
            message_lines.push("".to_string());
        }

        let paging_index = state.chat_paging_index as usize;
        let display_lines = &message_lines
            [paging_index..std::cmp::min(paging_index + message_box_height, message_lines.len())];

        draw_lines(display_lines, 0);

        move_cursor((
            PROMPT.len() as u16,
            crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT - 1,
        ));
        print!("------");
    }

    let current_row = crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT;
    move_cursor((PROMPT.len() as u16, current_row));

    if redraw_input {
        execute!(std::io::stdout(), Clear(ClearType::CurrentLine)).unwrap();
        let h = CHAT_BOX_HEIGHT as usize;
        let paging_index = state.paging_index as usize;
        let lines = if state.input.len() > h {
            &state.input[paging_index..std::cmp::min(paging_index + h, state.input.len())]
        } else {
            &state.input
        };

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

fn window_width() -> u16 {
    crossterm::terminal::size().unwrap().0
}

fn move_cursor(position: (u16, u16)) {
    execute!(
        std::io::stdout(),
        crossterm::cursor::MoveTo(position.0, position.1)
    )
    .unwrap();
}

fn clamp_input_position(state: &mut State) {
    let col = state.get_input_position().0;
    let current_line = state.get_current_line();
    if col >= current_line.len() as u16 {
        state.input_cursor_position.0 = current_line.len() as u16 + PROMPT.len() as u16;
    }
}

fn left(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        if state.input_cursor_position.0 > PROMPT.len() as u16 {
            state.input_cursor_position.0 -= 1;
            move_cursor(state.input_cursor_position);
        }
    }
}

fn right(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        let col = state.get_input_position().0;
        let current_line = state.get_current_line();
        if col < current_line.len() as u16 {
            state.input_cursor_position.0 += 1;
            move_cursor(state.input_cursor_position);
        }
    }
}

fn up(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        let row = state.get_input_position().1;
        if row == 0 && state.paging_index > 0 {
            state.paging_index -= 1;
        } else if row > 0 {
            state.input_cursor_position.1 -= 1;
            clamp_input_position(state);
            move_cursor(state.input_cursor_position);
        }
    }
}

fn down(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        let row = state.get_input_position().1;
        if row + state.paging_index < state.input.len() as u16 - 1 {
            if row == CHAT_BOX_HEIGHT - 1 && row + state.paging_index + 1 < state.input.len() as u16
            {
                state.paging_index += 1;
            } else {
                state.input_cursor_position.1 += 1;
                clamp_input_position(state);
                move_cursor(state.input_cursor_position);
            }
        }
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
    state
        .messages
        .push(openai::Message::new(openai::MessageType::User, message));

    state.input = vec![String::new()];
    let origin = input_origin();
    state.input_cursor_position.0 = origin.0;
    state.input_cursor_position.1 = origin.1;

    move_cursor(state.input_cursor_position);
}

fn enter(state: &mut State) -> bool {
    if state.input_mode == InputMode::Edit {
        state.input.push(String::new());
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

fn input(state: &mut State, c: char, key_modifiers: crossterm::event::KeyModifiers) -> bool {
    if state.input_mode == InputMode::Command {
        if c == 'a' {
            state.input_mode = InputMode::Edit;
            cursor_to_line();
            move_cursor(state.input_cursor_position);

            return true;
        }

        if c == 'q' {
            return false;
        }
    } else {
        if c == 'v' && key_modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
            let clipboard_text = wrap(&get_clipboard_text(), window_width() as usize);

            if clipboard_text.is_empty() {
                return true;
            }

            let (col, row) = state.get_input_position();
            let remainder = &state.input[col..];

            let mut added_lines = 0;
            for line in clipboard_text {
                state.input[
        }

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
