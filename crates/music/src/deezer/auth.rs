use anyhow::{bail, Result};

/// Pulls an ARL out of a pasted cookie header, autolog URL, or the token itself.
pub fn arl(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("the ARL is empty");
    }
    if let Some(rest) = trimmed.strip_prefix("deezer://autolog/") {
        return take(rest);
    }
    for part in trimmed.split([';', '\n', '\t', ' ']) {
        let part = part.trim();
        if let Some(value) = part
            .strip_prefix("arl=")
            .or_else(|| part.strip_prefix("ARL="))
        {
            return take(value);
        }
    }
    take(trimmed)
}

fn take(value: &str) -> Result<String> {
    let value = value.trim().trim_matches('"');
    if value.len() < 32 {
        bail!("that does not look like a Deezer ARL");
    }
    Ok(value.to_string())
}
