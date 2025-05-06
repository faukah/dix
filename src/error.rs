use thiserror::Error;

/// Application errors with thiserror
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Command failed: {command} {args:?} - {message}")]
    CommandFailed {
        command: String,
        args: Vec<String>,
        message: String,
    },

    #[error("Failed to decode command output from {context}: {source}")]
    CommandOutputError {
        source: std::str::Utf8Error,
        context: String,
    },

    #[error("Failed to parse data in {context}: {message}")]
    ParseError {
        message: String,
        context: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("Regex error in {context}: {source}")]
    RegexError {
        source: regex::Error,
        context: String,
    },

    #[error("IO error in {context}: {source}")]
    IoError {
        source: std::io::Error,
        context: String,
    },

    #[error("Database error: {source}")]
    DatabaseError { source: rusqlite::Error },
}

// Implement From traits to support the ? operator
impl From<std::io::Error> for AppError {
    fn from(source: std::io::Error) -> Self {
        Self::IoError {
            source,
            context: "unknown context".into(),
        }
    }
}

impl From<std::str::Utf8Error> for AppError {
    fn from(source: std::str::Utf8Error) -> Self {
        Self::CommandOutputError {
            source,
            context: "command output".into(),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(source: rusqlite::Error) -> Self {
        Self::DatabaseError { source }
    }
}

impl From<regex::Error> for AppError {
    fn from(source: regex::Error) -> Self {
        Self::RegexError {
            source,
            context: "regex operation".into(),
        }
    }
}

impl AppError {
    /// Create a command failure error with context
    pub fn command_failed<S: Into<String>>(command: S, args: &[&str], message: S) -> Self {
        Self::CommandFailed {
            command: command.into(),
            args: args.iter().map(|&s| s.to_string()).collect(),
            message: message.into(),
        }
    }

    /// Create a parse error with context
    pub fn parse_error<S: Into<String>, C: Into<String>>(
        message: S,
        context: C,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self::ParseError {
            message: message.into(),
            context: context.into(),
            source,
        }
    }

    /// Create an IO error with context
    pub fn io_error<C: Into<String>>(source: std::io::Error, context: C) -> Self {
        Self::IoError {
            source,
            context: context.into(),
        }
    }

    /// Create a regex error with context
    pub fn regex_error<C: Into<String>>(source: regex::Error, context: C) -> Self {
        Self::RegexError {
            source,
            context: context.into(),
        }
    }

    /// Create a command output error with context
    pub fn command_output_error<C: Into<String>>(source: std::str::Utf8Error, context: C) -> Self {
        Self::CommandOutputError {
            source,
            context: context.into(),
        }
    }
}
