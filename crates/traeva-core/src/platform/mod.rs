//! Filesystem safety primitives. The first release supports macOS only
//! (docs/architecture.md §1), so everything lives in `macos.rs`.

pub(crate) mod macos;

#[cfg(test)]
mod tests;
