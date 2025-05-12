## Usage

```bash
tllm [OPTIONS] [MESSAGE]
```

### Arguments

**[MESSAGE]** (Optional): The message/prompt to send to the LLM. If the message looks like a valid file path, the tool will attempt to read the file's content and use that as the message.

### Options

- `-s, --system-prompt <FILE_PATH>`: Path to a file containing the system prompt. Ignored if the path is invalid.
- `-l, --list`: List saved conversations and enter an interactive editor mode to select one to load. Conflicts with `-L`.
- `-L, --load-last-conversation`: Load the most recently updated conversation. Conflicts with `-l`.
- `-e, --editor`: Open the system editor (`$EDITOR`) to compose the message. Shows conversation history if loading a conversation.
- `-p, --provider <PROVIDER>`: Choose the LLM provider. Valid options: `anthropic`, `openai`. Defaults to openai (specifically GPT-4) if not set via config or CLI.
- `-o, --open`: Open the current or a selected conversation in the system editor (read-only). If no conversation is active/loaded, prompts to select one.
- `-r, --respond`: After receiving a response when using the editor (`-e`), immediately reopen the editor to compose the next message in the same conversation. Does nothing if `-e` is not used.
- `-X, --export-all <FILE_PATH>`: Export the entire conversation history from the database to the specified file. All other flags are ignored if this is set.
- `-d, --database <DB_PATH>`: Use a specific SQLite database file instead of the default (`~/.local/tllm/tllm.sqlite` or `~/.local/tllm-dev/tllm.sqlite` for debug builds).
- `-x, --no-config`: Ignore the configuration file (`~/.config/tllm/config`).
- `-S, --stream`: Stream the LLM response to standard output as it arrives.
- `-h, --help`: Print help information.
- `-V, --version`: Print version information.

### Configuration

You can set default options by creating a configuration file at `~/.config/tllm/config`. The tool will create the necessary directories (`~/.local/tllm` and `~/.config/tllm`) on first run if they don't exist.

The configuration file uses a simple key=value format. Lines starting with `#` are ignored.

#### Supported Configuration Keys:

- `provider`: `anthropic` or `openai`
- `list`: `true` or `false`
- `load_last_conversation`: `true` or `false`
- `editor`: `true` or `false`
- `system_prompt`: Path to the system prompt file
- `open`: `true` or `false`
- `respond`: `true` or `false`
- `stream`: `true` or `false`

#### Example `~/.config/tllm/config`:

```ini
# Default LLM provider
provider=openai

# Always use the editor by default
editor=true

# Default system prompt file
system_prompt=/home/user/prompts/default_system.txt

# Automatically stream responses
stream=true
```

**Precedence**: Command-line arguments always override settings in the configuration file. The `--no-config` flag prevents the configuration file from being loaded at all.

### Database

Conversations are stored in a SQLite database located by default at:

- Release Build: `~/.local/tllm/tllm.sqlite`
- Debug Build: `~/.local/tllm-dev/tllm.sqlite`

You can specify a different database location using the `-d` or `--database` option.

### Logs

Log files are stored in:

- Release Build: `~/.local/tllm/logs/`
- Debug Build: `~/.local/tllm-dev/logs/`

Log file names are timestamps in microseconds since the UNIX epoch (e.g., `1678886400123456.log`) for release builds, or `debug.log` for debug builds.

### Examples

Send a simple message:
```bash
tllm "What is the capital of France?"
```

Send a message using the editor:
```bash
tllm -e
```
(Your `$EDITOR` will open. Type your message, save, and close.)

Start a new conversation with a system prompt and stream the response:
```bash
tllm -s ~/prompts/coder.txt -S "Write a python function for fibonacci"
```

List conversations and load one:
```bash
tllm -l
```
(Editor opens with a list. Change nothing to load for the desired conversation, save, and close.)

Load the last conversation and continue it using the editor, reopening the editor after each response:
```bash
tllm -L -e -r
```

Export all conversations:
```bash
tllm -X ~/tllm_backup.txt
```

Use the Anthropic provider with a custom database:
```bash
tllm -p anthropic -d /mnt/data/my_tllm.sqlite "Tell me about Claude Sonnet 3.5"
```

### Development Notes

- The code contains several `.unwrap()` calls that should ideally be replaced with more robust error handling (e.g., using `Result` and `?`).
- API key management relies on the external `wire` crate. Ensure it's configured correctly.
- The default configuration path (`~/.local/tllm/config/`) is noted as potentially incorrect in a code comment.
