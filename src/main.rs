mod display;
mod logger;
mod openai;

use crate::logger::Logger;

struct Flags {
    generate_name: bool,
}

impl Flags {
    fn new() -> Self {
        Self {
            generate_name: true,
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

fn parse_flags() -> Flags {
    let mut flag = Flags::new();
    let args: Vec<String> = std::env::args().collect();

    for i in 1..args.len() {
        match args[i].as_str() {
            "--no-name" => {
                flag.generate_name = false;
            }
            _ => (),
        }
    }

    flag
}

const NAME_PROMPT: &str = r#"
you will receive as input a conversation.
respond _only_ with a name for the conversation.
keep it simple, concise, and precise.
respond with no more than one line or a phrase.
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

    let flags = parse_flags();

    if args.len() > 1 {
        let mut chat_history = vec![openai::Message::new(
            openai::MessageType::User,
            args[1].clone(),
        )];

        let response = openai::prompt(&system_prompt, &chat_history)?;

        println!("\n\n{}\n\n", response.content);

        chat_history.push(response);

        let messages_json = serde_json::to_string(&chat_history).unwrap();
        let destination = conversations_path.join(now.clone());
        let destination = match destination.to_str() {
            Some(s) => format!("{}.json", s),
            None => panic!(
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

        let messages = display::terminal_app(system_prompt.clone(), debug);

        if messages.len() > 0 {
            let mut name = now.clone();
            if flags.generate_name {
                println!("Generating name...");
                match openai::prompt(&NAME_PROMPT.to_string(), &messages) {
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
                Some(s) => format!("{}.json", s),
                None => panic!(
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
