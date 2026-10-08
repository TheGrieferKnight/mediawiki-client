# mediawiki-client

An async MediaWiki API client and command-line tool for reading and editing pages.

## CLI

### Build

Project was built and developed with / on target cpu x86_64-unknown-linux-gnu using rustc 1.100.0-nightly.

```bash
cargo build --release
```

The binary is written to `target/release/mediawiki-client`.

### Configuration

Set API endpoint and bot credentials through environment variables:

```bash
export WIKI_API_URL="https://wiki.yourwiki.com/api.php"
export WIKI_BOT_USER="YourBotUsername"
export WIKI_BOT_PASS="YourBotPassword"
```

### Usage

```bash
mediawiki-client read "Page title"

mediawiki-client edit "Page title" "Edit summary" "New page content"

mediawiki-client edit-file "Page title" "Edit summary" ./page.txt

cat ./page.txt | mediawiki-client edit-file "Page title" "Edit summary" -
```

Edits replace the page’s entire wikitext.

Read output goes to stdout. JSON logs and usage errors go to stderr.
Configure logging with `RUST_LOG`:

```bash
RUST_LOG=info mediawiki-client read "Page title"
```

Commands exit with code `0` on success and `1` on failure or invalid arguments.

## Library

### Usage

For a local dependency without the CLI dependencies:

```toml
[dependencies]
mediawiki_client = {
    package = "mediawiki-client",
    path = "../mediawiki-sdk",
    default-features = false,
    features = ["rustls-tls"],
}
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use mediawiki_client::{Result, WikiClient};

#[tokio::main]
async fn main() -> Result<()> {
    let mut wiki = WikiClient::builder("https://example.org/w/api.php")
        .credentials("YourBotUsername", "YourBotPassword")
        .user_agent("my-wiki-tool/0.1.0")
        .build()?;

    let text = wiki.read_page("Example").await?;

    wiki.edit_page("Example", &text, "Update example")
        .await?;

    let stats = wiki.stats();
    assert!(stats.api_requests > 0);

    Ok(())
}
```

The library provides configurable timeouts, lazy login, and one retry on
MediaWiki’s `badtoken` error. Network failures are not retried because an edit
may already have been applied.

The library does not initialize logging or perform file I/O. The caller
supplies a Tokio-compatible runtime.

## Cargo features

- `cli` — enables the binary dependencies; enabled by default.
- `default-tls` — selects the package’s default Rustls configuration;
  enabled by default.
- `rustls-tls` — enables Reqwest’s Rustls backend.
