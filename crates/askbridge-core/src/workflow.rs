use crate::{AppCommand, AppError, AppState, DispatchOutcome, Result};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowController {
    state: AppState,
}

impl WorkflowController {
    pub const fn state(self) -> AppState {
        self.state
    }

    pub const fn is_idle(self) -> bool {
        matches!(self.state, AppState::Idle)
    }

    pub fn start(&mut self, command: AppCommand) -> Result<AppState> {
        if !self.is_idle() {
            return Err(AppError::WorkflowBusy(format!("{:?}", self.state)));
        }
        self.state = match command {
            AppCommand::CaptureWithPrompt | AppCommand::CaptureQuickDispatch => {
                AppState::SelectingRegion
            }
            AppCommand::TextOnlyPrompt => AppState::PreparingDispatch,
        };
        Ok(self.state)
    }

    pub fn capture_completed(&mut self, command: AppCommand) -> Result<AppState> {
        self.require_state(AppState::SelectingRegion, "capture_completed")?;
        self.state = match command {
            AppCommand::CaptureWithPrompt => AppState::PreparingDispatch,
            AppCommand::CaptureQuickDispatch => AppState::PreparingDispatch,
            AppCommand::TextOnlyPrompt => {
                return Err(self.invalid_transition("capture_completed(text_only_prompt)"));
            }
        };
        Ok(self.state)
    }

    pub fn prompt_submitted(&mut self) -> Result<AppState> {
        self.require_state(AppState::Prompting, "prompt_submitted")?;
        self.state = AppState::PreparingDispatch;
        Ok(self.state)
    }

    pub fn begin_browser(&mut self) -> Result<AppState> {
        self.require_state(AppState::PreparingDispatch, "begin_browser")?;
        self.state = AppState::StartingBrowser;
        Ok(self.state)
    }

    pub fn browser_started(&mut self) -> Result<AppState> {
        self.require_state(AppState::StartingBrowser, "browser_started")?;
        self.state = AppState::ConnectingBrowser;
        Ok(self.state)
    }

    pub fn browser_connected(&mut self) -> Result<AppState> {
        self.require_state(AppState::ConnectingBrowser, "browser_connected")?;
        self.state = AppState::ResolvingTarget;
        Ok(self.state)
    }

    pub fn target_resolved(&mut self) -> Result<AppState> {
        self.require_state(AppState::ResolvingTarget, "target_resolved")?;
        self.state = AppState::WaitingForPage;
        Ok(self.state)
    }

    pub fn page_ready(&mut self) -> Result<AppState> {
        self.require_state(AppState::WaitingForPage, "page_ready")?;
        self.state = AppState::PreparingPage;
        Ok(self.state)
    }

    pub fn desktop_surface_ready(&mut self) -> Result<AppState> {
        self.require_state(AppState::StartingBrowser, "desktop_surface_ready")?;
        self.state = AppState::PreparingPage;
        Ok(self.state)
    }

    pub fn defer_page_preparation(&mut self) -> Result<AppState> {
        self.require_state(AppState::PreparingPage, "defer_page_preparation")?;
        self.state = AppState::Idle;
        Ok(self.state)
    }

    pub fn page_prepared(&mut self, outcome: &DispatchOutcome) -> Result<AppState> {
        self.require_state(AppState::PreparingPage, "page_prepared")?;
        self.state = match outcome {
            DispatchOutcome::PreparedForUser(_) => AppState::PreparedForUser,
            DispatchOutcome::ManualFallbackReady(_) => AppState::PreparingFallback,
            DispatchOutcome::Cancelled => AppState::Cancelling,
        };
        Ok(self.state)
    }

    pub fn fallback_prepared(&mut self) -> Result<AppState> {
        self.require_state(AppState::PreparingFallback, "fallback_prepared")?;
        self.state = AppState::FallbackReady;
        Ok(self.state)
    }

    pub fn prepare_fallback_after_browser_failure(&mut self) -> Result<AppState> {
        if !matches!(
            self.state,
            AppState::PreparingDispatch
                | AppState::StartingBrowser
                | AppState::ConnectingBrowser
                | AppState::ResolvingTarget
                | AppState::WaitingForPage
                | AppState::PreparingPage
        ) {
            return Err(self.invalid_transition("prepare_fallback_after_browser_failure"));
        }
        self.state = AppState::PreparingFallback;
        Ok(self.state)
    }

    pub fn finish_delivery(&mut self) -> Result<AppState> {
        if !matches!(
            self.state,
            AppState::PreparedForUser | AppState::FallbackReady
        ) {
            return Err(self.invalid_transition("finish_delivery"));
        }
        self.state = AppState::Idle;
        Ok(self.state)
    }

    pub fn retry_fallback(&mut self) -> Result<AppState> {
        self.require_state(AppState::FallbackReady, "retry_fallback")?;
        self.state = AppState::PreparingDispatch;
        Ok(self.state)
    }

    pub fn begin_cancelling(&mut self) -> Result<AppState> {
        if matches!(self.state, AppState::Idle | AppState::Cancelling) {
            return Err(self.invalid_transition("begin_cancelling"));
        }
        self.state = AppState::Cancelling;
        Ok(self.state)
    }

    pub fn finish_cancelling(&mut self) -> Result<AppState> {
        self.require_state(AppState::Cancelling, "finish_cancelling")?;
        self.state = AppState::Idle;
        Ok(self.state)
    }

    pub fn fail(&mut self) -> Result<AppState> {
        if matches!(self.state, AppState::Idle | AppState::Error) {
            return Err(self.invalid_transition("fail"));
        }
        self.state = AppState::Error;
        Ok(self.state)
    }

    pub fn recover(&mut self) -> Result<AppState> {
        self.require_state(AppState::Error, "recover")?;
        self.state = AppState::Idle;
        Ok(self.state)
    }

    fn require_state(&self, required: AppState, event: &str) -> Result<()> {
        if self.state != required {
            return Err(self.invalid_transition(event));
        }
        Ok(())
    }

    fn invalid_transition(&self, event: &str) -> AppError {
        AppError::InvalidWorkflowTransition {
            state: format!("{:?}", self.state),
            event: event.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DispatchMode, DispatchRequest, PreparationFailureStage, PreparationOutcome, RecoveryHint,
    };

    #[test]
    fn capture_with_prompt_skips_local_prompt_and_reaches_dispatch_preparation() {
        let mut workflow = WorkflowController::default();

        assert_eq!(
            workflow
                .start(AppCommand::CaptureWithPrompt)
                .expect("start"),
            AppState::SelectingRegion
        );
        assert_eq!(
            workflow
                .capture_completed(AppCommand::CaptureWithPrompt)
                .expect("capture"),
            AppState::PreparingDispatch
        );
    }

    #[test]
    fn quick_capture_skips_prompt_and_text_skips_capture() {
        let mut quick = WorkflowController::default();
        quick
            .start(AppCommand::CaptureQuickDispatch)
            .expect("quick start");
        assert_eq!(
            quick
                .capture_completed(AppCommand::CaptureQuickDispatch)
                .expect("quick capture"),
            AppState::PreparingDispatch
        );

        let mut text = WorkflowController::default();
        assert_eq!(
            text.start(AppCommand::TextOnlyPrompt).expect("text start"),
            AppState::PreparingDispatch
        );
    }

    #[test]
    fn rejects_parallel_and_out_of_order_events() {
        let mut workflow = WorkflowController::default();
        workflow
            .start(AppCommand::CaptureWithPrompt)
            .expect("first workflow");

        assert!(matches!(
            workflow.start(AppCommand::TextOnlyPrompt),
            Err(AppError::WorkflowBusy(_))
        ));
        assert!(matches!(
            workflow.prompt_submitted(),
            Err(AppError::InvalidWorkflowTransition { .. })
        ));
    }

    #[test]
    fn cancellation_and_error_paths_return_to_idle() {
        let mut cancelled = WorkflowController::default();
        cancelled.start(AppCommand::TextOnlyPrompt).expect("start");
        cancelled.begin_cancelling().expect("cancel");
        assert_eq!(
            cancelled.finish_cancelling().expect("finish"),
            AppState::Idle
        );

        let mut failed = WorkflowController::default();
        failed.start(AppCommand::CaptureWithPrompt).expect("start");
        failed.fail().expect("fail");
        assert_eq!(failed.recover().expect("recover"), AppState::Idle);
    }

    #[test]
    fn phase4_browser_path_reaches_page_preparation_boundary() {
        let mut workflow = WorkflowController::default();
        workflow.start(AppCommand::TextOnlyPrompt).expect("start");

        assert_eq!(
            workflow.begin_browser().expect("begin browser"),
            AppState::StartingBrowser
        );
        assert_eq!(
            workflow.browser_started().expect("browser started"),
            AppState::ConnectingBrowser
        );
        assert_eq!(
            workflow.browser_connected().expect("browser connected"),
            AppState::ResolvingTarget
        );
        assert_eq!(
            workflow.target_resolved().expect("target resolved"),
            AppState::WaitingForPage
        );
        assert_eq!(
            workflow.page_ready().expect("page ready"),
            AppState::PreparingPage
        );
        assert_eq!(
            workflow.defer_page_preparation().expect("phase 5 boundary"),
            AppState::Idle
        );
    }

    #[test]
    fn phase4_rejects_out_of_order_browser_events() {
        let mut workflow = WorkflowController::default();
        workflow.start(AppCommand::TextOnlyPrompt).expect("start");

        assert!(matches!(
            workflow.browser_connected(),
            Err(AppError::InvalidWorkflowTransition { .. })
        ));
        workflow.begin_browser().expect("begin browser");
        assert!(matches!(
            workflow.target_resolved(),
            Err(AppError::InvalidWorkflowTransition { .. })
        ));
    }

    #[test]
    fn phase4_desktop_surface_skips_cdp_states() {
        let mut workflow = WorkflowController::default();
        workflow.start(AppCommand::TextOnlyPrompt).expect("start");
        workflow.begin_browser().expect("begin target");

        assert_eq!(
            workflow.desktop_surface_ready().expect("desktop PWA ready"),
            AppState::PreparingPage
        );
        assert_eq!(
            workflow.defer_page_preparation().expect("phase 5 boundary"),
            AppState::Idle
        );
    }

    #[test]
    fn phase5_prepared_and_fallback_results_follow_distinct_paths() {
        let request = DispatchRequest::new(
            "text-1".to_owned(),
            DispatchMode::TextOnlyPrompt,
            "chatgpt".to_owned(),
            "Explain".to_owned(),
            None,
            1,
        )
        .expect("request");

        let mut prepared = workflow_at_preparing_page();
        let prepared_outcome = DispatchOutcome::from_preparation(
            &request,
            PreparationOutcome::prepared("https://example.test/chat", true, false),
        )
        .expect("prepared outcome");
        assert_eq!(
            prepared.page_prepared(&prepared_outcome).expect("prepared"),
            AppState::PreparedForUser
        );
        assert_eq!(prepared.finish_delivery().expect("finish"), AppState::Idle);

        let mut fallback = workflow_at_preparing_page();
        let fallback_outcome = DispatchOutcome::from_preparation(
            &request,
            PreparationOutcome::manual_fallback(
                "https://example.test/chat",
                PreparationFailureStage::ComposerDiscovery,
                RecoveryHint::FocusComposerAndPaste,
                false,
                false,
            ),
        )
        .expect("fallback outcome");
        assert_eq!(
            fallback.page_prepared(&fallback_outcome).expect("fallback"),
            AppState::PreparingFallback
        );
        assert_eq!(
            fallback.fallback_prepared().expect("clipboard ready"),
            AppState::FallbackReady
        );
        assert_eq!(fallback.finish_delivery().expect("finish"), AppState::Idle);
    }

    #[test]
    fn recoverable_browser_failures_can_enter_fallback_from_every_browser_stage() {
        let stages = [
            AppState::PreparingDispatch,
            AppState::StartingBrowser,
            AppState::ConnectingBrowser,
            AppState::ResolvingTarget,
            AppState::WaitingForPage,
            AppState::PreparingPage,
        ];
        for stage in stages {
            let mut workflow = WorkflowController { state: stage };
            assert_eq!(
                workflow
                    .prepare_fallback_after_browser_failure()
                    .expect("fallback transition"),
                AppState::PreparingFallback
            );
            assert_eq!(
                workflow.fallback_prepared().expect("fallback ready"),
                AppState::FallbackReady
            );
        }
    }

    fn workflow_at_preparing_page() -> WorkflowController {
        let mut workflow = WorkflowController::default();
        workflow.start(AppCommand::TextOnlyPrompt).expect("start");
        workflow.begin_browser().expect("browser");
        workflow.browser_started().expect("started");
        workflow.browser_connected().expect("connected");
        workflow.target_resolved().expect("target");
        workflow.page_ready().expect("page");
        workflow
    }
}
