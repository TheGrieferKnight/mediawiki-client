use std::env;
use std::error::Error as _;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::Level;
use tracing_subscriber::EnvFilter;
use mediawiki_client::WikiClient;

const SERVICE: &str = "mediawiki-client";

const USAGE: &str = "Usage:
  mediawiki-client read <title>
  mediawiki-client edit <title> <summary> <content>
  mediawiki-client edit-file <title> <summary> <path|->
  mediawiki-client convert-runes <summary>";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Wiki(#[from] mediawiki_client::Error),

    #[error("Could not read '{path}'")]
    Input { path: String, source: io::Error },

    #[error("Could not write to stdout")]
    Output(#[source] io::Error),
}

impl CliError {
    fn kind(&self) -> &'static str {
        match self {
            Self::Wiki(error) => error.kind(),
            Self::Input { .. } => "input",
            Self::Output(_) => "output",
        }
    }

    /// Flattens the source chain into one string, if a source exists.
    fn cause(&self) -> Option<String> {
        if let Self::Wiki(error) = self {
            return error.cause();
        }

        let mut causes = Vec::new();
        let mut source = self.source();

        while let Some(cause) = source {
            causes.push(cause.to_string());
            source = cause.source();
        }

        (!causes.is_empty()).then(|| causes.join(": "))
    }
}

type CliResult<T> = Result<T, CliError>;

#[derive(Debug)]
enum Command {
    Read {
        title: String,
    },
    Edit {
        title: String,
        summary: String,
        content: String,
    },
    EditFile {
        title: String,
        summary: String,
        path: String,
    },
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Read { .. } => "read",
            Self::Edit { .. } => "edit",
            Self::EditFile { .. } => "edit-file",
        }
    }
}

fn parse_args(args: &[String]) -> Option<Command> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match args.as_slice() {
        ["read", title] => Some(Command::Read {
            title: (*title).to_owned(),
        }),
        ["edit", title, summary, content] => Some(Command::Edit {
            title: (*title).to_owned(),
            summary: (*summary).to_owned(),
            content: (*content).to_owned(),
        }),
        ["edit-file", title, summary, path] => Some(Command::EditFile {
            title: (*title).to_owned(),
            summary: (*summary).to_owned(),
            path: (*path).to_owned(),
        }),
        _ => None,
    }
}

fn read_input(path: &str) -> CliResult<String> {
    let result = if path == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).map(|_| buf)
    } else {
        std::fs::read_to_string(path)
    };

    result.map_err(|source| CliError::Input {
        path: path.to_owned(),
        source,
    })
}

/// Context for one command run, built up as it executes and emitted once. Only used for logging.
struct RunEvent {
    command: &'static str,
    api_url: Option<String>,
    bot_user: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    content_bytes: Option<u64>,
    pages_total: Option<u64>,
    pages_edited: u64,
    phase: &'static str,
    current_page: Option<String>,
    backup_path: Option<String>,
    backup_bytes: Option<u64>,
    api_requests: u32,
    logins: u32,
    token_retries: u32,
}

impl RunEvent {
    fn new(command: &'static str) -> Self {
        Self {
            command,
            api_url: None,
            bot_user: None,
            title: None,
            summary: None,
            content_bytes: None,
            pages_total: None,
            pages_edited: 0,
            phase: "setup",
            current_page: None,
            backup_path: None,
            backup_bytes: None,
            api_requests: 0,
            logins: 0,
            token_retries: 0,
        }
    }

    fn emit(&self, elapsed: Duration, error: Option<&CliError>) {
        let ev = self;
        let duration_ms = elapsed.as_millis() as u64;
        let outcome = if error.is_some() { "error" } else { "success" };
        let error_kind = error.map(CliError::kind);
        let error_message = error.map(ToString::to_string);
        let error_cause = error.and_then(CliError::cause);

        macro_rules! wide {
            ($level:expr) => {
                tracing::event!(
                    $level,
                    service = SERVICE,
                    version = env!("CARGO_PKG_VERSION"),
                    command = ev.command,
                    api_url = ev.api_url.as_deref(),
                    bot_user = ev.bot_user.as_deref(),
                    title = ev.title.as_deref(),
                    summary = ev.summary.as_deref(),
                    content_bytes = ev.content_bytes,
                    pages_total = ev.pages_total,
                    pages_edited = ev.pages_edited,
                    phase = ev.phase,
                    current_page = ev.current_page.as_deref(),
                    backup_path = ev.backup_path.as_deref(),
                    backup_bytes = ev.backup_bytes,
                    api_requests = ev.api_requests,
                    logins = ev.logins,
                    token_retries = ev.token_retries,
                    duration_ms = duration_ms,
                    outcome = outcome,
                    error.kind = error_kind,
                    error.message = error_message.as_deref(),
                    error.cause = error_cause.as_deref(),
                    "command finished"
                )
            };
        }

        if error.is_some() {
            wide!(Level::ERROR);
        } else {
            wide!(Level::INFO);
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}

async fn execute(command: Command, wiki: &mut WikiClient, event: &mut RunEvent) -> CliResult<()> {
    match command {
        Command::Read { title } => {
            event.title = Some(title.clone());
            event.phase = "read";

            let text = wiki.read_page(&title).await?;
            event.content_bytes = Some(text.len() as u64);

            writeln!(io::stdout().lock(), "{text}").map_err(CliError::Output)?;
        }
        Command::Edit {
            title,
            summary,
            content,
        } => edit_one(wiki, event, title, summary, content).await?,
        Command::EditFile {
            title,
            summary,
            path,
        } => {
            let content = read_input(&path)?;
            edit_one(wiki, event, title, summary, content).await?;
        }
    }

    event.phase = "done";
    Ok(())
}

async fn edit_one(
    wiki: &mut WikiClient,
    event: &mut RunEvent,
    title: String,
    summary: String,
    content: String,
) -> CliResult<()> {
    event.title = Some(title.clone());
    event.summary = Some(summary.clone());
    event.content_bytes = Some(content.len() as u64);
    event.phase = "edit";

    wiki.edit_page(&title, &content, &summary).await?;
    event.pages_edited += 1;
    Ok(())
}

async fn run(command: Command, event: &mut RunEvent) -> CliResult<()> {
    let mut wiki = WikiClient::from_env()?;

    event.api_url = Some(wiki.api_url().to_owned());
    event.bot_user = Some(wiki.bot_user().to_owned()).filter(|u| !u.is_empty());

    let result = execute(command, &mut wiki, event).await;

    let stats = wiki.stats();
    event.api_requests = stats.api_requests;
    event.logins = stats.logins;
    event.token_retries = stats.token_retries;

    result
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let args: Vec<String> = env::args().skip(1).collect();

    let Some(command) = parse_args(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };

    let started = Instant::now();
    let mut event = RunEvent::new(command.name());
    let result = run(command, &mut event).await;

    event.emit(started.elapsed(), result.as_ref().err());

    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
