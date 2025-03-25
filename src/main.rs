use std::env;
use std::fs;
use std::process::Command;

use chamber_common::{get_local_dir, get_root_dir};
use clap::{ArgGroup, Parser};

mod sql;

fn create_if_nonexistent(path: &std::path::PathBuf) {
    if !path.exists() {
        match std::fs::create_dir_all(&path) {
            Ok(_) => (),
            Err(e) => panic!("Failed to create directory: {:?}, {}", path, e),
        };
    }
}

// Normally this would return an error
// but if this fails then the app can't run correctly
fn setup() {
    // TODO: better path config handling
    let home_dir = match std::env::var("HOME") {
        Ok(d) => d,
        Err(e) => {
            panic!("Error getting home variable: {}", e);
        }
    };

    let root = if cfg!(dev) {
        format!("{}/.local/tllm-dev", home_dir)
    } else {
        format!("{}/.local/tllm", home_dir)
    };

    chamber_common::Workspace::new(&root);

    create_if_nonexistent(&get_local_dir());
    create_if_nonexistent(&get_root_dir().join("logs"));

    let log_name = if cfg!(dev) {
        "debug".to_string()
    } else {
        format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros()
        )
    };

    // TODO: proper logging, obviously
    chamber_common::Logger::init(
        get_root_dir()
            .join("logs")
            .join(format!("{}.log", log_name))
            .to_str()
            .unwrap(),
    );
}

fn user_editor() -> std::io::Result<()> {
    let temp_file = "temp_input.txt";

    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());

    let status = Command::new(&editor).arg(temp_file).status()?;

    if !status.success() {
        println!("Editor command failed!");
        return Ok(());
    }

    match fs::read_to_string(temp_file) {
        Ok(contents) => {
            if contents.trim().is_empty() {
                println!("File is empty!");
            } else {
                println!("File contents:");
                println!("{}", contents);

                let word_count = contents.split_whitespace().count();
                println!("Word count: {}", word_count);
            }
        }
        Err(e) => {
            println!("Error reading file: {}", e);
        }
    }

    // Clean up: remove the temporary file
    fs::remove_file(temp_file)?;

    Ok(())
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(group(
    ArgGroup::new("commands")
        .args(["page", "page_with_id", "editor"])
        .required(false)
))]
struct Cli {
    /// Message to send to the LLM
    #[arg(index = 1)]
    message: Option<String>,

    /// Page through the last conversation
    #[arg(short = 'p')]
    page: bool,

    /// Page through the conversation with the given ID
    #[arg(short = 'P')]
    page_with_id: Option<String>,

    /// Send a message using the system editor
    #[arg(short = 'e')]
    editor: bool,

    /// Choose which LLM provider to use (anthropic or openai)
    #[arg(short = 'a')]
    #[arg(value_parser = parse_provider)]
    provider: Option<Provider>,
}

#[derive(Debug, Clone)]
enum Provider {
    Anthropic,
    OpenAI,
}

fn parse_provider(s: &str) -> Result<Provider, String> {
    match s.to_lowercase().as_str() {
        "anthropic" => Ok(Provider::Anthropic),
        "openai" => Ok(Provider::OpenAI),
        _ => Err(format!(
            "Invalid provider: {}. Must be 'anthropic' or 'openai'",
            s
        )),
    }
}

/// Commands:
/// - Default usage: tllm <message>
///   - This just spits the response out into the terminal
/// - p
///   - Page through the last conversation
/// - P <conversation id>
///   - Page through the conversation with the given ID
/// - e
///   - Send a message with whatever editor is set in $EDITOR
/// - l [TODO]
///   - List saved conversations
///
///   - NOTE: If you are responding to a conversation (e.g., -ep or -eP),
///           the conversation will display in the editor
///
/// - a <anthropic|openai>
///   - Choose which LLM provider to use
///   - Assumes that the appropriate API key is set:
///     - `anthropic` -- `$ANTHROPIC_API_kEY`
///     - `openai`    -- `$OPENAI_API_KEY`
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup();
    let mut wire = wire::Wire::new(None, None, Some("http://localhost:8000".to_string()))
        .await
        .unwrap();

    let args = std::env::args();

    let cli = Cli::parse();

    // Example usage of the parsed arguments
    match (cli.page, cli.page_with_id, cli.editor, cli.message) {
        (true, None, false, None) => {
            // TODO: Handle 'p' command
        }
        (false, Some(id), false, None) => {
            // TODO: Handle 'P' command with ID
        }
        (false, None, true, None) => {
            // TODO: Handle 'e' command
        }
        (false, None, false, Some(message)) => {
            // Default to GPT4o if no provider is specified
            let api = if let Some(provider) = cli.provider {
                match provider {
                    Provider::Anthropic => {
                        wire::types::API::Anthropic(wire::types::AnthropicModel::Claude35Sonnet)
                    }
                    Provider::OpenAI => wire::types::API::OpenAI(wire::types::OpenAIModel::GPT4o),
                }
            } else {
                wire::types::API::OpenAI(wire::types::OpenAIModel::GPT4o)
            };

            // If the message is a filepath, try and load the contents of the file as the chat
            // message
            let message = {
                let path = std::path::PathBuf::try_from(message.clone());
                if path.is_ok() {
                    match std::fs::read_to_string(path.unwrap()) {
                        Ok(c) => c,
                        Err(_) => message,
                    }
                } else {
                    message
                }
            };

            let messages = vec![wire::types::Message {
                message_type: wire::types::MessageType::User,
                content: message,
                api: api.clone(),
                system_prompt: String::new(),
            }];

            let response = match wire.prompt(api, "", &messages).await {
                Ok(r) => r,
                Err(e) => {
                    panic!("Error receiving response: {}", e);
                }
            };

            println!("{}", response.content);
        }
        _ => {
            println!("Invalid combination of arguments");
        }
    }

    // Default case
    // TODO: until CLI parsing is done
    //       this will assume there is only one argument
    for (i, arg) in args.enumerate() {
        println!("{}, {}", i, arg);
        if i == 1 {}
    }

    Ok(())
}
