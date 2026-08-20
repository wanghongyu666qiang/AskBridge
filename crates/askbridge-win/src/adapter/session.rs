use std::{path::Path, sync::atomic::AtomicBool};

use crate::browser::{CdpClient, CdpTarget};

/// Browser surface and lifetime-bounded resources available to a provider adapter.
pub enum PageSession<'a> {
    /// A managed Chrome page reachable through the persistent CDP client.
    DedicatedChrome {
        client: &'a CdpClient,
        target: &'a CdpTarget,
        temp_root: &'a Path,
        cancelled: &'a AtomicBool,
    },
    /// A desktop PWA that AskBridge may open but cannot inspect through CDP.
    DesktopPwa { target_url: &'a str },
}
