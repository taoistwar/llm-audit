use crate::{audit_log_max_chars_from_env, env_flag_explicit_false, env_flag_true, env_usize};
use reqwest::{Client, redirect};
use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::info;

fn http_client_timeout() -> Option<Duration> {
    match std::env::var("HTTP_CLIENT_TIMEOUT_SECS") {
        Ok(s) if s.trim() == "0" => None,
        Ok(s) => match s.trim().parse::<u64>() {
            Ok(secs) if secs > 0 => Some(Duration::from_secs(secs)),
            _ => Some(Duration::from_secs(600)),
        },
        Err(_) => Some(Duration::from_secs(600)),
    }
}

#[derive(Clone)]
pub struct LangfuseConfig {
    base_url: String,
    public_key: String,
    secret_key: String,
}

impl LangfuseConfig {
    pub fn factory() -> Option<Self> {
        if env_flag_explicit_false("LANGFUSE_ENABLE") {
            info!("Langfuse disabled: LANGFUSE_ENABLE is off (0/false/no/off)");
            return None;
        }

        let public_key = std::env::var("LANGFUSE_PUBLIC_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let secret_key = std::env::var("LANGFUSE_SECRET_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let base_url = std::env::var("LANGFUSE_BASE_URL")
            .unwrap_or_else(|_| "https://cloud.langfuse.com".to_string());

        match (public_key, secret_key) {
            (Some(public_key), Some(secret_key)) => {
                info!(
                    "Langfuse ingestion enabled ({})",
                    base_url.trim_end_matches('/')
                );
                Some(LangfuseConfig::new(base_url, public_key, secret_key))
            }
            _ => {
                info!("Langfuse disabled: set LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY");
                None
            }
        }
    }
    pub fn new(base_url: String, public_key: String, secret_key: String) -> Self {
        Self {
            base_url,
            public_key,
            secret_key,
        }
    }
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    pub fn public_key(&self) -> &str {
        &self.public_key
    }
    pub fn secret_key(&self) -> &str {
        &self.secret_key
    }
    pub fn set_base_url(&mut self, base_url: String) {
        self.base_url = base_url;
    }
    pub fn set_public_key(&mut self, public_key: String) {
        self.public_key = public_key;
    }
    pub fn set_secret_key(&mut self, secret_key: String) {
        self.secret_key = secret_key;
    }
}

#[derive(Clone)]
pub struct AppState {
    llm_url: String,
    http: Client,
    langfuse: Option<LangfuseConfig>,
    /// 为 true 时对每条经代理转发的 LLM 请求与响应写入本地审计日志文件（`target: llm_audit`），与 Langfuse 是否成功无关
    audit_log_enabled: bool,
    /// 审计日志中 input/output JSON 的最大 UTF-8 字节数（`usize::MAX` 表示不截断）
    audit_log_max_chars: usize,
    request_body_max_bytes: usize,
    response_capture_max_bytes: usize,
    background_tasks: Arc<BackgroundTasks>,
}

#[derive(Default)]
struct BackgroundTasks {
    pending: AtomicUsize,
    changed: Notify,
}

impl AppState {
    pub fn factory() -> Self {
        let llm_url = std::env::var("LLM_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string())
            .trim_end_matches('/')
            .to_string();

        let langfuse = LangfuseConfig::factory();

        let audit_log_enabled = env_flag_true("AUDIT_LOG_ENABLE");
        if audit_log_enabled {
            info!("AUDIT_LOG_ENABLE: write llm_audit logs for every proxied request/response");
        }
        let audit_log_max_chars = audit_log_max_chars_from_env();
        let request_body_max_bytes = env_usize("REQUEST_BODY_MAX_BYTES", 16 * 1024 * 1024);
        let response_capture_max_bytes = env_usize("RESPONSE_CAPTURE_MAX_BYTES", 4 * 1024 * 1024);
        if audit_log_max_chars == usize::MAX {
            info!("AUDIT_LOG_MAX_CHARS: unlimited (full input/output in audit logs)");
        }
        let mut http_builder = Client::builder().redirect(redirect::Policy::none());
        if let Some(d) = http_client_timeout() {
            info!("HTTP client timeout: {:?}", d);
            http_builder = http_builder.timeout(d);
        } else {
            info!("HTTP client timeout: disabled (HTTP_CLIENT_TIMEOUT_SECS=0)");
        }

        Self {
            llm_url,
            http: http_builder.build().expect("failed to build HTTP client"),
            langfuse,
            audit_log_enabled,
            audit_log_max_chars,
            request_body_max_bytes,
            response_capture_max_bytes,
            background_tasks: Arc::default(),
        }
    }
    pub fn new(
        llm_url: String,
        http: Client,
        langfuse: Option<LangfuseConfig>,
        audit_log_enabled: bool,
        audit_log_max_chars: usize,
    ) -> Self {
        Self {
            llm_url,
            http,
            langfuse,
            audit_log_enabled,
            audit_log_max_chars,
            request_body_max_bytes: 16 * 1024 * 1024,
            response_capture_max_bytes: 4 * 1024 * 1024,
            background_tasks: Arc::default(),
        }
    }
    pub fn llm_url(&self) -> &str {
        &self.llm_url
    }
    pub fn http(&self) -> &Client {
        &self.http
    }
    pub fn langfuse(&self) -> &Option<LangfuseConfig> {
        &self.langfuse
    }
    pub fn audit_log_enabled(&self) -> bool {
        self.audit_log_enabled
    }
    pub fn audit_log_max_chars(&self) -> usize {
        self.audit_log_max_chars
    }
    pub fn request_body_max_bytes(&self) -> usize {
        self.request_body_max_bytes
    }
    pub fn response_capture_max_bytes(&self) -> usize {
        self.response_capture_max_bytes
    }
    pub fn set_llm_url(&mut self, llm_url: String) {
        self.llm_url = llm_url;
    }
    pub fn set_http(&mut self, http: Client) {
        self.http = http;
    }
    pub fn set_langfuse(&mut self, langfuse: Option<LangfuseConfig>) {
        self.langfuse = langfuse;
    }
    pub fn set_audit_log_enabled(&mut self, audit_log_enabled: bool) {
        self.audit_log_enabled = audit_log_enabled;
    }
    pub fn set_audit_log_max_chars(&mut self, audit_log_max_chars: usize) {
        self.audit_log_max_chars = audit_log_max_chars;
    }
    pub fn set_request_body_max_bytes(&mut self, max: usize) {
        self.request_body_max_bytes = max;
    }
    pub fn set_response_capture_max_bytes(&mut self, max: usize) {
        self.response_capture_max_bytes = max;
    }
    pub fn spawn_background<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let tasks = self.background_tasks.clone();
        tasks.pending.fetch_add(1, Ordering::AcqRel);
        tokio::spawn(async move {
            future.await;
            tasks.pending.fetch_sub(1, Ordering::AcqRel);
            tasks.changed.notify_waiters();
        });
    }
    pub async fn wait_for_background_tasks(&self, timeout: Duration) {
        let tasks = self.background_tasks.clone();
        let wait = async move {
            loop {
                let changed = tasks.changed.notified();
                if tasks.pending.load(Ordering::Acquire) == 0 {
                    break;
                }
                changed.await;
            }
        };
        let _ = tokio::time::timeout(timeout, wait).await;
    }
}
