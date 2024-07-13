use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;

#[derive(PartialEq, Clone)]
pub enum MessageType {
    System,
    User,
    Separator,
    Assistant,
}

impl MessageType {
    pub fn to_string(&self) -> String {
        match self {
            MessageType::System => "system".to_string(),
            MessageType::User => "user".to_string(),
            MessageType::Separator => "".to_string(),
            MessageType::Assistant => "assistant".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct Message {
    pub message_type: MessageType,
    pub content: String,
}

impl Message {
    pub fn new(message_type: MessageType, content: String) -> Self {
        Self {
            message_type,
            content,
        }
    }

    pub fn to_string(&self) -> String {
        format!("{}", self.content)
    }
}

// TODO: streaming to interface
pub fn prompt(chat_history: &Vec<Message>) -> String {
    let host = "api.openai.com";
    let path = "/v1/chat/completions";
    let port = 443;
    let body = serde_json::json!({
        "model": "gpt-4o",
        "messages": chat_history.iter().map(|message| {
            serde_json::json!({
                "role": message.message_type.to_string(),
                "content": message.content
            })
        }).collect::<Vec<serde_json::Value>>(),
    });

    let json = serde_json::json!(body);
    let json_string = serde_json::to_string(&json).expect("Failed to serialize JSON");

    let authorization_token =
        env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY environment variable not set");

    let request = format!(
        "POST {} HTTP/1.1\r\n\
        Host: {}\r\n\
        Content-Type: application/json\r\n\
        Content-Length: {}\r\n\
        Authorization: Bearer {}\r\n\
        Connection: close\r\n\r\n\
        {}",
        path,
        host,
        json_string.len(),
        authorization_token,
        json_string
    );

    let stream = TcpStream::connect((host, port)).expect("Failed to connect");

    let connector = native_tls::TlsConnector::new().expect("Failed to create TLS connector");
    let mut stream = connector
        .connect(host, stream)
        .expect("Failed to establish TLS connection");

    stream
        .write_all(request.as_bytes())
        .expect("Failed to write to stream");
    stream.flush().expect("Failed to flush stream");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("Failed to read from stream");

    let response_body = response.split("\r\n\r\n").collect::<Vec<&str>>()[1];
    let mut remaining = response_body;
    let mut decoded_body = String::new();
    while !remaining.is_empty() {
        if let Some(index) = remaining.find("\r\n") {
            let (size_str, rest) = remaining.split_at(index);
            let size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);

            if size == 0 {
                break;
            }

            let chunk = &rest[2..2 + size];
            decoded_body.push_str(chunk);

            remaining = &rest[2 + size + 2..];
        } else {
            break;
        }
    }

    let response_json: serde_json::Value = match serde_json::from_str(&decoded_body) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Failed to parse JSON: {}", e);
            eprintln!("Request: {}", request);
            eprintln!("Raw response: {}", response);
            eprintln!("Response: {}", decoded_body);
            std::process::exit(1);
        }
    };

    let mut content = response_json["choices"][0]["message"]["content"].to_string();
    content = content
        .replace("\\n", "\n")
        .replace("\\\"", "\"")
        .replace("\\'", "'")
        .replace("\\\\", "\\");

    content
}
