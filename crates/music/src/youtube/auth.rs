use anyhow::{Result, bail};

const PROOF: &[&str] = &["SAPISID", "__Secure-3PAPISID"];

pub fn header(input: &str) -> Result<String> {
    if input.trim().is_empty() {
        bail!("cookie header is empty");
    }
    let raw = input
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("Cookie:")
                .or_else(|| line.strip_prefix("cookie:"))
                .map(str::trim)
        })
        .unwrap_or_else(|| input.trim());
    let pairs: Vec<&str> = raw
        .split(';')
        .map(str::trim)
        .filter(|pair| pair.contains('=') && !pair.contains(char::is_whitespace))
        .collect();
    let signed_in = pairs
        .iter()
        .filter_map(|pair| pair.split_once('='))
        .any(|(name, _)| PROOF.contains(&name));
    if !signed_in {
        bail!(
            "the pasted text carries no SAPISID or __Secure-3PAPISID; copy the whole value of the Cookie request header, not the request Cookies panel"
        );
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
    fn drops_the_header_name() {
        let value = header("Cookie: SAPISID=xyz; SID=def").unwrap();
        assert_eq!(value, "SAPISID=xyz; SID=def");
    }

    #[test]
    fn picks_the_cookie_line_out_of_a_blob() {
        let blob = "POST /youtubei/v1/browse HTTP/2\nHost: music.youtube.com\ncookie: SAPISID=abc; SID=def\nOrigin: https://music.youtube.com";
        assert_eq!(header(blob).unwrap(), "SAPISID=abc; SID=def");
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
    fn rejects_an_empty_paste() {
        assert!(header("   \n ").is_err());
    }

    #[test]
    fn rejects_a_bare_cookie_name() {
        assert!(header("SAPISID").is_err());
    }
}
