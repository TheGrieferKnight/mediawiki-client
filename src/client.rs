use crate::{Error, Result};
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::env;
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const DEFAULT_API_URL: &str = "https://wiki.yourwiki.com/api.php";
const DEFAULT_USER_AGENT: &str = "mediawiki-client/0.1.0";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub api_requests: u32,

    pub logins: u32,

    pub token_retries: u32,
}

/// MediaWiki Client Builder
pub struct WikiClientBuilder {
    api_url: String,
    bot_user: String,
    bot_pass: String,
    user_agent: String,
    connect_timeout: Duration,
    timeout: Duration,
}

impl fmt::Debug for WikiClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WikiClientBuilder")
            .field("api_url", &self.api_url)
            .field("bot_user", &self.bot_user)
            .field("bot_pass", &"[REDACTED]")
            .field("user_agent", &self.user_agent)
            .field("connect_timeout", &self.connect_timeout)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl WikiClientBuilder {
    pub fn new(api_url: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
            bot_user: String::new(),
            bot_pass: String::new(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            connect_timeout: Duration::from_secs(10),
            timeout: Duration::from_secs(60),
        }
    }

    pub fn api_url(mut self, api_url: impl Into<String>) -> Self {
        self.api_url = api_url.into();
        self
    }

    pub fn credentials(mut self, user: impl Into<String>, pass: impl Into<String>) -> Self {
        self.bot_user = user.into();
        self.bot_pass = pass.into();
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn build(self) -> Result<WikiClient> {
        let client = Client::builder()
            .cookie_store(true)
            .connect_timeout(self.connect_timeout)
            .timeout(self.timeout)
            .user_agent(self.user_agent)
            .build()?;

        Ok(WikiClient {
            client,
            api_url: self.api_url,
            bot_user: self.bot_user,
            bot_pass: self.bot_pass,
            csrf_token: None,
            api_requests: AtomicU32::new(0),
            logins: 0,
            token_retries: 0,
        })
    }
}

/// Asynchronous MediaWiki API Client
pub struct WikiClient {
    client: Client,
    api_url: String,
    bot_user: String,
    bot_pass: String,
    csrf_token: Option<String>,
    api_requests: AtomicU32,
    logins: u32,
    token_retries: u32,
}

impl fmt::Debug for WikiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WikiClient")
            .field("api_url", &self.api_url)
            .field("bot_user", &self.bot_user)
            .field("bot_pass", &"[REDACTED]")
            .field(
                "csrf_token",
                &self.csrf_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("stats", &self.stats())
            .finish()
    }
}

impl WikiClient {
    pub fn new(
        api_url: impl Into<String>,
        bot_user: impl Into<String>,
        bot_pass: impl Into<String>,
    ) -> Result<Self> {
        Self::builder(api_url)
            .credentials(bot_user, bot_pass)
            .build()
    }

    pub fn builder(api_url: impl Into<String>) -> WikiClientBuilder {
        WikiClientBuilder::new(api_url)
    }

    pub fn from_env() -> Result<Self> {
        Self::new(
            env::var("WIKI_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_owned()),
            env::var("WIKI_BOT_USER").unwrap_or_default(),
            env::var("WIKI_BOT_PASS").unwrap_or_default(),
        )
    }

    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    pub fn bot_user(&self) -> &str {
        &self.bot_user
    }

    pub fn stats(&self) -> Stats {
        Stats {
            api_requests: self.api_requests.load(Ordering::Relaxed),
            logins: self.logins,
            token_retries: self.token_retries,
        }
    }

    pub async fn read_page(&self, title: &str) -> Result<String> {
        validate_title(title)?;

        let request = self.client.get(&self.api_url).query(&[
            ("action", "parse"),
            ("page", title),
            ("prop", "wikitext"),
            ("format", "json"),
            ("formatversion", "2"),
        ]);

        self.send(request)
            .await?
            .pointer("/parse/wikitext")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(Error::MissingWikitext)
    }

    /// Replaces a page's wikitext.
    /// Logs in lazily and retries once on `badtoken`. On Network failures does not retry, but changes may still have gone through.
    pub async fn edit_page(&mut self, title: &str, content: &str, summary: &str) -> Result<()> {
        self.write_page(title, content, summary, false).await
    }

    /// Creates a page only if it does not already exist. Existing pages return MediaWiki's 'articleexists' error.
    pub async fn create_page(&mut self, title: &str, content: &str, summary: &str) -> Result<()> {
        self.write_page(title, content, summary, true).await
    }

    async fn write_page(
        &mut self,
        title: &str,
        content: &str,
        summary: &str,
        create_only: bool,
    ) -> Result<()> {
        validate_title(title)?;

        if self.csrf_token.is_none() {
            self.login().await?;
        }

        let response = match self.submit_edit(title, content, summary, create_only).await {
            Err(Error::Api { code, .. }) if code == "badtoken" => {
                self.token_retries += 1;
                self.login().await?;
                self.submit_edit(title, content, summary, create_only)
                    .await?
            }
            other => other?,
        };

        match response.pointer("/edit/result").and_then(Value::as_str) {
            Some("Success") => Ok(()),
            _ => Err(Error::EditNotConfirmed),
        }
    }

    pub async fn login(&mut self) -> Result<()> {
        self.csrf_token = None;

        if self.bot_user.is_empty() || self.bot_pass.is_empty() {
            return Err(Error::MissingCredentials);
        }

        self.logins += 1;
        let login_token = self.fetch_token("login", "logintoken").await?;

        let request = self.client.post(&self.api_url).form(&[
            ("action", "login"),
            ("lgname", self.bot_user.as_str()),
            ("lgpassword", self.bot_pass.as_str()),
            ("lgtoken", login_token.as_str()),
            ("format", "json"),
        ]);

        let response = self.send(request).await?;
        let result = response
            .pointer("/login/result")
            .and_then(Value::as_str)
            .unwrap_or("Unknown");

        if result != "Success" {
            return Err(Error::LoginFailed(result.to_owned()));
        }

        let token = self.fetch_token("csrf", "csrftoken").await?;

        // "+\\" => MediaWiki's anonymous token.
        if token.is_empty() || token == "+\\" {
            return Err(Error::AnonymousCsrfToken);
        }

        self.csrf_token = Some(token);
        Ok(())
    }

    async fn send(&self, request: RequestBuilder) -> Result<Value> {
        self.api_requests.fetch_add(1, Ordering::Relaxed);

        let response: Value = request.send().await?.error_for_status()?.json().await?;

        check_api_error(&response)?;
        Ok(response)
    }

    async fn fetch_token(&self, kind: &'static str, key: &str) -> Result<String> {
        let request = self.client.get(&self.api_url).query(&[
            ("action", "query"),
            ("meta", "tokens"),
            ("type", kind),
            ("format", "json"),
        ]);

        self.send(request)
            .await?
            .pointer(&format!("/query/tokens/{key}"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(Error::MissingToken(kind))
    }

    async fn submit_edit(
        &self,
        title: &str,
        content: &str,
        summary: &str,
        create_only: bool,
    ) -> Result<Value> {
        let token = self.csrf_token.as_deref().ok_or(Error::NoCsrfToken)?;

        let mut fields = vec![
            ("action", "edit"),
            ("title", title),
            ("text", content),
            ("summary", summary),
            ("token", token),
            ("assert", "user"),
            ("format", "json"),
        ];
        if create_only {
            fields.push(("createonly", "1"));
        }
        let request = self.client.post(&self.api_url).form(&fields);

        self.send(request).await
    }
}

fn check_api_error(response: &Value) -> Result<()> {
    let Some(error) = response.get("error") else {
        return Ok(());
    };

    let field = |name: &str, default: &str| {
        error
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_owned()
    };

    Err(Error::Api {
        code: field("code", "unknown"),
        info: field("info", "No additional details"),
    })
}

fn validate_title(title: &str) -> Result<()> {
    if title.trim().is_empty() {
        return Err(Error::EmptyTitle);
    }
    Ok(())
}
