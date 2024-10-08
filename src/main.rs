mod api;
mod display;
mod logger;

use crate::logger::Logger;

struct Flags {
    generate_name: bool,
    api: String,
    adhoc: String,
    help: bool,
    load_conversation: String,
}

impl Flags {
    fn new() -> Self {
        Self {
            generate_name: false,
            api: "anthropic".to_string(),
            adhoc: String::new(),
            help: false,
            load_conversation: String::new(),
        }
    }
}

fn create_if_nonexistent(path: &std::path::PathBuf) {
    if !path.exists() {
        match std::fs::create_dir_all(&path) {
            Ok(_) => (),
            Err(e) => panic!("Failed to create directory: {:?}, {}", path, e),
        };
    }
}

fn man() {}

fn parse_flags() -> Result<Flags, Box<dyn std::error::Error>> {
    let mut flags = Flags::new();
    let args: Vec<String> = std::env::args().collect();

    for i in 1..args.len() {
        match args[i].as_str() {
            "-n" => {
                flags.generate_name = !flags.generate_name;
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
            _ => (),
        }
    }

    match flags.api.as_str() {
        "anthropic" => {}
        "openai" => {}
        _ => {
            error!("Invalid API: {}", flags.api);
            return Err("Invalid API".into());
        }
    }

    Ok(flags)
}

const NAME_PROMPT: &str = r#"
you will receive as input a conversation.
respond _only_ with a name for the conversation.
respond with no more than 5 words.
use no punctuation or formatting
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now: String = chrono::Local::now().timestamp_micros().to_string();

    match std::env::var("OPENAI_API_KEY") {
        Ok(_) => (),
        Err(_) => panic!("OPENAI_API_KEY environment variable not set"),
    }

    let home_dir = match std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .or_else(|_| {
            std::env::var("HOMEDRIVE").and_then(|homedrive| {
                std::env::var("HOMEPATH").map(|homepath| format!("{}{}", homedrive, homepath))
            })
        }) {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => panic!("Failed to get home directory"),
    };

    let local_path = home_dir.join(".local/tllm");
    let config_path = home_dir.join(".config/tllm");

    let conversations_path = local_path.join("conversations");
    let logging_path = local_path.join("logs");
    logger::Logger::init(format!(
        "{}/{}.log",
        logging_path.to_str().unwrap(),
        now.clone()
    ));

    create_if_nonexistent(&local_path);
    create_if_nonexistent(&config_path);

    create_if_nonexistent(&conversations_path);
    create_if_nonexistent(&logging_path);

    let args: Vec<String> = std::env::args().collect();

    let mut system_prompt = String::new();
    if let Ok(sp) = std::fs::read_to_string(config_path.join("system_prompt")) {
        system_prompt = sp.trim().to_string();
    }

    let flags = parse_flags()?;

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
        _ => {}
    }

    if flags.adhoc.len() > 0 {
        let mut chat_history = vec![api::Message::new(api::MessageType::User, args[1].clone())];

        let response = api::prompt(&system_prompt, &chat_history)?;

        println!("\n\n{}\n\n", response.content);

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
                println!("Conversation saved to {}", destination);
            }
            Err(e) => {
                println!("Error saving messages: {}", e);
            }
        }
    } else {
        let mut debug = false;
        if let Ok(d) = std::env::var("TLLM_DEBUG") {
            debug = !d.is_empty();
        }

        let messages = display::terminal_app(
            system_prompt.clone(),
            flags.api,
            flags.load_conversation.clone(),
            debug,
        );

        if messages.len() > 0 {
            let mut name = now.clone();
            if flags.generate_name {
                println!("Generating name...");
                match api::prompt(&NAME_PROMPT.to_string(), &messages) {
                    Ok(response) => {
                        name = response.content.clone();
                        name = name.replace(" ", "_").to_lowercase();
                    }
                    Err(e) => {
                        error!("Failed to generate name: {}", e);
                        error!("Conversation: {:?}", messages);
                    }
                }
            }

            let messages_json = serde_json::to_string(&messages).unwrap();
            let destination = conversations_path.join(name.clone());
            let destination = match destination.to_str() {
                Some(s) => {
                    if flags.load_conversation.len() > 0 {
                        format!("{}", flags.load_conversation)
                    } else {
                        format!("{}.json", s)
                    }
                }
                _ => panic!(
                    "Failed to convert path to string: {:?} + {:?}",
                    conversations_path, name
                ),
            };

            match std::fs::write(destination.clone(), messages_json) {
                Ok(_) => {
                    println!("Conversation saved to {}", destination);
                }
                Err(e) => {
                    println!("Error saving messages: {}", e);
                }
            }
        }
    }

    Ok(())
}
