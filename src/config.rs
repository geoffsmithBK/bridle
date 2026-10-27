//! Command-line flags.

pub const DEFAULT_URL: &str = "http://127.0.0.1:9931";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub url: String,
    pub model: Option<String>,
    pub wire_raw: bool,
}

pub const USAGE: &str = "usage: bridle [--url URL] [--model ID] [--wire-raw]

  --url URL     llama-server base URL (default http://127.0.0.1:9931)
  --model ID    model id to request (default: the server's first model)
  --wire-raw    also log every raw SSE line to the wire log";

/// Parse flags (without the program name). Ok(None) means --help was asked for.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Config>, lexopt::Error> {
    use lexopt::prelude::*;
    let mut config = Config { url: DEFAULT_URL.to_string(), model: None, wire_raw: false };
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Long("url") => config.url = parser.value()?.string()?,
            Long("model") => config.model = Some(parser.value()?.string()?),
            Long("wire-raw") => config.wire_raw = true,
            Short('h') | Long("help") => return Ok(None),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Some(config))
}
