use crate::server_sync::AccountProfile;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct StatusPayload {
    pub phase: &'static str,
    pub title: String,
    pub detail: String,
    pub code_available: bool,
    pub profile: Option<AccountProfile>,
    /// 代理与临时证书是否仍在接管网络；切换账号期间保持为 true。
    pub session_active: bool,
    /// 当前是否处于协议监听模式。
    pub protocol_mode: bool,
    /// 当前代理监听端口（仅会话运行期间有值）。
    pub proxy_port: Option<u16>,
    /// 当前正在等待进入农场的布防 QQ 号。
    pub armed_qq: Option<String>,
    /// 当前状态所属的 QQ 号，供前端把结果记到对应账号行。
    pub target_qq: Option<String>,
    /// 协议会话的完整目录，独立于状态文案展示及复制。
    pub capture_directory: Option<String>,
}

impl StatusPayload {
    pub fn idle() -> Self {
        Self::new(
            "idle",
            "未接管网络",
            "启动代理，或选择账号直接同步。",
            false,
        )
    }

    pub fn new(
        phase: &'static str,
        title: impl Into<String>,
        detail: impl Into<String>,
        code_available: bool,
    ) -> Self {
        Self {
            phase,
            title: title.into(),
            detail: detail.into(),
            code_available,
            profile: None,
            session_active: false,
            protocol_mode: false,
            proxy_port: None,
            armed_qq: None,
            target_qq: None,
            capture_directory: None,
        }
    }

    pub fn with_profile(mut self, profile: AccountProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    pub fn with_target_qq(mut self, qq: impl Into<String>) -> Self {
        self.target_qq = Some(qq.into());
        self
    }

    pub fn with_capture_directory(mut self, directory: &Path) -> Self {
        self.capture_directory = Some(directory.to_string_lossy().into_owned());
        self
    }
}
