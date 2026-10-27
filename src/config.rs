//! Command-line flags.

pub const DEFAULT_URL: &str = "http://127.0.0.1:9931";

/// Which saved session to resume.
#[derive(Debug, Clone, PartialEq)]
pub enum Resume {
    /// `--continue`: the most recent session in this directory.
    Latest,
    /// `--resume STAMP`
    Stamp(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub url: String,
    pub model: Option<String>,
    pub wire_raw: bool,
    pub resume: Option<Resume>,
}

pub const USAGE: &str = "usage: bridle [--url URL] [--model ID] [--wire-raw] [-c | --resume STAMP]

  --url URL     llama-server base URL (default http://127.0.0.1:9931)
  --model ID    model id to request (default: the server's first model)
  --wire-raw    also log every raw SSE line to the wire log
  -c, --continue  resume the most recent session in this directory
  --resume STAMP  resume a specific session (see .bridle/sessions/)";

/// Parse flags (without the program name). Ok(None) means --help was asked for.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Config>, lexopt::Error> {
    use lexopt::prelude::*;
    let mut config = Config { url: DEFAULT_URL.to_string(), model: None, wire_raw: false, resume: None };
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Long("url") => config.url = parser.value()?.string()?,
            Long("model") => config.model = Some(parser.value()?.string()?),
            Long("wire-raw") => config.wire_raw = true,
            Short('c') | Long("continue") => config.resume = Some(Resume::Latest),
            Long("resume") => config.resume = Some(Resume::Stamp(parser.value()?.string()?)),
            Short('h') | Long("help") => return Ok(None),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Some(config))
}
