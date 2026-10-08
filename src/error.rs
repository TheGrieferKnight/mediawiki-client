use std::error::Error as _;
use thiserror::Error;

/// MediaWiki Client Error
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("Page title cannot be empty")]
    EmptyTitle,

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error("MediaWiki error [{code}]: {info}")]
    Api { code: String, info: String },

    #[error("MediaWiki login failed: {0}")]
    LoginFailed(String),

    #[error("Editing requires WIKI_BOT_USER and WIKI_BOT_PASS")]
    MissingCredentials,

    #[error("Missing MediaWiki {0} token")]
    MissingToken(&'static str),

    #[error("MediaWiki returned an unauthenticated CSRF token")]
    AnonymousCsrfToken,

    #[error("No CSRF token available")]
    NoCsrfToken,

    #[error("Read response did not contain page wikitext")]
    MissingWikitext,

    #[error("MediaWiki did not confirm a successful edit")]
    EditNotConfirmed,
}

impl Error {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::EmptyTitle => "empty_title",
            Self::Http(_) => "http",
            Self::Api { .. } => "api",
            Self::LoginFailed(_) => "login_failed",
            Self::MissingCredentials => "missing_credentials",
            Self::MissingToken(_) => "missing_token",
            Self::AnonymousCsrfToken => "anonymous_csrf_token",
            Self::NoCsrfToken => "no_csrf_token",
            Self::MissingWikitext => "missing_wikitext",
            Self::EditNotConfirmed => "edit_not_confirmed",
        }
    }

    /// Flattens the source chain into one string, if a source exists.
    pub fn cause(&self) -> Option<String> {
        let mut causes = Vec::new();
        let mut source = self.source();

        while let Some(cause) = source {
            causes.push(cause.to_string());
            source = cause.source();
        }

        (!causes.is_empty()).then(|| causes.join(": "))
    }
}

/// A result returned by the MediaWiki client.
pub type Result<T> = std::result::Result<T, Error>;
