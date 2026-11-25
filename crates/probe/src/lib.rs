//! Checks against the real services, which need accounts and change what is in them, so every
//! one is ignored unless asked for with `cargo test -p probe -- --ignored`. The examples beside
//! it probe what the lyrics sources answer. Nothing in the app depends on this crate: it is the
//! one place that names every provider at once outside `sonora`.

#[cfg(test)]
mod live;
