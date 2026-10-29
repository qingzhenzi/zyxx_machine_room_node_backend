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

    async fn send(&self, email: &str, subject: &str, html: &str, fallback_link: &str) {
        let Some(mailer) = &self.mailer else {
            tracing::info!(
                email,
                link = fallback_link,
                "开发邮件发送器：{subject} 链接（未配置 SMTP，生产请设置 EMAIL_DRIVER=smtp）"
            );
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
            tracing::error!(%error, email, fallback_link, "SMTP 发送失败");
        }
    }
}
