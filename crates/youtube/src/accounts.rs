use std::collections::HashSet;

use music::AccountChoice;
use ytmusic::{Identity, YtMusic};

const LIMIT: usize = 8;

pub struct Account {
    pub index: usize,
    pub identity: Identity,
}

impl Account {
    pub fn id(&self) -> String {
        match &self.identity.page_id {
            Some(page) => format!("{}:{page}", self.index),
            None => self.index.to_string(),
        }
    }

    pub fn choice(&self) -> AccountChoice {
        AccountChoice {
            id: self.id(),
            name: self.identity.profile.name.clone(),
            detail: self.identity.profile.email.clone(),
        }
    }
}

pub async fn list(cookies: &str) -> Vec<Account> {
    let mut found: Vec<Account> = Vec::new();
    let mut seen = HashSet::new();
    for index in 0..LIMIT {
        let identities = match YtMusic::with_cookies(cookies)
            .as_user(index)
            .identities()
            .await
        {
            Ok(identities) => identities,
            Err(error) => {
                log::debug!("youtube: authuser {index} names no account: {error:#}");
                break;
            }
        };
        let known = found.len();
        for identity in identities {
            if seen.insert(identity.key()) {
                found.push(Account { index, identity });
            }
        }
        if found.len() == known {
            break;
        }
    }
    found
}
