pub mod remote;

use std::sync::Arc;

use gpui::App;
use state::{AppSettings, Registration};

/// Attaches the plugin manager with the built-in transports.
pub fn attach(cx: &mut App) {
    state::attach_plugins(
        cx,
        vec![Registration {
            plugin: Arc::new(remote::RemotePlugin),
            enabled: AppSettings::remote_api,
            port: AppSettings::remote_port,
            auth_required: AppSettings::remote_auth,
        }],
    );
}
