use native_tls::TlsStream;
use std::env;
use std::io::BufRead;
use std::io::{Read, Write};
use std::net::TcpStream;

use crate::logger::Logger;
use crate::{error, info};

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

struct RequestParams {
    host: String,
    path: String,
    port: u16,
    messages: Vec<Message>,
    model: String,
    stream: bool,
    authorization_token: String,
    max_tokens: Option<u16>,
    system_prompt: Option<String>,
}

fn build_request(params: &RequestParams) -> String {
    let body = serde_json::json!({
        "model": params.model,
        "messages": params.messages.iter().map(|message| {
            serde_json::json!({
                "role": message.message_type.to_string(),
                "content": message.content
            })
        }).collect::<Vec<serde_json::Value>>(),
        "stream": params.stream,
    });

    let json = serde_json::json!(body);
    let json_string = serde_json::to_string(&json).expect("Failed to serialize JSON");

    format!(
        "POST {} HTTP/1.1\r\n\
        Host: {}\r\n\
        Content-Type: application/json\r\n\
        Content-Length: {}\r\n\
        Authorization: Bearer {}\r\n\
        Connection: close\r\n\r\n\
        {}",
        params.path,
        params.host,
        json_string.len(),
        params.authorization_token,
        json_string
    )
}

fn get_openai_request_params(system_prompt: String, chat_history: &Vec<Message>) -> RequestParams {
    RequestParams {
        host: "api.openai.com".to_string(),
        path: "/v1/chat/completions".to_string(),
        port: 443,
        messages: vec![Message::new(MessageType::System, system_prompt.clone())]
            .iter()
            .chain(chat_history.iter())
            .cloned()
            .collect::<Vec<Message>>(),
        model: "gpt-4o-mini".to_string(),
        stream: false,
        authorization_token: env::var("OPENAI_API_KEY")
            .expect("OPENAI_API_KEY environment variable not set"),
        max_tokens: None,
        system_prompt: None,
    }
}

fn get_anthropic_request_params(
    system_prompt: String,
    chat_history: &Vec<Message>,
) -> RequestParams {
    RequestParams {
        host: "api.anthropic.com".to_string(),
        path: "/v1/messages".to_string(),
        port: 443,
        messages: chat_history.iter().cloned().collect::<Vec<Message>>(),
        model: "claude-3-5-sonnet-20240620".to_string(),
        stream: true,
        authorization_token: env::var("ANTHROPIC_API_KEY")
            .expect("ANTHROPIC_API_KEY environment variable not set"),
        max_tokens: Some(8192),
        system_prompt: Some(system_prompt),
    }
}

fn process_openai_stream(
    stream: TlsStream<TcpStream>,
    tx: &std::sync::mpsc::Sender<String>,
) -> Result<String, std::io::Error> {
    let mut reader = std::io::BufReader::new(stream);
    let mut headers = String::new();
    while reader.read_line(&mut headers).unwrap() > 2 {
        if headers == "\r\n" {
            break;
        }

        headers.clear();
    }

    let mut full_message = String::new();
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
                    error!("JSON parse error: {}", e);
                    error!("Error payload: {}", payload);

                    serde_json::Value::Null
                }
            };

            let delta = response_json["choices"][0]["delta"]["content"]
                .to_string()
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
                .replace("\\'", "'")
                .replace("\\\\", "\\");

            // remove quotes
            let delta = delta[1..delta.len() - 1].to_string();

            if delta != "null" {
                tx.send(delta.clone()).expect("Failed to send OAI delta");
                full_message.push_str(&delta);
            }
        }

        event_buffer.clear();
    }

    Ok(full_message)
}

fn process_anthropic_stream(
    stream: TlsStream<TcpStream>,
    tx: &std::sync::mpsc::Sender<String>,
) -> Result<String, std::io::Error> {
    let mut reader = std::io::BufReader::new(stream);
    let mut all_headers = Vec::new();
    let mut headers = String::new();
    while reader.read_line(&mut headers).unwrap() > 2 {
        if headers == "\r\n" {
            break;
        }

        all_headers.push(headers.clone());
        headers.clear();
    }

    let mut full_message = all_headers.join("");
    let mut event_buffer = String::new();
    while reader.read_line(&mut event_buffer).unwrap() > 0 {
        if event_buffer.starts_with("event: message_stop") {
            break;
        } else if event_buffer.starts_with("data: ") {
            let payload = event_buffer[6..].trim();

            if payload.is_empty() || payload == "[DONE]" {
                break;
            }

            let response_json: serde_json::Value = serde_json::from_str(&payload)?;

            let mut delta = "null".to_string();
            if response_json["type"] == "content_block_delta" {
                delta = response_json["delta"]["text"]
                    .to_string()
                    .replace("\\n", "\n")
                    .replace("\\\"", "\"")
                    .replace("\\'", "'")
                    .replace("\\\\", "\\");

                // remove quotes
                delta = delta[1..delta.len() - 1].to_string();
            }

            if delta != "null" {
                tx.send(delta.clone())
                    .expect("Failed to send Anthropic delta");
                full_message.push_str(&delta);
            }
        }

        event_buffer.clear();
    }

    Ok(full_message)
}

pub fn prompt_stream(
    system_prompt: String,
    chat_history: &Vec<Message>,
    api: &String,
    tx: std::sync::mpsc::Sender<String>,
) -> Result<(), std::io::Error> {
    let params = match api.as_str() {
        "anthropic" => get_anthropic_request_params(system_prompt.clone(), chat_history),
        "openai" => get_openai_request_params(system_prompt.clone(), chat_history),
        _ => panic!("Invalid API: {}--how'd this get here?", api),
    };

    let stream = TcpStream::connect((params.host.clone(), params.port)).expect("Failed to connect");

    let connector = native_tls::TlsConnector::new().expect("Failed to create TLS connector");
    let mut stream = connector
        .connect(&params.host, stream)
        .expect("Failed to establish TLS connection");

    let request = build_request(&params);
    stream
        .write_all(request.as_bytes())
        .expect("Failed to write to stream");
    stream.flush().expect("Failed to flush stream");

    let response = match api.as_str() {
        "anthropic" => process_anthropic_stream(stream, &tx),
        "openai" => process_openai_stream(stream, &tx),
        _ => panic!("Invalid API: {}--how'd this get here?", api),
    };

    match response {
        Ok(full_message) => {
            info!("Response: {}", full_message);
        }
        Err(e) => {
            error!("Failed to process stream: {}", e);
            return Err(e);
        }
    }

    Ok(())
}

pub fn prompt(
    system_prompt: &String,
    chat_history: &Vec<Message>,
) -> Result<Message, std::io::Error> {
    let host = "api.openai.com";
    let path = "/v1/chat/completions";
    let port = 443;

    let messages = vec![Message::new(MessageType::System, system_prompt.clone())]
        .iter()
        .chain(chat_history.iter())
        .cloned()
        .collect::<Vec<Message>>();

    let body = serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": messages.iter().map(|message| {
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

    let stream = TcpStream::connect((host, port))?;

    let connector = native_tls::TlsConnector::new().expect("Failed to create TLS connector");
    let mut stream = connector
        .connect(host, stream)
        .expect("Failed to establish TLS connection");

    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("Failed to read from stream");

    let parts = response.split("\r\n\r\n").collect::<Vec<&str>>();
    let headers = parts[0];
    let response_body = parts[1];
    let mut remaining = response_body;
    let mut decoded_body = String::new();

    // they like to use this transfer encoding for long responses
    if headers.contains("Transfer-Encoding: chunked") {
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
    } else {
        decoded_body = response_body.to_string();
    }

    let response_json = serde_json::from_str(&decoded_body);

    if response_json.is_err() {
        error!("Failed to parse JSON: {}", decoded_body);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Failed to parse JSON",
        ));
    }

    let response_json: serde_json::Value = response_json.unwrap();
    let mut content = response_json["choices"][0]["message"]["content"].to_string();
    content = content
        .replace("\\n", "\n")
        .replace("\\\"", "\"")
        .replace("\\'", "'")
        .replace("\\\\", "\\");

    if content.starts_with("\"") && content.ends_with("\"") {
        content = content[1..content.len() - 1].to_string();
    }

    Ok(Message::new(MessageType::Assistant, content))
}
