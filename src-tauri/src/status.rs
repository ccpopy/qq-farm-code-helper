use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct StatusPayload {
    pub phase: &'static str,
    pub title: String,
    pub detail: String,
    pub code_available: bool,
}

impl StatusPayload {
    pub fn idle() -> Self {
        Self::new(
            "idle",
            "尚未启动",
            "保存服务器设置后，启动一次性本地代理。",
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
        }
    }
}
