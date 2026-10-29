//! 输入规范化与校验。所有对外文案均为中文，方便前端直接展示。

use std::collections::BTreeSet;

use crate::{
    error::{DomainError, DomainResult},
    models::AnnouncementStatus,
};

pub const MAX_PAGE_SIZE: i64 = 100;
pub const DEFAULT_PAGE_SIZE: i64 = 20;

pub fn normalize_username(raw: &str) -> DomainResult<String> {
    let username = raw.trim().to_ascii_lowercase();
    let length = username.chars().count();
    if !(3..=32).contains(&length) {
        return Err(DomainError::bad_request("用户名长度需为 3-32 个字符"));
    }
    let mut characters = username.chars();
    let first = characters
        .next()
        .ok_or_else(|| DomainError::bad_request("用户名不能为空"))?;
    if !first.is_ascii_alphabetic() {
        return Err(DomainError::bad_request("用户名必须以字母开头"));
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(DomainError::bad_request(
            "用户名只能包含字母、数字、下划线、短横线和点",
        ));
    }
    Ok(username)
}

pub fn validate_password(raw: &str) -> DomainResult<()> {
    let length = raw.chars().count();
    if !(8..=128).contains(&length) {
        return Err(DomainError::bad_request("密码长度需为 8-128 个字符"));
    }
    if raw.trim().is_empty() {
        return Err(DomainError::bad_request("密码不能全为空白字符"));
    }
    Ok(())
}

pub fn normalize_optional_email(raw: &str) -> DomainResult<Option<String>> {
    let email = raw.trim().to_ascii_lowercase();
    if email.is_empty() {
        return Ok(None);
    }
    if email.chars().count() > 254 || email.chars().any(char::is_whitespace) {
        return Err(DomainError::bad_request("邮箱格式不正确"));
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err(DomainError::bad_request("邮箱格式不正确"));
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') || !domain.contains('.') {
        return Err(DomainError::bad_request("邮箱格式不正确"));
    }
    Ok(Some(email))
}

pub fn normalize_display_name(raw: &str) -> DomainResult<String> {
    let display_name = raw.trim().to_owned();
    let length = display_name.chars().count();
    if !(1..=64).contains(&length) {
        return Err(DomainError::bad_request("显示名称长度需为 1-64 个字符"));
    }
    Ok(display_name)
}

pub fn normalize_role_code(raw: &str) -> DomainResult<String> {
    let code = raw.trim().to_ascii_lowercase();
    let length = code.chars().count();
    if !(2..=32).contains(&length) {
        return Err(DomainError::bad_request("角色编码长度需为 2-32 个字符"));
    }
    let mut characters = code.chars();
    let first = characters
        .next()
        .ok_or_else(|| DomainError::bad_request("角色编码不能为空"))?;
    if !first.is_ascii_alphabetic() {
        return Err(DomainError::bad_request("角色编码必须以字母开头"));
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(DomainError::bad_request(
            "角色编码只能包含字母、数字、下划线和短横线",
        ));
    }
    Ok(code)
}

pub fn normalize_role_name(raw: &str) -> DomainResult<String> {
    let name = raw.trim().to_owned();
    let length = name.chars().count();
    if !(1..=64).contains(&length) {
        return Err(DomainError::bad_request("角色名称长度需为 1-64 个字符"));
    }
    Ok(name)
}

pub fn normalize_description(raw: &str) -> DomainResult<String> {
    let description = raw.trim().to_owned();
    if description.chars().count() > 255 {
        return Err(DomainError::bad_request("描述不能超过 255 个字符"));
    }
    Ok(description)
}

pub fn normalize_role_codes(codes: &[String]) -> DomainResult<Vec<String>> {
    if codes.len() > 32 {
        return Err(DomainError::bad_request("一次最多设置 32 个角色"));
    }
    let mut normalized = BTreeSet::new();
    for code in codes {
        normalized.insert(normalize_role_code(code)?);
    }
    Ok(normalized.into_iter().collect())
}

pub fn normalize_permission_codes(codes: &[String]) -> DomainResult<Vec<String>> {
    if codes.len() > 64 {
        return Err(DomainError::bad_request("一次最多设置 64 个权限"));
    }
    let mut normalized = BTreeSet::new();
    for code in codes {
        normalized.insert(normalize_permission_code(code)?);
    }
    Ok(normalized.into_iter().collect())
}

fn normalize_permission_code(raw: &str) -> DomainResult<String> {
    let code = raw.trim().to_ascii_lowercase();
    if code.chars().count() > 64 {
        return Err(DomainError::bad_request("权限编码不能超过 64 个字符"));
    }
    let Some((resource, action)) = code.split_once(':') else {
        return Err(DomainError::bad_request("权限编码格式应为 <资源>:<动作>"));
    };
    if resource.is_empty() || action.is_empty() || action.contains(':') {
        return Err(DomainError::bad_request("权限编码格式应为 <资源>:<动作>"));
    }
    for part in [resource, action] {
        let mut characters = part.chars();
        let first = characters
            .next()
            .ok_or_else(|| DomainError::bad_request("权限编码的资源与动作都不能为空"))?;
        if !first.is_ascii_alphabetic() {
            return Err(DomainError::bad_request(
                "权限编码的资源与动作必须以字母开头",
            ));
        }
        if !characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        {
            return Err(DomainError::bad_request(
                "权限编码只能包含字母、数字、下划线和短横线",
            ));
        }
    }
    Ok(code)
}

pub fn normalize_announcement_title(raw: &str) -> DomainResult<String> {
    let title = raw.trim().to_owned();
    let length = title.chars().count();
    if !(1..=200).contains(&length) {
        return Err(DomainError::bad_request("公告标题长度需为 1-200 个字符"));
    }
    Ok(title)
}

pub fn normalize_announcement_content(raw: &str) -> DomainResult<String> {
    let content = raw.trim().to_owned();
    let length = content.chars().count();
    if !(1..=100_000).contains(&length) {
        return Err(DomainError::bad_request("公告正文长度需为 1-100000 个字符"));
    }
    Ok(content)
}

pub fn normalize_keyword(raw: Option<&str>) -> DomainResult<String> {
    let keyword = raw.unwrap_or("").trim().to_owned();
    if keyword.chars().count() > 100 {
        return Err(DomainError::bad_request("搜索关键词不能超过 100 个字符"));
    }
    Ok(keyword)
}

pub fn parse_announcement_status(raw: Option<&str>) -> DomainResult<Option<AnnouncementStatus>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => value.parse::<AnnouncementStatus>().map(Some).map_err(|_| {
            DomainError::bad_request("无效的公告状态，可选值为 draft / published / archived")
        }),
    }
}

pub fn page_params(page: Option<i64>, page_size: Option<i64>) -> DomainResult<(i64, i64)> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if page < 1 {
        return Err(DomainError::bad_request("page 必须大于等于 1"));
    }
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(DomainError::bad_request(format!(
            "page_size 需在 1-{MAX_PAGE_SIZE} 之间"
        )));
    }
    if page > 1_000_000 {
        return Err(DomainError::bad_request("page 过大"));
    }
    Ok((page, page_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_is_normalized_and_validated() {
        assert_eq!(normalize_username("  Admin.User  ").unwrap(), "admin.user");
        assert!(normalize_username("ab").is_err());
        assert!(normalize_username("1admin").is_err());
        assert!(normalize_username("admin space").is_err());
    }

    #[test]
    fn password_length_is_enforced() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("long-enough-password").is_ok());
        assert!(validate_password("        ").is_err());
    }

    #[test]
    fn email_is_optional_and_normalized() {
        assert_eq!(normalize_optional_email("  ").unwrap(), None);
        assert_eq!(
            normalize_optional_email(" Admin@Example.COM ").unwrap(),
            Some("admin@example.com".to_owned())
        );
        assert!(normalize_optional_email("not-an-email").is_err());
        assert!(normalize_optional_email("a@b").is_err());
    }

    #[test]
    fn role_and_permission_codes_are_deduplicated() {
        let roles = normalize_role_codes(&["viewer".into(), " ViewER ".into()]).unwrap();
        assert_eq!(roles, vec!["viewer".to_owned()]);

        let permissions =
            normalize_permission_codes(&["announcement:read".into(), "announcement:read".into()])
                .unwrap();
        assert_eq!(permissions, vec!["announcement:read".to_owned()]);
        assert!(normalize_permission_codes(&["announcement".into()]).is_err());
    }

    #[test]
    fn pagination_is_bounded() {
        assert_eq!(page_params(None, None).unwrap(), (1, DEFAULT_PAGE_SIZE));
        assert!(page_params(Some(0), None).is_err());
        assert!(page_params(None, Some(0)).is_err());
        assert!(page_params(None, Some(MAX_PAGE_SIZE + 1)).is_err());
    }
}
