use anyhow::{Result, bail};

/// The cookies that tell a signed-in header from a guest one.
pub(crate) const PROOF: &[&str] = &["SAPISID", "__Secure-3PAPISID"];

/// Normalizes a `Cookie` header value to `name=value` pairs joined by `; ` and refuses one that
/// carries no proof of a signed-in account.
pub fn header(input: &str) -> Result<String> {
    let pairs: Vec<&str> = input
        .split(';')
        .map(str::trim)
        .filter(|pair| pair.contains('=') && !pair.contains(char::is_whitespace))
        .collect();
    let signed_in = pairs
        .iter()
        .filter_map(|pair| pair.split_once('='))
        .any(|(name, _)| PROOF.contains(&name));
    if !signed_in {
        bail!("the cookies carry no SAPISID or __Secure-3PAPISID");
    }
    Ok(pairs.join("; "))
}

#[cfg(test)]
mod tests {
    use super::header;

    #[test]
    fn keeps_a_signed_in_header() {
        let value = header("VISITOR_INFO1_LIVE=abc;  SAPISID=xyz; PREF=f1").unwrap();
        assert_eq!(value, "VISITOR_INFO1_LIVE=abc; SAPISID=xyz; PREF=f1");
    }

    #[test]
    fn accepts_the_secure_variant_alone() {
        assert!(header("__Secure-3PAPISID=xyz").is_ok());
    }

    #[test]
    fn rejects_a_signed_out_header() {
        assert!(header("VISITOR_INFO1_LIVE=abc; PREF=f1").is_err());
    }

    #[test]
    fn rejects_an_empty_header() {
        assert!(header("   ").is_err());
    }
}
