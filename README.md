# TLLM

A terminal interface for interacting with large language models.

## Examples

### Basic Usage

```
tllm -a anthropic "What is the meaning of life?"
```

This will prompt the Anthropic API with the question "What is the meaning of life?" and display the response in your terminal.

### Loading a Conversation

```
tllm -l ./conversation.json
```

This will load the conversation from the file `./conversation.json` and open it in the terminal interface.

### Using a System Prompt

```
tllm -s "You are a helpful and informative AI assistant." "What is the capital of France?"
```

This will use the specified system prompt and then ask the question "What is the capital of France?".

## Installation

Ensure you have Rust installed. You can download and install Rust from the official website: [https://www.rust-lang.org/](https://www.rust-lang.org/).

   ```bash
   git clone https://github.com/jtan2231/tllm.git
   cd tllm
   cargo build
   cargo install --path .
   ```

## Usage

1. Set your API key for the desired language model as an environment variable:
   * Anthropic: `ANTHROPIC_API_KEY`
   * OpenAI: `OPENAI_API_KEY`
   * Gemini: `GEMINI_API_KEY`

2. Run the executable:

   ```bash
   # the -a flag defaults to anthropic
   tllm -a [gemini|anthropic|openai]
   ```

3. The terminal will display the interface, allowing you to interact with the language model.

## Features

* **Multiple API support:** Interact with Anthropic, OpenAI, and Gemini language models.
* **Conversation history:** Load and save conversations for future reference.
* **System prompt:** Set a system prompt to guide the language model's responses.
* **Streaming support:** Receive responses in real-time for a more interactive experience.
* **Directory view:** Search and browse files using [Dewey](https://github.com/JTan2231/dewey).
* **Key bindings:** Use tab to switch between chat and directory view.
* **Text editing:** Use arrow keys, backspace, and Ctrl+W/Ctrl+V for basic editing.
