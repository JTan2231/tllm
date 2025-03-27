# tllm - Terminal LLM

A command-line interface for interacting with Large Language Models (LLMs) like Anthropic's Claude and OpenAI's GPT. `tllm` allows you to have conversations directly from your terminal, manage conversation history, and customize your interactions.

## Features

* **Simple Command-Line Interface:** Easy to use with intuitive commands and options.
* **Provider Selection:** Choose between Anthropic and OpenAI as your LLM provider. Defaults to OpenAI's `GPT4o`.
* **Conversation History:** Saves and manages your conversations in a local SQLite database.
* **Load Previous Conversations:** List and load past conversations to continue where you left off.
* **System Prompts:** Define system-level instructions to guide the LLM's responses.
* **Editor Integration:** Compose messages and review conversations using your preferred system editor (e.g., Vim, Nano).
* **Configuration File:** Customize default settings through a configuration file.
* **File Input:** Provide the content of a file as your message to the LLM.
* **Continue Conversations:** Easily continue the last conversation or a loaded one.
* **Read-Only Conversation View:** Open past conversations in your editor for review without modification.

## Installation

This doesn't require anything special beyond a `cargo build --release`. The binary should then be good to use.

## Configuration

`tllm` can be configured via a configuration file located at `~/.local/tllm/config/config`. If the directory or file does not exist, `tllm` will create them on first run.

The configuration file uses a simple `key=value` format, with `#` for comments. The following options can be configured:

* `provider`: The default LLM provider to use (`anthropic` or `openai`). Defaults to `openai`.
* `list`: Default behavior for listing conversations (`true` or `false`). Defaults to `false`.
* `load_last_conversation`: Default behavior for loading the last conversation (`true` or `false`). Defaults to `false`.
* `editor`: Default behavior for using the system editor for messages (`true` or `false`). Defaults to `false`.
* `system_prompt`: Default path to a file containing the system prompt.

**Example `~/.local/tllm/config/config`:**

```ini
# Set the default provider to anthropic
provider=anthropic

# Always use the editor by default
editor=true

# Specify a default system prompt file
system_prompt=~/.config/tllm/default_system_prompt.txt
```

**Note:** Command-line arguments will always override the settings in the configuration file.

## Usage

```
tllm [OPTIONS] [MESSAGE]
```

### Arguments

* `[MESSAGE]`: The message to send to the LLM. If this is a path to a file, the content of the file will be used as the message.

### Options

* `-s, --system-prompt <PATH>`: Path to a file containing your system prompt. This will be ignored if the path is invalid.
* `-l, --list`: List saved conversations. This will open your editor with a list of conversations and allow you to select one to load using the `load` operation.
* `-L, --load-last-conversation`: Load the last updated conversation.
* `-e`: Send a message using the system editor. This will open your editor, optionally pre-filled with the history of the current conversation.
* `-p, --provider <PROVIDER>`: Choose which LLM provider to use (`anthropic` or `openai`).
* `-o, --open`: Open the current conversation in the system editor in read-only mode. If no conversation is active, it will prompt you to select one.
* `-r, --respond`: Open the system editor for writing after the last response. Useful for continuing a conversation without having to reissue commands. This only works if the `-e` option is also used.
* `-h, --help`: Print help information.
* `-V, --version`: Print version information.

### Examples

1.  **Send a simple message:**
    ```bash
    tllm "What is the capital of France?"
    ```

2.  **Send a message with a system prompt:**
    ```bash
    tllm -s system_prompt.txt "Explain this concept as if I were a five-year-old."
    ```
    where `system_prompt.txt` contains:
    ```
    You are a helpful assistant that explains complex topics in simple terms.
    ```

3.  **List and load a conversation:**
    ```bash
    tllm -l
    ```
    This will open your editor. Modify the lines to change `nothing` to `load` for the conversation you want to continue, then save and close the editor.

4.  **Load the last conversation:**
    ```bash
    tllm -L
    ```

5.  **Send a message using the editor:**
    ```bash
    tllm -e
    ```
    This will open your editor. Type your message, save, and close the editor to send it.

6.  **Specify the provider:**
    ```bash
    tllm -p anthropic "How does Claude compare to other LLMs?"
    ```

7.  **Open the current conversation in read-only mode:**
    ```bash
    tllm -o
    ```

8.  **Start a conversation with the editor and continue responding:**
    ```bash
    tllm -e -r "Hello, let's start a new conversation."
    ```
    After the initial response, your editor will reopen for your next message. This will continue until you submit an empty message.

9.  **Send the content of a file as a message:**
    ```bash
    tllm path/to/my/document.txt
    ```

## Important Notes

* **API Keys:** This README does not cover the handling of API keys for Anthropic and OpenAI. You will likely need to configure these separately, possibly through environment variables or another configuration mechanism not shown in this code snippet. Refer to the documentation of the `wire` crate used in the code for details on API key management.
* **Error Handling:** The provided code includes a `TODO` comment about improving error handling, particularly around `unwrap()` calls. In a production-ready application, these should be handled more gracefully.
* **Logging:** The application sets up basic logging to a file in `~/.local/tllm/logs/`.
* **Database Location:** Conversation history is stored in an SQLite database at `~/.local/tllm/tllm.sqlite`.

## License

MIT
