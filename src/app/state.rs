use super::App;
use super::stream_session::{ConnectingStream, StreamStartTarget, StreamingSession};
use crate::api::catalog::Game;
use crate::{
    ConsolesResponse, DeviceCodeAuth, MsalAuth, Stream, StreamState, StreamingCredentials,
    WaitTimeResponse, XboxProfile,
};
use anyhow::Result;
use tokio::task::JoinHandle;

pub(crate) struct CredentialsLoadResult {
    pub(super) result: Result<(StreamingCredentials, Option<XboxProfile>)>,
    pub(super) auth: MsalAuth,
}

pub(crate) enum AppState {
    InitializeAuthentication,
    LanguageSelect {
        selected: usize,
    },
    RequestingDeviceCode(JoinHandle<Result<DeviceCodeAuth>>),
    WaitingForDeviceAuthorization {
        device_code: DeviceCodeAuth,
        job: JoinHandle<Result<MsalAuth>>,
    },
    LoadingCredentials(JoinHandle<Result<CredentialsLoadResult>>),
    ModeSelect {
        selected: usize,
    },
    LoadingTitles(JoinHandle<Result<Vec<Game>>>),
    TitleList {
        selected: usize,
    },
    LoadingConsoles(JoinHandle<Result<ConsolesResponse>>),
    ConsoleList {
        selected: usize,
    },
    StartingStream {
        target: StreamStartTarget,
        job: Option<JoinHandle<Result<Stream>>>,
    },
    Connecting {
        session: ConnectingStream,
        poll_job: Option<JoinHandle<Result<(Stream, StreamState)>>>,
        wait_estimate_job: Option<JoinHandle<Result<WaitTimeResponse>>>,
    },
    Streaming(Box<StreamingSession>),
    Settings {
        return_to: Box<AppState>,
        selected: usize,
        title_id: Option<String>,
        locale_expanded: bool,
    },
    Error {
        reason: String,
        details: String,
        retry_sign_in: bool,
    },
}

impl AppState {
    pub(super) fn abort_read_only_jobs(&self) {
        match self {
            Self::RequestingDeviceCode(job) => job.abort(),
            Self::WaitingForDeviceAuthorization { job, .. } => job.abort(),
            Self::LoadingCredentials(job) => job.abort(),
            Self::LoadingTitles(job) => job.abort(),
            Self::LoadingConsoles(job) => job.abort(),
            _ => {}
        }
    }

    fn keeps_stream_alive(&self) -> bool {
        match self {
            Self::Streaming(_) => true,
            Self::Settings { return_to, .. } => return_to.keeps_stream_alive(),
            _ => false,
        }
    }

    pub(crate) fn streaming(&self) -> Option<&StreamingSession> {
        match self {
            Self::Streaming(streaming) => Some(streaming),
            Self::Settings { return_to, .. } => return_to.streaming(),
            _ => None,
        }
    }

    pub(crate) fn streaming_mut(&mut self) -> Option<&mut StreamingSession> {
        match self {
            Self::Streaming(streaming) => Some(streaming),
            Self::Settings { return_to, .. } => return_to.streaming_mut(),
            _ => None,
        }
    }

    pub(super) fn active_title_id(&self) -> Option<&str> {
        match self {
            Self::StartingStream { target, .. } => target.game_id.as_deref(),
            Self::Connecting { session, .. } => session.game_id.as_deref(),
            Self::Streaming(streaming) => streaming.title_id.as_deref(),
            Self::Settings { return_to, .. } => return_to.active_title_id(),
            _ => None,
        }
    }

    pub(super) fn into_streaming(self) -> Option<StreamingSession> {
        match self {
            Self::Streaming(streaming) => Some(*streaming),
            Self::Settings { return_to, .. } => return_to.into_streaming(),
            _ => None,
        }
    }
}

impl App {
    pub async fn tick(&mut self) -> Result<()> {
        match &self.state {
            AppState::InitializeAuthentication => {
                if let Some(error) = self.service.auth.take_storage_error() {
                    self.set_localized_error_screen("error-login-storage", error);
                } else if self.service.auth.has_saved_login() {
                    self.load_credentials();
                } else {
                    let selected = crate::Locale::ALL
                        .iter()
                        .position(|&locale| locale == self.settings.locale)
                        .unwrap_or(0);
                    self.set_state(AppState::LanguageSelect { selected });
                }
            }
            AppState::RequestingDeviceCode(_)
            | AppState::WaitingForDeviceAuthorization { .. }
            | AppState::LoadingCredentials(_)
            | AppState::LoadingTitles(_)
            | AppState::LoadingConsoles(_) => self.pump_entry_state().await?,
            AppState::StartingStream { .. } | AppState::Connecting { .. } => {
                self.pump_connection().await?
            }
            AppState::Streaming(_) => self.pump_rtc_session().await?,
            AppState::Settings { return_to, .. } if return_to.keeps_stream_alive() => {
                self.pump_rtc_session().await?
            }
            AppState::TitleList { .. } => self.pump_title_details().await?,
            AppState::LanguageSelect { .. }
            | AppState::ModeSelect { .. }
            | AppState::ConsoleList { .. }
            | AppState::Settings { .. }
            | AppState::Error { .. } => {}
        }
        Ok(())
    }
}

impl AppState {
    /// A session creation POST must finish before cleanup can know its session ID.
    /// Home cleanup deliberately never terminates the console's game session.
    pub(crate) async fn shutdown(mut self) -> Result<()> {
        loop {
            match self {
                Self::Settings { return_to, .. } => {
                    self = *return_to;
                    continue;
                }
                Self::RequestingDeviceCode(job) => crate::jobs::cancel(job).await,
                Self::WaitingForDeviceAuthorization { job, .. } => crate::jobs::cancel(job).await,
                Self::LoadingCredentials(job) => crate::jobs::cancel(job).await,
                Self::LoadingTitles(job) => crate::jobs::cancel(job).await,
                Self::LoadingConsoles(job) => crate::jobs::cancel(job).await,
                Self::StartingStream { job: Some(job), .. } => {
                    if let Ok(Ok(stream)) = job.await {
                        stream.stop().await?;
                    }
                }
                Self::Connecting {
                    session,
                    poll_job,
                    wait_estimate_job,
                } => {
                    if let Some(job) = poll_job {
                        crate::jobs::cancel(job).await;
                    }
                    if let Some(job) = wait_estimate_job {
                        crate::jobs::cancel(job).await;
                    }
                    session.stream.stop().await?;
                }
                Self::Streaming(streaming) => (*streaming).stop().await?,
                _ => {}
            }
            return Ok(());
        }
    }
}

impl App {
    pub(crate) async fn shutdown(&mut self) -> Result<()> {
        if let Some(job) = self.catalog_collections_job.take() {
            crate::jobs::cancel(job).await;
        }
        let state = std::mem::replace(&mut self.state, AppState::InitializeAuthentication);
        state.shutdown().await
    }
}
