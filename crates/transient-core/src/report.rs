//! Error messages with their causes, for the log.

use std::error::Error;
use std::fmt;

/// Shows an error followed by each of its causes, separated by colons.
///
/// Transient's error types keep the error that caused them as their *source*
/// (`#[source]` or `#[from]`) instead of copying its text into their own
/// message, so the cause can still be inspected. `Report` puts the whole chain
/// on one line for the log:
///
/// ```
/// # use transient_core::report::Report;
/// let error = std::io::Error::other("disk full");
/// assert_eq!(Report(&error).to_string(), "disk full");
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Report<'a>(pub &'a (dyn Error + 'static));

impl fmt::Display for Report<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)?;
        let mut cause = self.0.source();
        while let Some(error) = cause {
            write!(formatter, ": {error}")?;
            cause = error.source();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("could not open the project")]
    struct Outer(#[source] Middle);

    #[derive(Debug, thiserror::Error)]
    #[error("could not read the file")]
    struct Middle(#[source] std::io::Error);

    #[test]
    fn every_cause_is_shown_once_in_order() {
        let error = Outer(Middle(std::io::Error::other("disk full")));
        assert_eq!(
            Report(&error).to_string(),
            "could not open the project: could not read the file: disk full"
        );
    }
}
