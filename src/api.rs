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

#[derive(Clone, Debug)]
struct RequestParams {
    provider: String,
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
    let body = match params.provider.as_str() {
        "openai" => serde_json::json!({
            "model": params.model,
            "messages": params.messages.iter()
                .map(|message| {
                    serde_json::json!({
                        "role": message.message_type.to_string(),
                        "content": message.content
                    })
                }).collect::<Vec<serde_json::Value>>(),
            "stream": params.stream,
        }),
        "anthropic" => serde_json::json!({
            "model": params.model,
            "messages": params.messages.iter().map(|message| {
                serde_json::json!({
                    "role": message.message_type.to_string(),
                    "content": message.content
                })
            }).collect::<Vec<serde_json::Value>>(),
            "stream": params.stream,
            "max_tokens": params.max_tokens.unwrap(),
            "system": params.system_prompt.clone().unwrap(),
        }),
        _ => panic!("Invalid provider for request_body: {}", params.provider),
    };

    let json = serde_json::json!(body);
    let json_string = serde_json::to_string(&json).expect("Failed to serialize JSON");

    let auth_string = match params.provider.as_str() {
        "openai" => "Authorization: Bearer ".to_string() + &params.authorization_token,
        "anthropic" => "x-api-key: ".to_string() + &params.authorization_token,
        _ => panic!("Invalid provider for auth_string: {}", params.provider),
    };

    let api_version = match params.provider.as_str() {
        "openai" => "\r\n",
        "anthropic" => "anthropic-version: 2023-06-01\r\n\r\n",
        _ => panic!("Invalid provider for api_version: {}", params.provider),
    };

    format!(
        "POST {} HTTP/1.1\r\n\
        Host: {}\r\n\
        Content-Type: application/json\r\n\
        Content-Length: {}\r\n\
        Accept: */*\r\n\
        {}\r\n\
        {}\
        {}",
        params.path,
        params.host,
        json_string.len(),
        auth_string,
        api_version,
        json_string.trim()
    )
}

fn get_openai_request_params(
    system_prompt: String,
    chat_history: &Vec<Message>,
    stream: bool,
) -> RequestParams {
    RequestParams {
        provider: "openai".to_string(),
        host: "api.openai.com".to_string(),
        path: "/v1/chat/completions".to_string(),
        port: 443,
        messages: vec![Message::new(MessageType::System, system_prompt.clone())]
            .iter()
            .chain(chat_history.iter())
            .cloned()
            .collect::<Vec<Message>>(),
        model: "gpt-4o-mini".to_string(),
        stream,
        authorization_token: env::var("OPENAI_API_KEY")
            .expect("OPENAI_API_KEY environment variable not set"),
        max_tokens: None,
        system_prompt: None,
    }
}

fn get_anthropic_request_params(
    system_prompt: String,
    chat_history: &Vec<Message>,
    stream: bool,
) -> RequestParams {
    RequestParams {
        provider: "anthropic".to_string(),
        host: "api.anthropic.com".to_string(),
        path: "/v1/messages".to_string(),
        port: 443,
        messages: chat_history.iter().cloned().collect::<Vec<Message>>(),
        model: "claude-3-5-sonnet-20240620".to_string(),
        stream,
        authorization_token: env::var("ANTHROPIC_API_KEY")
            .expect("ANTHROPIC_API_KEY environment variable not set"),
        max_tokens: Some(4096),
        system_prompt: Some(system_prompt),
    }
}

fn process_openai_stream(
    stream: TlsStream<TcpStream>,
    tx: &std::sync::mpsc::Sender<String>,
) -> Result<String, std::io::Error> {
    info!("processing openai stream");
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
    info!("processing anthropic stream");
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
        "anthropic" => get_anthropic_request_params(system_prompt.clone(), chat_history, true),
        "openai" => get_openai_request_params(system_prompt.clone(), chat_history, true),
        _ => panic!("Invalid API: {}--how'd this get here?", api),
    };

    let stream = TcpStream::connect((params.host.clone(), params.port)).expect("Failed to connect");

    let connector = native_tls::TlsConnector::new().expect("Failed to create TLS connector");
    let mut stream = connector
        .connect(&params.host, stream)
        .expect("Failed to establish TLS connection");

    let request = build_request(&params);
    info!("sending request {}", request);
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
        Ok(_) => {}
        Err(e) => {
            error!("Failed to process stream: {}", e);
            return Err(e);
        }
    }

    Ok(())
}

pub fn prompt(
    api: &String,
    system_prompt: &String,
    chat_history: &Vec<Message>,
) -> Result<Message, std::io::Error> {
    let params = match api.as_str() {
        "anthropic" => get_anthropic_request_params(system_prompt.clone(), chat_history, false),
        "openai" => get_openai_request_params(system_prompt.clone(), chat_history, false),
        _ => panic!("Invalid API: {}--how'd this get here?", api),
    };

    info!("Using params: {:?}", params);

    info!("Connecting to {}:{}", params.host, params.port);

    let stream = TcpStream::connect((params.host.clone(), params.port))?;

    let connector = native_tls::TlsConnector::new().expect("Failed to create TLS connector");
    let mut stream = connector
        .connect(&params.host, stream)
        .expect("Failed to establish TLS connection");

    info!("TLS connection established");

    let request = build_request(&params);
    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    info!("Request sent: {}", request);

    let mut reader = std::io::BufReader::new(stream);
    let mut content_length = 0;
    let mut headers = Vec::new();
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap() > 0 {
        info!("Header: {}", line);
        if line == "\r\n" {
            info!("End of headers");
            break;
        }

        if line.contains("Content-Length") {
            let parts: Vec<&str> = line.split(":").collect();
            content_length = parts[1].trim().parse().unwrap();
        }

        headers.push(line.clone());
        line.clear();
    }

    info!("Headers: {:?}", headers);

    let mut response_body = String::new();
    reader
        .take(content_length as u64)
        .read_to_string(&mut response_body)?;

    info!("Response body: {}", response_body);

    let mut remaining = response_body.as_str();
    let mut decoded_body = String::new();

    // they like to use this transfer encoding for long responses
    if headers.contains(&"Transfer-Encoding: chunked".to_string()) {
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

    let mut content = match api.as_str() {
        "openai" => response_json["choices"][0]["message"]["content"].to_string(),
        "anthropic" => response_json["content"][0]["text"].to_string(),
        _ => String::new(),
    };

    content = content
        .replace("\\\"", "\"")
        .replace("\\'", "'")
        .replace("\\\\", "\\");

    if content.starts_with("\"") && content.ends_with("\"") {
        content = content[1..content.len() - 1].to_string();
    }

    Ok(Message::new(MessageType::Assistant, content))
}
