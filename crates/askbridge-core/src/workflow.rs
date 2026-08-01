use crate::{AppCommand, AppError, AppState, Result};

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
            AppCommand::TextOnlyPrompt => AppState::Prompting,
        };
        Ok(self.state)
    }

    pub fn capture_completed(&mut self, command: AppCommand) -> Result<AppState> {
        self.require_state(AppState::SelectingRegion, "capture_completed")?;
        self.state = match command {
            AppCommand::CaptureWithPrompt => AppState::Prompting,
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

    #[test]
    fn capture_with_prompt_reaches_dispatch_preparation() {
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
            AppState::Prompting
        );
        assert_eq!(
            workflow.prompt_submitted().expect("prompt"),
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
            AppState::Prompting
        );
        assert_eq!(
            text.prompt_submitted().expect("text prompt"),
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
        workflow.prompt_submitted().expect("prompt");

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
        workflow.prompt_submitted().expect("prompt");

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
        workflow.prompt_submitted().expect("prompt");
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
}
