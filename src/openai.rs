use std::env;
use std::io::BufRead;
use std::io::Write;
use std::net::TcpStream;

use crate::display::log;

#[derive(PartialEq, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum MessageType {
    System,
    User,
    Assistant,
}

impl MessageType {
    pub fn to_string(&self) -> String {
        match self {
            MessageType::System => "system".to_string(),
            MessageType::User => "user".to_string(),
            MessageType::Assistant => "assistant".to_string(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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
}

// TODO: streaming to interface
pub fn prompt(chat_history: &Vec<Message>, tx: std::sync::mpsc::Sender<String>) {
    let host = "api.openai.com";
    let path = "/v1/chat/completions";
    let port = 443;
    let body = serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": chat_history.iter().map(|message| {
            serde_json::json!({
                "role": message.message_type.to_string(),
                "content": message.content
            })
        }).collect::<Vec<serde_json::Value>>(),
        "stream": true,
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
        Accept: text/event-stream\r\n\
        Connection: keep-alive\r\n\r\n\
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

    let mut reader = std::io::BufReader::new(stream);
    let mut headers = String::new();
    while reader.read_line(&mut headers).unwrap() > 2 {
        if headers == "\r\n" {
            break;
        }

        headers.clear();
    }

    let mut event_buffer = String::new();
    while reader.read_line(&mut event_buffer).unwrap() > 0 {
        if event_buffer.starts_with("data: ") {
            let payload = event_buffer[6..].trim();

            if payload.is_empty() || payload == "[DONE]" {
                break;
            }

            let response_json: serde_json::Value = match serde_json::from_str(&payload) {
                Ok(json) => json,
                Err(e) => {
                    log(&format!("JSON parse error: {}", e));
                    log(&format!("Error payload: {}", payload));

                    serde_json::Value::Null
                }
            };

            let delta = response_json["choices"][0]["delta"]["content"]
                .to_string()
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
                .replace("\\'", "'")
                .replace("\\\\", "\\");

            if delta != "null" {
                tx.send(delta).expect("Failed to send OAI delta");
            }
        }

        event_buffer.clear();
    }
}
