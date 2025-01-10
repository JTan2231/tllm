#[cfg(debug_assertions)]
const DEBUG: bool = true;
#[cfg(not(debug_assertions))]
const DEBUG: bool = false;

fn create_if_nonexistent(path: &std::path::PathBuf) {
    if !path.exists() {
        match std::fs::create_dir_all(&path) {
            Ok(_) => (),
            Err(e) => panic!("Failed to create directory: {:?}, {}", path, e),
        };
    }
}

fn touch_file(path: &std::path::PathBuf) {
    if !path.exists() {
        match std::fs::File::create(&path) {
            Ok(_) => (),
            Err(e) => panic!("Failed to create file: {:?}, {}", path, e),
        };
    }
}

pub fn get_home_dir() -> std::path::PathBuf {
    match std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .or_else(|_| {
            std::env::var("HOMEDRIVE").and_then(|homedrive| {
                std::env::var("HOMEPATH").map(|homepath| format!("{}{}", homedrive, homepath))
            })
        }) {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => panic!("Failed to get home directory"),
    }
}

pub fn get_config_dir() -> std::path::PathBuf {
    let home_dir = get_home_dir();
    home_dir.join(".config/tllm")
}

pub fn get_local_dir() -> std::path::PathBuf {
    let home_dir = get_home_dir();
    home_dir.join(".local/tllm")
}

pub fn get_conversations_dir() -> std::path::PathBuf {
    let local_dir = get_local_dir();
    local_dir.join("conversations")
}

pub fn setup() {
    if std::env::var("OPENAI_API_KEY").is_err()
        && std::env::var("ANTHROPIC_API_KEY").is_err()
        && std::env::var("GEMINI_API_KEY").is_err()
    {
        panic!("TLLM requires at least $OPENAI_API_KEY, $ANTHROPIC_API_KEY, or $GEMINI_API_KEY to be set");
    }

    let local_path = get_local_dir();
    let config_path = get_config_dir();

    let conversations_path = local_path.join("conversations");
    let logging_path = local_path.join("logs");
    create_if_nonexistent(&logging_path);

    crate::logger::Logger::init(format!("{}/debug.log", logging_path.to_str().unwrap()));

    create_if_nonexistent(&local_path);
    create_if_nonexistent(&config_path);

    create_if_nonexistent(&conversations_path);
}
