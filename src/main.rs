mod config;
mod display;
mod logger;
mod network;

use crate::logger::Logger;

struct Flags {
    save_conversation: bool,
    api: String,
    adhoc: String,
    help: bool,
    system_prompt: String,
    load_conversation: String,
}

impl Flags {
    fn new() -> Self {
        Self {
            save_conversation: true,
            api: "anthropic".to_string(),
            adhoc: String::new(),
            help: false,
            system_prompt: String::new(),
            load_conversation: String::new(),
        }
    }
}

fn man() {
    println!("Usage: tllm [OPTIONS] [TEXT]");
    println!("\nOptions:");
    println!("\t-n\t\tDo not save the file");
    println!("\t-a API\t\tUse the specified API (anthropic, openai)");
    println!("\t-i TEXT\t\tUse the specified text as an ad-hoc prompt");
    println!("\t-h\t\tDisplay this help message");
    println!("\t-l FILE\t\tLoad a conversation from the specified file");
    println!("\t-s TEXT or FILE\t\tUse the specified text/file as the system prompt");
}

fn parse_flags() -> Result<Flags, Box<dyn std::error::Error>> {
    let mut flags = Flags::new();
    let args: Vec<String> = std::env::args().collect();

    for i in 1..args.len() {
        match args[i].as_str() {
            "-n" => {
                flags.save_conversation = !flags.save_conversation;
            }
            "-a" => {
                if i + 1 < args.len() {
                    flags.api = args[i + 1].clone();
                } else {
                    man();
                    return Err("API flag -a requires an argument".into());
                }
            }
            "-i" => {
                if i + 1 < args.len() {
                    flags.adhoc = args[i + 1].clone();
                } else {
                    man();
                    return Err("API flag -i requires an argument".into());
                }
            }
            "-h" => {
                flags.help = true;
            }
            "-l" => {
                if i + 1 < args.len() {
                    let filepath = std::path::PathBuf::from(args[i + 1].clone());
                    if !filepath.exists() {
                        error!("File does not exist: {:?}", filepath);
                        return Err("File does not exist".into());
                    }

                    flags.load_conversation = args[i + 1].clone();
                } else {
                    man();
                    return Err("API flag -l requires a filepath argument".into());
                }
            }
            "-s" => {
                if i + 1 < args.len() {
                    flags.system_prompt = args[i + 1].clone();
                } else {
                    man();
                    return Err("API flag -s requires an argument".into());
                }
            }
            _ => (),
        }
    }

    match flags.api.as_str() {
        "anthropic" => {}
        "openai" => {}
        "gemini" => {}
        _ => {
            error!("Invalid API: {}", flags.api);
            return Err("Invalid API".into());
        }
    }

    Ok(flags)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now: String = chrono::Local::now().timestamp_micros().to_string();
    config::setup();

    let config_path = config::get_config_dir();
    let conversations_path = config::get_conversations_dir();

    let flags = parse_flags()?;

    let system_prompt = match flags.system_prompt.len() {
        0 => {
            let system_prompt_path = config_path.join("system_prompt");
            if system_prompt_path.exists() {
                match std::fs::read_to_string(system_prompt_path) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to read system prompt: {}", e);
                        String::new()
                    }
                }
            } else {
                String::new()
            }
        }
        _ => {
            let system_prompt_path = std::path::PathBuf::from(flags.system_prompt.clone());
            if system_prompt_path.exists() {
                match std::fs::read_to_string(system_prompt_path) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to read system prompt: {}", e);
                        String::new()
                    }
                }
            } else {
                flags.system_prompt.clone()
            }
        }
    };

    if flags.help {
        man();
        return Ok(());
    }

    match flags.api.as_str() {
        "anthropic" => match std::env::var("ANTHROPIC_API_KEY") {
            Ok(_) => (),
            Err(_) => panic!("ANTHROPIC_API_KEY environment variable not set"),
        },
        "openai" => match std::env::var("OPENAI_API_KEY") {
            Ok(_) => (),
            Err(_) => panic!("OPENAI_API_KEY environment variable not set"),
        },
        "gemini" => match std::env::var("GEMINI_API_KEY") {
            Ok(_) => (),
            Err(_) => panic!("GEMINI_API_KEY environment variable not set"),
        },
        _ => {}
    }

    if flags.adhoc.len() > 0 {
        let adhoc = if std::path::PathBuf::from(&flags.adhoc).exists() {
            std::fs::read_to_string(flags.adhoc.clone())?
        } else {
            flags.adhoc.clone()
        };

        let mut chat_history = vec![network::Message::new(network::MessageType::User, adhoc)];

        let response = network::prompt(&flags.api, &system_prompt, &chat_history)?;
        let content = response.content.replace("\\n", "\n");

        println!("{}\n\n", content);

        if flags.save_conversation {
            chat_history.push(response);

            let messages_json = serde_json::to_string(&chat_history).unwrap();
            let destination = conversations_path.join(now.clone());
            let destination = match destination.to_str() {
                Some(s) => format!("{}.json", s),
                _ => panic!(
                    "Failed to convert path to string: {:?} + {:?}",
                    conversations_path, now
                ),
            };

            match std::fs::write(destination.clone(), messages_json) {
                Ok(_) => {
                    info!("Conversation saved to {}", destination);
                }
                Err(e) => {
                    info!("Error saving messages: {}", e);
                }
            }
        }
    } else {
        match display::display_manager(
            display::WindowView::Chat,
            &system_prompt,
            &flags.api,
            flags.load_conversation.clone(),
        ) {
            Ok(_) => {}
            Err(e) => panic!("error in display manager: {}", e),
        };
    }

    Ok(())
}
