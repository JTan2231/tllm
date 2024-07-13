use std::io::Write;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

mod openai;

// fairly sure this whole program doesn't play nice with unicode

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

        display();
    }
}

const LOG_LOCATION: &str = "./dev.log";

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
            lines.push("".to_string());
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

#[derive(PartialEq, Clone)]
enum InputMode {
    Edit,
    Command,
}

#[derive(Clone)]
struct State {
    messages: Vec<openai::Message>,
    input: Vec<String>,
    input_line: u16,
    input_mode: InputMode,
    paging_index: u16,
    // these 2 are (x, y) coordinates
    // and will _always_ be relative to the terminal window
    input_cursor_position: (u16, u16),
    highlight_cursor_position: (u16, u16),
}

impl State {
    fn clone(&self) -> State {
        State {
            messages: self.messages.clone(),
            input: self.input.clone(),
            input_line: self.input_line,
            input_mode: self.input_mode.clone(),
            paging_index: self.paging_index,
            input_cursor_position: self.input_cursor_position,
            highlight_cursor_position: self.highlight_cursor_position,
        }
    }

    // changing origin from terminal window to input box
    fn get_input_position(&self) -> (u16, u16) {
        let origin = input_origin();
        (
            self.input_cursor_position.0 - origin.0,
            self.input_cursor_position.1 - origin.1,
        )
    }
}

fn return_cursor() {
    execute!(std::io::stdout(), crossterm::cursor::MoveToColumn(0)).unwrap();
}

fn clear_line() {
    execute!(std::io::stdout(), Clear(ClearType::CurrentLine)).unwrap();
}

fn display() {
    enable_raw_mode().unwrap();
    execute!(std::io::stdout(), Clear(ClearType::All)).unwrap();
    print!("{}", PROMPT);
    std::io::stdout().flush().unwrap();

    let mut init = true;

    let (width, height) = crossterm::terminal::size().unwrap();

    let mut state = State {
        messages: Vec::new(),
        input: Vec::from([String::new()]),
        input_line: 0,
        input_mode: InputMode::Command,
        paging_index: 0,
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
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::SHIFT)
                        {
                            shift_enter(&mut state_queue);
                        } else {
                            running = enter(&mut state_queue);
                        }
                    }

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
                        running = input(&mut state_queue, c);
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
                    state.input != state_queue.input || init,
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

fn draw(state: &State, redraw_messages: bool, redraw_input: bool) {
    move_cursor((0, 0));

    if redraw_messages {
        execute!(std::io::stdout(), Clear(ClearType::All)).unwrap();
        // message box
        // this is the chat history
        // minus 1 for the separator
        let message_box_height = crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT - 1;
        let mut message_lines = Vec::new();
        for message in &state.messages {
            message_lines.extend(wrap(
                &message.content,
                crossterm::terminal::size().unwrap().0 as usize - 2,
            ));
            message_lines.push("---".to_string());
        }

        while message_lines.len() < message_box_height as usize {
            message_lines.push("".to_string());
        }

        let mut display_count = 0;
        for line in message_lines.iter().rev() {
            if display_count >= message_box_height {
                break;
            }

            print!("  {}\n", line);
            display_count += 1;
        }

        print!("  ------\n");
    }

    let mut current_row = crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT;
    move_cursor((PROMPT.len() as u16, current_row));

    if redraw_input {
        execute!(std::io::stdout(), Clear(ClearType::CurrentLine)).unwrap();
        // lmfao
        // is this a joke
        //
        // we only want to take lines that will fit in the input box,
        // starting from the end
        let lines = if state.input.len() > CHAT_BOX_HEIGHT as usize {
            &state.input[state.input.len() - CHAT_BOX_HEIGHT as usize..]
        } else {
            &state.input
        };

        // right now, with how displayed_input is set,
        // current_row will always be within the bounds of the input box
        //
        // this should probably be changed in the future
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

    if state.input_mode == InputMode::Edit {
        cursor_to_line();
        move_cursor(state.input_cursor_position);
    } else {
        cursor_to_block();
        move_cursor(state.highlight_cursor_position);
    }
}

fn input_width() -> u16 {
    crossterm::terminal::size().unwrap().0 - 1 - PROMPT.len() as u16
}

fn input_origin() -> (u16, u16) {
    (
        PROMPT.len() as u16,
        crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT,
    )
}

fn move_cursor(position: (u16, u16)) {
    execute!(
        std::io::stdout(),
        crossterm::cursor::MoveTo(position.0, position.1)
    )
    .unwrap();
}

fn left(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        if state.input_cursor_position.0 > PROMPT.len() as u16 {
            state.input_cursor_position.0 -= 1;
            move_cursor(state.input_cursor_position);
        }
    } else {
        if state.highlight_cursor_position.0 > 0 {
            state.highlight_cursor_position.0 -= 1;
            move_cursor(state.highlight_cursor_position);
        }
    }
}

fn right(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        let (col, row) = state.get_input_position();
        if col < state.input[row as usize].len() as u16 {
            state.input_cursor_position.0 += 1;
            move_cursor(state.input_cursor_position);
        }
    }
}

fn up(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        if state.input_cursor_position.1 > crossterm::terminal::size().unwrap().1 - CHAT_BOX_HEIGHT
        {
            state.input_cursor_position.1 -= 1;
            move_cursor(state.input_cursor_position);
        }
    }
}

fn down(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        let row = state.get_input_position().1;
        if row < state.input.len() as u16 - 1 {
            state.input_cursor_position.1 += 1;
            move_cursor(state.input_cursor_position);
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

fn shift_enter(_state: &mut State) {
    panic!("Chat submission not yet implemented!");
}

fn enter(state: &mut State) -> bool {
    if state.input_mode == InputMode::Edit {
        state.input.push(String::new());
        state.input_cursor_position.0 = PROMPT.len() as u16;
        state.input_cursor_position.1 += 1;
    }

    true
}

fn input(state: &mut State, c: char) -> bool {
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
        let (col, row) = state.get_input_position();
        state.input[row as usize].insert(col as usize, c);
        state.input_cursor_position.0 += 1;
    }

    return true;
}

fn backspace(state: &mut State) {
    if state.input_mode == InputMode::Edit {
        let (col, row) = state.get_input_position();
        if col > 0 {
            state.input[row as usize].remove(col as usize - 1);
            state.input_cursor_position.0 -= 1;
            move_cursor(state.input_cursor_position);
        } else if row > 0 {
            state.input.remove(row as usize);
            state.input_cursor_position.0 =
                state.input[(row as usize) - 1].len() as u16 + PROMPT.len() as u16;
            state.input_cursor_position.1 -= 1;
            move_cursor(state.input_cursor_position);
        }
    }
}
