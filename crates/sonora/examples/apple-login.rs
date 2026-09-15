//! Opens the Apple Music sign-in window on its own, outside the app, and stores the account it
//! brings back where the provider looks for it.
//!
//! This is the same window and the same throwaway session the app puts up for a cookie
//! sign-in; it exists so an account can be imported without clicking through the UI first. No
//! browser profile is read, and nothing about the account is printed.
//!
//! ```sh
//! cargo run --package sonora --example apple-login
//! ```

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use music::{MusicProvider as _, SignIn};

/// How often the window is asked where it is.
const POLL: Duration = Duration::from_millis(250);

/// How long to leave the window up before giving up on it.
const PATIENCE: Duration = Duration::from_secs(600);

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    if !webview::supported() {
        bail!("this platform has no sign-in window backend");
    }
    let provider = music::apple::AppleProvider::new();
    let sign_in = provider
        .web_sign_in()
        .context("apple music describes no sign-in window")?;
    let target = webview::Target {
        url: sign_in.url.to_owned(),
        landing: sign_in.landing.to_owned(),
        domain: sign_in.domain.to_owned(),
        proof: sign_in.proof.iter().map(|it| (*it).to_owned()).collect(),
        title: "Sign in to Apple Music".to_owned(),
        agent: sign_in.agent.map(str::to_owned),
    };

    println!("opening the Apple Music sign-in window; close it to cancel");
    let mut login = webview::Login::open(target).context("cannot open the sign-in window")?;
    let opened = Instant::now();
    let cookies = loop {
        match login.poll() {
            webview::Poll::Cookies(header) => break header,
            webview::Poll::Closed => bail!("the window was closed before signing in"),
            webview::Poll::Pending => {}
        }
        if opened.elapsed() >= PATIENCE {
            bail!("the sign-in window timed out");
        }
        std::thread::sleep(POLL);
    };
    println!("signed in, checking the account with Apple");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot start a runtime")?;
    let (input, receiver) = tokio::sync::mpsc::unbounded_channel();
    input.send(cookies).ok();
    let session = runtime.block_on(provider.sign_in(
        SignIn::Secret,
        std::sync::Arc::new(|_| {}),
        receiver,
    ))?;
    println!(
        "stored the account for storefront {}; Sonora will pick it up on the next start",
        session.profile.id
    );
    Ok(())
}
