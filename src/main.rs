use std::env;
use std::fs;
use std::process::Command;

use chamber_common::{get_config_dir, get_local_dir, get_root_dir};
use clap::{ArgAction, CommandFactory, Parser};
use tempfile::{Builder, TempPath};

mod sql;

use crate::sql::{Database, Role};

// TODO: There are a lot of unwraps throughout here that should be addressed,
//       I don't think it should be used so liberally

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
//
// TODO: the config shouldn't be in `~/.local/tllm/config/`
fn setup() {
    // TODO: better path config handling
    let home_dir = match std::env::var("HOME") {
        Ok(d) => d,
        Err(e) => {
            panic!("Error getting home variable: {}", e);
        }
    };

    let root = if cfg!(debug_assertions) {
        format!("{}/.local/tllm-dev", home_dir)
    } else {
        format!("{}/.local/tllm", home_dir)
    };

    chamber_common::Workspace::new(&root);

    create_if_nonexistent(&get_local_dir());
    create_if_nonexistent(&get_root_dir().join("logs"));
    create_if_nonexistent(&get_config_dir());

    let log_name = if cfg!(debug_assertions) {
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

enum Shell {
    Bash,
    Zsh,
    Fish,
    Other(String),
}

impl Shell {
    fn from_path(shell_path: &str) -> Self {
        let shell_name = std::path::PathBuf::from(shell_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_lowercase();

        match shell_name.as_str() {
            "bash" => Shell::Bash,
            "zsh" => Shell::Zsh,
            "fish" => Shell::Fish,
            other => Shell::Other(other.to_string()),
        }
    }

    fn get_rc_source_command(&self) -> String {
        match self {
            Shell::Bash => ". ~/.bashrc".to_string(),
            Shell::Zsh => ". ~/.zshrc".to_string(),
            Shell::Fish => "source ~/.config/fish/config.fish".to_string(),
            Shell::Other(_) => "".to_string(),
        }
    }

    fn get_interactive_args(&self) -> Vec<String> {
        match self {
            Shell::Fish => vec!["-C".to_string()],
            _ => vec!["-i".to_string(), "-c".to_string()],
        }
    }
}

fn user_editor(file_contents: &str) -> std::io::Result<String> {
    // Create a temporary file with a custom prefix and suffix
    let temp_file = Builder::new()
        .prefix("tllm_input")
        .suffix(".md")
        .tempfile()?;

    let temp_path: TempPath = temp_file.into_temp_path();

    match std::fs::write(temp_path.to_path_buf(), file_contents) {
        Ok(_) => {}
        Err(e) => {
            println!("Error writing to temp file");
            return Err(e);
        }
    };

    let shell_path = env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let shell = Shell::from_path(&shell_path);

    let command = format!("{} {}", editor, temp_path.to_str().unwrap());
    let rc_source = shell.get_rc_source_command();
    let full_command = if rc_source.is_empty() {
        command
    } else {
        format!("{} && {}", rc_source, command)
    };

    let status = Command::new(shell_path)
        .args(shell.get_interactive_args())
        .arg("-c")
        .arg(&full_command)
        .status()?;

    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Editor command failed",
        ));
    }

    // Read the contents of the temporary file
    let user_message = match fs::read_to_string(&temp_path) {
        Ok(contents) => {
            if contents.is_empty() {
                return Ok(String::new());
            }

            contents
        }
        Err(e) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Error reading file: {}", e),
            ));
        }
    };

    Ok(user_message)
}

/// Opens a read-only file with the given contents in the system editor
/// Only really for reading saved conversations
fn user_reader(file_contents: &str) -> std::io::Result<()> {
    // Create a temporary file with a custom prefix and suffix
    let temp_file = Builder::new()
        .prefix("tllm_conversation")
        .suffix(".md")
        .tempfile()?;

    let temp_path: TempPath = temp_file.into_temp_path();

    match std::fs::write(temp_path.to_path_buf(), file_contents) {
        Ok(_) => {}
        Err(e) => {
            println!("Error writing to temp file");
            return Err(e);
        }
    };

    let mut perms = fs::metadata(temp_path.to_path_buf())?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(temp_path.to_path_buf(), perms)?;

    let shell_path = env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let shell = Shell::from_path(&shell_path);

    let command = format!("{} {}", editor, temp_path.to_str().unwrap());
    let rc_source = shell.get_rc_source_command();
    let full_command = if rc_source.is_empty() {
        command
    } else {
        format!("{} && {}", rc_source, command)
    };

    let status = Command::new(shell_path)
        .args(shell.get_interactive_args())
        .arg("-c")
        .arg(&full_command)
        .status()?;

    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Editor command failed",
        ));
    }

    Ok(())
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Message to send to the LLM
    #[arg(index = 1)]
    message: Option<String>,

    /// Path to a file containing your system prompt. This will be ignored if the path is invalid.
    #[arg(short = 's', long)]
    system_prompt: Option<String>,

    /// List saved conversations
    #[arg(short = 'l', long, conflicts_with = "load_last_conversation", action = ArgAction::SetTrue)]
    list: Option<bool>,

    #[arg(short = 'L', long, conflicts_with = "list", action = ArgAction::SetTrue)]
    load_last_conversation: Option<bool>,

    /// Send a message using the system editor
    #[arg(short = 'e', long, action = ArgAction::SetTrue)]
    editor: Option<bool>,

    /// Choose which LLM provider to use (anthropic or openai)
    #[arg(short = 'p', long)]
    #[arg(value_parser = parse_provider)]
    provider: Option<Provider>,

    /// Open the current conversation in the system editor
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    open: Option<bool>,

    /// Open the system editor for writing after last response.
    /// Useful for continuing a conversation without having to reissue commands.
    /// NOTE: This doesn't do anything if the editor isn't selected
    #[arg(short = 'r', long, action = ArgAction::SetTrue)]
    respond: Option<bool>,

    /// Path to a file for dumping the contents of all conversations
    /// All other flags are ignored if this is set with a valid filepath
    #[arg(short = 'X', long)]
    export_all: Option<String>,

    /// Path to a sqlite database the user would like to use instead of the default
    #[arg(short = 'd', long)]
    database: Option<String>,

    /// Ignore whatever config file set in ~/.config/tllm/
    #[arg(short = 'x', long)]
    no_config: Option<bool>,

    /// Stream output to stdout
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    stream: Option<bool>,
}

#[derive(Debug)]
struct Options {
    message: Option<String>,
    system_prompt: Option<String>,

    list: bool,
    load_last_conversation: bool,
    editor: bool,

    provider: Option<Provider>,

    open: bool,
    respond: bool,

    export_all: Option<String>,
    database: Option<String>,

    stream: bool,
}

trait ConfigParse {
    fn parse_config(value: &str) -> Option<Self>
    where
        Self: Sized;
}

impl ConfigParse for Provider {
    fn parse_config(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAI),
            _ => None,
        }
    }
}

impl ConfigParse for bool {
    fn parse_config(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }
}

impl<T: ConfigParse> ConfigParse for Option<T> {
    fn parse_config(value: &str) -> Option<Self> {
        Some(T::parse_config(value))
    }
}

impl ConfigParse for String {
    fn parse_config(value: &str) -> Option<Self> {
        Some(value.to_string())
    }
}

/// Shorthand for setting up parsing config key-values from the Cli struct
/// Assumes that _all_ CLI options are configurable as defaults
macro_rules! config_fields {
    ($cli:expr, $key:expr, $value:expr, $($field:ident),+ $(,)?) => {
        match $key {
            $(
                stringify!($field) => {
                    $cli.$field = ConfigParse::parse_config($value).unwrap_or($cli.$field);
                }
            )+
            _ => {}
        }
    };
}

/// Read the config file, if it exists
/// Merge valid key=value pairs with the Cli from the CLI parse
/// Return the consolidated struct
/// NOTE: The CLI _always_ takes priority over the config
fn merge_with_config(mut cli: Cli, config_path: &std::path::PathBuf) -> Cli {
    let config_content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(_) => return cli,
    };

    for line in config_content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split on first '=' only
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() != 2 {
            continue;
        }

        let key = parts[0].trim();
        let value = parts[1].trim();

        config_fields!(
            cli,
            key,
            value,
            provider,
            list,
            load_last_conversation,
            editor,
            system_prompt,
            open,
            respond,
            stream
        );
    }

    cli
}

/// This is very stupid
fn cli_to_options(cli: Cli) -> Options {
    Options {
        message: cli.message,
        system_prompt: cli.system_prompt,

        list: cli.list.unwrap_or(false),
        load_last_conversation: cli.load_last_conversation.unwrap_or(false),
        editor: cli.editor.unwrap_or(false),

        provider: cli.provider,

        open: cli.open.unwrap_or(false),
        respond: cli.respond.unwrap_or(false),

        export_all: cli.export_all,
        database: cli.database,

        stream: cli.stream.unwrap_or(false),
    }
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

fn provider_to_api(provider: Option<Provider>) -> wire::types::API {
    // Default to GPT4o if no provider is specified
    if let Some(provider) = provider {
        match provider {
            Provider::Anthropic => {
                wire::types::API::Anthropic(wire::types::AnthropicModel::Claude35Sonnet)
            }
            Provider::OpenAI => wire::types::API::OpenAI(wire::types::OpenAIModel::GPT4o),
        }
    } else {
        wire::types::API::OpenAI(wire::types::OpenAIModel::GPT4o)
    }
}

/// Packages the given user message string for the APIs, sends it, saves, the user message and
/// assistant response, then updates the database with the messages.
/// Returns the title of the current conversation.
async fn send_and_save_message(
    wire: &mut wire::Wire,
    db: &mut sql::Database,
    user_message: String,
    system_prompt: &str,
    conversation_to_load: Option<String>,
    loaded_conversation: Vec<sql::Message>,
    api: wire::types::API,
    stream: bool,
) -> String {
    let messages = {
        let mut new_messages: Vec<wire::types::Message> = loaded_conversation
            .iter()
            .map(|m| wire::types::Message {
                message_type: match m.role {
                    Role::User => wire::types::MessageType::User,
                    Role::Assistant => wire::types::MessageType::Assistant,
                },
                content: m.content.clone(),
                api: api.clone(),
                system_prompt: system_prompt.to_string(),
            })
            .collect();

        new_messages.push(wire::types::Message {
            message_type: wire::types::MessageType::User,
            content: user_message.clone(),
            api: api.clone(),
            system_prompt: String::new(),
        });

        new_messages
    };

    let response = if stream {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle =
            std::thread::spawn(move || wire::prompt_stream(api, "", &messages, tx).unwrap());

        loop {
            match rx.recv() {
                Ok(d) => print!("{}", d),
                Err(_) => break,
            }
        }

        println!("");

        handle.join().unwrap()
    } else {
        match wire.prompt(api, "", &messages).await {
            Ok(r) => r,
            Err(e) => {
                panic!("Error receiving response: {}", e);
            }
        }
    };

    // DB updates

    let conversation_name = match conversation_to_load {
        Some(ref ctl) => ctl.to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };

    let conversation_id = match conversation_to_load {
        Some(ref ctl) => match db.get_conversation(ctl) {
            Ok(c) => c.unwrap().conversation_id,
            Err(_e) => {
                panic!("Error creating conversation");
            }
        },
        None => match db.create_conversation(&conversation_name) {
            Ok(id) => id,
            Err(_e) => {
                panic!("Error creating conversation");
            }
        },
    };

    let user_message_id = match conversation_to_load {
        Some(_) => {
            let messages: Vec<sql::Message> = loaded_conversation.iter().cloned().collect();

            match db.create_message_with_thread(
                &user_message,
                Role::User,
                messages.last().unwrap().message_id,
                conversation_id,
            ) {
                Ok((new_message_id, _)) => new_message_id,
                Err(_e) => {
                    panic!("Error saving user message");
                }
            }
        }
        None => match db.create_message(&user_message, Role::User) {
            Ok(id) => id,
            Err(_e) => {
                panic!("Error saving user message");
            }
        },
    };

    match db.create_message_with_thread(
        &response.content,
        Role::Assistant,
        user_message_id,
        conversation_id,
    ) {
        Ok(_) => {}
        Err(_e) => {
            panic!("Error saving user message");
        }
    };

    if stream {
        println!("\n---\n");
    }

    if loaded_conversation.len() > 0 {
        println!("Updated conversation {}", conversation_name);
    } else {
        println!("New conversation started with title {}", conversation_name);
    }

    println!("---");

    if !stream {
        println!("{}", response.content);
        println!("---");
    }

    conversation_name
}

fn get_conversation_string(
    db: &sql::Database,
    conversation_to_load: Option<String>,
) -> (String, Vec<sql::Message>) {
    match conversation_to_load {
        Some(ref title) => {
            let messages = db.get_conversation_messages(&title).unwrap();
            let mut message_history = format!("\n\n\n{}\n", HISTORY_SEPARATOR);

            for message in messages.iter().rev() {
                message_history
                    .push_str(&format!("\n{}\n\n{}\n", message.content, MESSAGE_SEPARATOR));
            }

            (message_history, messages)
        }
        None => (String::new(), Vec::new()),
    }
}

fn conversation_picker(db: &sql::Database) -> Option<String> {
    let conversations = db.get_conversations().unwrap();

    if conversations.is_empty() {
        println!("There are no stored conversations.");
        return None;
    }

    let mut file_contents = r#"
# Replace the `nothing` in front of the target conversations with your desired operation
# Supported operations:
# - `load`
#
# e.g,
# ```
# nothing 123
# load 456
# nothing 789
#
# Lines starting with # or invalid operations will be ignored
            "#
    .trim()
    .to_string();

    file_contents.push_str("\n\n");

    let preview_separator = format!(
        "\n\n\n{} CONVERSATION PREVIEWS {}\n\n\n",
        MESSAGE_SEPARATOR, MESSAGE_SEPARATOR
    );

    let mut previews = preview_separator.clone();

    // .rev() for displaying the latest conversations first
    for conv in conversations.iter().rev() {
        file_contents.push_str(&format!("nothing {}\n", conv.title));

        let messages = db.get_conversation_messages(&conv.title).unwrap();
        let messages_concatenated = &messages
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<String>>()
            .join(&format!("\n\n{}\n\n", MESSAGE_SEPARATOR));

        let messages_truncated =
            &messages_concatenated[..std::cmp::min(256, messages_concatenated.len())];

        previews.push_str(&format!(
            "\n\n\n {} {} {} \n\n\n {}",
            MESSAGE_SEPARATOR, conv.title, MESSAGE_SEPARATOR, messages_truncated
        ));
    }

    file_contents.push_str(&previews);

    let user_input = user_editor(&file_contents).unwrap();

    // TODO: This vector is only really suitable for one operation at a time
    let mut operations = vec![];
    for line in user_input.lines() {
        if line.starts_with("#") {
            continue;
        } else if line.starts_with(&preview_separator) {
            break;
        }

        match line.split_whitespace().collect::<Vec<_>>()[..] {
            ["load", title, ..] => operations.push(title.to_string()),
            _ => continue,
        }
    }

    if operations.len() > 1 {
        println!("Multiple loads not supported, aborting operation.");
        return None;
    }

    if operations.is_empty() {
        println!("No conversations selected.");
        None
    } else {
        Some(operations[0].clone())
    }
}

/// Parses the CLI and conslidates that with the user-defined config defaults
fn get_options() -> Options {
    let mut cli = Cli::parse();

    if let None = cli.no_config {
        cli = merge_with_config(cli, &get_config_dir().join("config"));
    }

    cli_to_options(cli)
}

const HISTORY_SEPARATOR: &str = "======== MESSAGE HISTORY ========";
const MESSAGE_SEPARATOR: &str = "========";

/// Commands:
/// - Default usage: tllm <message>
///   - This just spits the response out into the terminal
///
/// See the `Cli` struct for flags
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: A lot of panics around here that need taken care of

    setup();
    let mut wire = wire::Wire::new(None).await.unwrap();

    let cli = get_options();

    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 && !get_config_dir().join("config").exists() {
        Cli::command().print_help().unwrap();
        std::process::exit(1);
    }

    let db_path = if let Some(db) = cli.database {
        std::path::PathBuf::from(db)
    } else {
        chamber_common::get_local_dir().join("tllm.sqlite")
    };

    // TODO: Fallback for when the db can't be setup
    let mut db = match Database::new(&db_path) {
        Ok(db) => db,
        Err(e) => panic!("Error setting up DB! {}", e),
    };

    match cli.export_all {
        Some(f) => {
            let mut full_conv_string = String::new();

            let conversations = db.get_conversations()?;
            for conv in conversations {
                let (conv_string, _) = get_conversation_string(&db, Some(conv.title.clone()));

                full_conv_string.push_str(&format!(
                    "{} {} {} \n\n\n {} \n\n\n",
                    MESSAGE_SEPARATOR, conv.title, MESSAGE_SEPARATOR, conv_string
                ));
            }

            std::fs::write(f.clone(), &full_conv_string)?;
            println!("Chat history exported to {}", f);

            return Ok(());
        }
        None => {}
    };

    let system_prompt = match cli.system_prompt {
        Some(ref f) => match std::fs::read_to_string(f) {
            Ok(c) => c,
            Err(_) => String::new(),
        },
        None => String::new(),
    };

    // Title (not ID!) of the target conversation to load/refer
    let conversation_to_load = match cli.list {
        true => conversation_picker(&db),
        false => None,
    };

    let conversation_to_load = match cli.load_last_conversation {
        true => {
            let last_conversation = db.get_last_updated_conversation()?;

            match last_conversation {
                Some(lc) => Some(lc.title),
                None => {
                    println!("No conversation found.");
                    None
                }
            }
        }
        false => conversation_to_load,
    };

    let mut current_conversation = if conversation_to_load.is_some() {
        conversation_to_load.clone().unwrap()
    } else {
        String::new()
    };

    // Example usage of the parsed arguments
    match cli.editor {
        true => {
            let api = provider_to_api(cli.provider.clone());

            let (conversation_string, loaded_conversation) =
                get_conversation_string(&db, conversation_to_load.clone());

            let file_contents = user_editor(&conversation_string)?;
            let mut contents_split = file_contents.split(HISTORY_SEPARATOR);
            let user_message = contents_split.next();

            if !(user_message.is_none() || user_message.unwrap().trim().is_empty()) {
                let user_message = user_message.unwrap().trim().to_string();

                current_conversation = send_and_save_message(
                    &mut wire,
                    &mut db,
                    user_message,
                    &system_prompt,
                    conversation_to_load.clone(),
                    loaded_conversation,
                    api,
                    cli.stream,
                )
                .await;
            } else {
                println!("User input empty, operation aborted.");
            }
        }
        false => {}
    }

    if let Some(ref message) = cli.message {
        let api = provider_to_api(cli.provider.clone());

        let (_, loaded_conversation) = get_conversation_string(&db, conversation_to_load.clone());

        // If the message is a filepath, try and load the contents of the file as the chat
        // message
        let user_message = {
            let path = std::path::PathBuf::try_from(message.clone());
            if path.is_ok() {
                match std::fs::read_to_string(path.unwrap()) {
                    Ok(c) => c,
                    Err(_) => message.to_string(),
                }
            } else {
                message.to_string()
            }
        };

        current_conversation = send_and_save_message(
            &mut wire,
            &mut db,
            user_message,
            &system_prompt,
            conversation_to_load.clone(),
            loaded_conversation,
            api,
            cli.stream,
        )
        .await;
    }

    if cli.respond && cli.editor {
        // We don't want this to run if no conversation has already been set
        // If there's nothing set, that means there's no conversation to respond to
        // and the opportunity to have started a new one has already passed
        while current_conversation.len() > 0 {
            let api = provider_to_api(cli.provider.clone());

            let (conversation_string, loaded_conversation) =
                get_conversation_string(&db, Some(current_conversation.clone()));

            let file_contents = user_editor(&conversation_string)?;
            let mut contents_split = file_contents.split(HISTORY_SEPARATOR);
            let user_message = contents_split.next();

            if user_message.is_none() || user_message.unwrap().trim().is_empty() {
                println!("Empty input, operation aborted.");
                return Ok(());
            }

            let user_message = user_message.unwrap().trim().to_string();

            current_conversation = send_and_save_message(
                &mut wire,
                &mut db,
                user_message,
                &system_prompt,
                Some(current_conversation),
                loaded_conversation,
                api,
                cli.stream,
            )
            .await;
        }
    }

    if cli.open {
        current_conversation = if current_conversation.is_empty() {
            match conversation_picker(&db) {
                Some(c) => c,
                None => return Ok(()),
            }
        } else {
            current_conversation
        };

        let (conversation_string, _) = get_conversation_string(&db, Some(current_conversation));
        user_reader(&conversation_string)?;
    }

    Ok(())
}
