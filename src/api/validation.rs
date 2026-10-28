use serde::Deserialize;

/// 通用分页/搜索查询参数。
#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub keyword: Option<String>,
    /// 仅后台公告列表使用；公开接口会忽略。
    #[serde(default)]
    pub status: Option<String>,
}
