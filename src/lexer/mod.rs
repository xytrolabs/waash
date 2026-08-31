//! Lexer (tokenizer) for WAASH shell input.
//!
//! Breaks raw input strings into a stream of tokens that the parser consumes.
//! Handles: words, strings (single/double quoted), operators, heredoc markers,
//! variable references, command substitutions, process substitutions, etc.

pub mod token;
pub mod scanner;

pub use scanner::Scanner;
