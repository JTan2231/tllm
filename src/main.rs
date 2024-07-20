mod display;
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

        // TODO:

        //let response = openai::prompt(&messages);
        //println!(
        //    "{}",
        //    display::wrap(&response, display::window_width() as usize).join("\n")
        //);
    } else {
        let mut config = Flags::new();
        if let Ok(system_prompt) = std::fs::read_to_string("~/.config/tgpt/system_prompt") {
            config.system_prompt = system_prompt;
        }

        display::terminal_app();
    }
}
