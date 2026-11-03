//! 离线导出 OpenAPI spec 到 stdout（不需要 PostgreSQL / Redis / 启动 HTTP 服务）。
//!
//! 用法：
//! ```bash
//! cargo run -p qe-api --example dump_openapi > frontend/docs/openapi.json
//! ```

fn main() -> anyhow::Result<()> {
    let spec = qe_api::api::openapi::spec();
    println!("{}", serde_json::to_string_pretty(&spec)?);
    Ok(())
}
