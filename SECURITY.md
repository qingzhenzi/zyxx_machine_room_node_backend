# 安全策略

## 报告漏洞

请**不要**通过公开 issue 报告安全漏洞。优先使用 GitHub 的
[Private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)，
或通过仓库维护者公开的安全联系方式私下发送。

报告时请包含：

- 受影响的版本/提交
- 复现步骤或 PoC
- 影响范围（认证绕过、越权、注入、信息泄露等）
- 你希望的披露时间

我们会在确认后尽快回复，并在修复发布后致谢（如你愿意）。

## 部署前检查

- 设置 `APP_ENV=production`
- 用 `openssl rand -hex 32` 生成 `JWT_SECRET`
- 修改 `BOOTSTRAP_ADMIN_PASSWORD` 和 `POSTGRES_PASSWORD`
- 不要在生产使用 `EMAIL_DRIVER=log`
- 通过网关/负载均衡终止 TLS，并限制管理接口来源
