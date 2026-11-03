//! 邮件发送器：`log`（开发）或 `smtp`（生产可用）。
//!
//! 通过 `EMAIL_DRIVER=smtp`、`SMTP_URL=smtp://user:pass@host:port` 配置；
//! 本地可用 Mailpit（`compose.mail.yaml`）验证。

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
};

use crate::config::Config;

#[derive(Clone)]
pub struct EmailService {
    base_url: String,
    from: Mailbox,
    mailer: Option<AsyncSmtpTransport<Tokio1Executor>>,
}

impl EmailService {
    pub fn new(config: &Config) -> Self {
        let from = config.smtp_from.parse::<Mailbox>().unwrap_or_else(|_| {
            "QEStudio <no-reply@qestudio.local>"
                .parse()
                .expect("valid default mailbox")
        });

        let mailer = if config.email_driver.eq_ignore_ascii_case("smtp") {
            match AsyncSmtpTransport::<Tokio1Executor>::from_url(&config.smtp_url) {
                Ok(builder) => Some(builder.build()),
                Err(error) => {
                    tracing::error!(%error, "SMTP_URL 配置无效，邮件将退化为日志输出");
                    None
                }
            }
        } else {
            None
        };

        Self {
            base_url: config.public_base_url.clone(),
            from,
            mailer,
        }
    }

    pub async fn send_password_reset(&self, email: &str, token: &str) {
        let link = format!("{}/#/reset-password?token={token}", self.base_url);
        self.send(
            email,
            "重置 QEStudio 密码",
            &format!(
                "<p>你请求重置 QEStudio 密码。</p><p><a href=\"{link}\">打开重置页面</a></p><p>链接 30 分钟内有效；如果不是你本人操作，请忽略此邮件。</p>"
            ),
            &link,
        )
        .await;
    }

    pub async fn send_email_verification(&self, email: &str, token: &str) {
        let link = format!("{}/#/verify-email?token={token}", self.base_url);
        self.send(
            email,
            "验证 QEStudio 邮箱",
            &format!(
                "<p>请验证你的 QEStudio 邮箱。</p><p><a href=\"{link}\">完成验证</a></p><p>如果不是你本人操作，请忽略此邮件。</p>"
            ),
            &link,
        )
        .await;
    }

    fn ticket_link(&self, ticket_id: &str) -> String {
        format!(
            "{}/tickets/{}",
            self.base_url.trim_end_matches('/'),
            ticket_id
        )
    }

    /// 工单转派通知（best-effort，收件人可为空由调用方判断）。
    pub async fn send_ticket_assigned(
        &self,
        email: &str,
        code: &str,
        title: &str,
        ticket_id: &str,
    ) {
        let link = self.ticket_link(ticket_id);
        self.send(
            email,
            &format!("工单已转派给你：{code}"),
            &format!(
                "<p>你被指派处理工单 <strong>{code}</strong>。</p><p>标题：{title}</p><p><a href=\"{link}\">打开工单</a></p>"
            ),
            &link,
        )
        .await;
    }

    /// 工单超时提醒（best-effort）。
    pub async fn send_ticket_overdue(
        &self,
        email: &str,
        code: &str,
        title: &str,
        ticket_id: &str,
        due_at: &str,
    ) {
        let link = self.ticket_link(ticket_id);
        self.send(
            email,
            &format!("工单已超时：{code}"),
            &format!(
                "<p>工单 <strong>{code}</strong> 已超过时效（截止 {due_at}）仍未解决。</p><p>标题：{title}</p><p><a href=\"{link}\">打开工单</a></p>"
            ),
            &link,
        )
        .await;
    }

    async fn send(&self, email: &str, subject: &str, html: &str, fallback_link: &str) {
        let Some(mailer) = &self.mailer else {
            // 日志里不默认输出令牌；本地调试时用 RUST_LOG=debug 查看。
            tracing::info!(
                email,
                "开发邮件发送器：{subject}（未配置 SMTP，链接已隐藏；RUST_LOG=debug 可查看）"
            );
            tracing::debug!(email, link = fallback_link, "开发邮件链接");
            return;
        };

        let message = Message::builder()
            .from(self.from.clone())
            .to(match email.parse() {
                Ok(mailbox) => mailbox,
                Err(error) => {
                    tracing::warn!(%error, email, "收件人邮箱格式无效，跳过邮件发送");
                    return;
                }
            })
            .subject(subject)
            .header(ContentType::TEXT_HTML)
            .body(html.to_owned());

        let message = match message {
            Ok(message) => message,
            Err(error) => {
                tracing::error!(%error, email, "构建邮件失败");
                return;
            }
        };

        if let Err(error) = mailer.send(message).await {
            // 不记录 fallback_link，避免令牌进入集中日志。
            tracing::error!(%error, email, "SMTP 发送失败");
        }
    }
}
