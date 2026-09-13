//! Job and plan types handed to the browser worker: launch variants, launch
//! plans with their single allowed fallback, and stage/surface enums.

use std::time::{Duration, Instant};

use askbridge_core::{AppError, BrowserLifecycle, DispatchRequest, PreparationPolicy, Result};

/// A deliberately small launch policy. The only automatic fallback we expose
/// is managed Chrome to clipboard paste; arbitrary chains would make it
/// impossible to reason about duplicate insertion safely.
#[derive(Debug, Clone)]
pub struct BrowserLaunchPlan {
    primary: BrowserLaunch,
    fallback: Option<BrowserLaunch>,
}

impl BrowserLaunchPlan {
    pub const fn single(primary: BrowserLaunch) -> Self {
        Self {
            primary,
            fallback: None,
        }
    }

    pub const fn dedicated_then_clipboard(
        dedicated: DedicatedChromeJob,
        clipboard: ClipboardPasteJob,
    ) -> Self {
        Self {
            primary: BrowserLaunch::DedicatedChrome(dedicated),
            fallback: Some(BrowserLaunch::ClipboardPaste(clipboard)),
        }
    }

    pub(crate) const fn primary(&self) -> &BrowserLaunch {
        &self.primary
    }

    pub(super) const fn fallback(&self) -> Option<&BrowserLaunch> {
        self.fallback.as_ref()
    }

    pub const fn primary_surface(&self) -> BrowserSurface {
        launch_surface(&self.primary)
    }

    pub const fn fallback_surface(&self) -> Option<BrowserSurface> {
        match self.fallback.as_ref() {
            Some(launch) => Some(launch_surface(launch)),
            None => None,
        }
    }

    pub(super) const fn dedicated_lifecycle(&self) -> Option<BrowserLifecycle> {
        match &self.primary {
            BrowserLaunch::DedicatedChrome(job) => Some(job.lifecycle),
            BrowserLaunch::DesktopPwa(_) | BrowserLaunch::ClipboardPaste(_) => None,
        }
    }
}

const fn launch_surface(launch: &BrowserLaunch) -> BrowserSurface {
    match launch {
        BrowserLaunch::DedicatedChrome(_) => BrowserSurface::DedicatedChrome,
        BrowserLaunch::DesktopPwa(_) => BrowserSurface::DesktopPwa,
        BrowserLaunch::ClipboardPaste(_) => BrowserSurface::ClipboardPaste,
    }
}

#[derive(Debug, Clone)]
pub struct BrowserJob {
    pub request: DispatchRequest,
    pub policy: PreparationPolicy,
    pub adapter_override: Option<String>,
    pub plan: BrowserLaunchPlan,
}

#[derive(Debug, Clone)]
pub enum BrowserLaunch {
    DedicatedChrome(DedicatedChromeJob),
    DesktopPwa(DesktopPwaJob),
    ClipboardPaste(ClipboardPasteJob),
}

#[derive(Debug, Clone)]
pub struct DedicatedChromeJob {
    pub configured_chrome_path: Option<String>,
    pub profile_dir: String,
    pub connect_timeout: Duration,
    pub page_timeout: Duration,
    /// Optional end-to-end budget for this managed-browser attempt. This is
    /// used by the screenshot clipboard-fallback route so every CDP stage
    /// shares one short deadline instead of receiving a fresh timeout.
    pub attempt_timeout: Option<Duration>,
    pub lifecycle: BrowserLifecycle,
    pub start_url: String,
    pub url_patterns: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AttemptDeadline {
    deadline: Option<Instant>,
}

impl AttemptDeadline {
    pub(super) fn new(timeout: Option<Duration>) -> Result<Self> {
        let deadline = timeout
            .map(|timeout| {
                if timeout.is_zero() {
                    return Err(AppError::TargetTimeout);
                }
                Instant::now().checked_add(timeout).ok_or_else(|| {
                    AppError::InvalidPreparation(
                        "dedicated browser attempt timeout is too large".to_owned(),
                    )
                })
            })
            .transpose()?;
        Ok(Self { deadline })
    }

    pub(super) fn remaining(self, configured_timeout: Duration) -> Result<Duration> {
        let Some(deadline) = self.deadline else {
            return Ok(configured_timeout);
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(AppError::TargetTimeout)
        } else {
            Ok(remaining.min(configured_timeout))
        }
    }

    pub(super) fn preparation_policy(
        self,
        configured: &PreparationPolicy,
    ) -> Result<PreparationPolicy> {
        let Some(_) = self.deadline else {
            return Ok(configured.clone());
        };
        let remaining = self.remaining(Duration::from_millis(configured.timeout_ms))?;
        let timeout_ms = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX);
        if timeout_ms == 0 {
            return Err(AppError::TargetTimeout);
        }
        PreparationPolicy::new(timeout_ms)
    }
}

#[derive(Debug, Clone)]
pub struct DesktopPwaJob {
    pub provider_id: String,
    pub configured_shortcut: Option<String>,
    pub start_url: String,
    pub url_patterns: Vec<String>,
}

/// Inputs for the clipboard-paste target: screenshot to clipboard, focus a
/// matching provider page or supported desktop client (opening a page if
/// needed), and synthesize one Ctrl+V.
#[derive(Debug, Clone)]
pub struct ClipboardPasteJob {
    pub start_url: String,
    pub title_keywords: Vec<String>,
    /// How to cold-open the user-selected target when no matching window is
    /// already available. Delivery stays clipboard-based either way.
    pub open_target: ClipboardPasteOpenTarget,
    /// Total budget for locating the target window, including the one-time
    /// cold-open wait when no matching window exists yet.
    pub locate_timeout: Duration,
}

#[derive(Debug, Clone)]
pub enum ClipboardPasteOpenTarget {
    DefaultBrowser,
    DesktopPwa {
        provider_id: String,
        configured_shortcut: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct BrowserWarmupJob {
    pub configured_chrome_path: Option<String>,
    pub profile_dir: String,
    pub connect_timeout: Duration,
    pub page_timeout: Duration,
    pub open_url: Option<String>,
}

/// Managed-Chrome inputs for a batch of no-send provider capability checks.
#[derive(Debug, Clone)]
pub struct ProviderHealthJob {
    /// Optional configured Chrome executable path.
    pub configured_chrome_path: Option<String>,
    /// Managed Chrome profile directory name.
    pub profile_dir: String,
    /// Maximum time allowed to connect to Chrome DevTools.
    pub connect_timeout: Duration,
    /// Maximum time allowed for each provider page inspection.
    pub page_timeout: Duration,
    /// Providers to inspect without mutating their pages.
    pub providers: Vec<crate::adapter::ProviderHealthCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserStage {
    Started,
    Connected,
    TargetResolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserSurface {
    DedicatedChrome,
    DesktopPwa,
    ClipboardPaste,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedicated_attempt_deadline_caps_every_stage_and_policy() {
        let uncapped = AttemptDeadline::new(None).expect("uncapped deadline");
        assert_eq!(
            uncapped
                .remaining(Duration::from_secs(15))
                .expect("configured timeout"),
            Duration::from_secs(15)
        );

        let capped = AttemptDeadline::new(Some(Duration::from_secs(3))).expect("capped deadline");
        let stage_timeout = capped
            .remaining(Duration::from_secs(15))
            .expect("remaining stage timeout");
        assert!(stage_timeout > Duration::ZERO);
        assert!(stage_timeout <= Duration::from_secs(3));

        let configured = PreparationPolicy::new(15_000).expect("configured policy");
        let policy = capped
            .preparation_policy(&configured)
            .expect("remaining preparation policy");
        assert!((1..=3_000).contains(&policy.timeout_ms));
    }
}
