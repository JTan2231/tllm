# TLLM

A terminal client for chatting with LLM APIs. Very sparse on features, currently tinkering on this to fit my needs. Will probably standardize at some point and make this more presentable/user-friendly.

Eventually hope for this to be a complete general terminal replacement for the web interfaces, as opposed to my own idiosyncratic tool.

## Install

TLLM currently only supports OpenAI--make sure your `OPENAI_API_KEY` environment variable is set.

To install,
```bash
git clone https://github.com/jtan2231/tllm.git && cd tllm
cargo build --release
sudo cp target/release/tllm /usr/bin
```
Then just use `tllm` to open a chat.

## Usage

The controls are vim-esque:
- Command Mode:
  - Cursor is a block and in the conversation display
  - `q` to exit
  - `a` to enter Edit Mode
  - `Enter` to send message
- Edit Mode:
  - Cursor is a line in input display
  - `ctrl + v` to paste
  - `esc` to enter Command Mode

 You can also use `ctrl + <left|right>` and `shift + <up|down>` to more quickly move about the text.

 ## TODO

 - Named conversation search
 - Embeddings (?)
 - Native text highlighting + yanking
 - CLI
 - Refactor `display.rs`--hideous code in there
