//! 输入规范化与校验。所有对外文案均为中文，方便前端直接展示。

use std::collections::BTreeSet;

use chrono::{Days, NaiveDate, NaiveTime};
use chrono_tz::Tz;

use crate::{
    error::{DomainError, DomainResult},
    models::AnnouncementStatus,
    schedule::{
        NewEventRule, NewExceptionSlot, NewWeeklySlot, TriggerKind, ValidatedEventRule,
        ValidatedExceptionSlot, ValidatedWeeklySlot,
    },
};

pub use crate::storage::{
    validate_image_name, validate_image_version, validate_manifest, validate_object_key,
    validate_platform, validate_sha256_hex,
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

/// 权限编码：`<资源>:<动作>`，允许最多 4 段（如 `ticket:rule:manage`、
/// `ticket:scope:all`）。每段都必须以字母开头，只含字母/数字/下划线/短横线。
fn normalize_permission_code(raw: &str) -> DomainResult<String> {
    let code = raw.trim().to_ascii_lowercase();
    if code.chars().count() > 64 {
        return Err(DomainError::bad_request("权限编码不能超过 64 个字符"));
    }
    const FORMAT_HINT: &str = "权限编码格式应为 <资源>:<动作> 或 <资源>:<子资源>:<动作>";
    let parts: Vec<&str> = code.split(':').collect();
    if !(2..=4).contains(&parts.len()) || parts.iter().any(|part| part.is_empty()) {
        return Err(DomainError::bad_request(FORMAT_HINT));
    }
    for part in parts {
        let mut characters = part.chars();
        let first = characters
            .next()
            .ok_or_else(|| DomainError::bad_request("权限编码的每一段都不能为空"))?;
        if !first.is_ascii_alphabetic() {
            return Err(DomainError::bad_request("权限编码的每一段必须以字母开头"));
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

pub fn sanitize_announcement_content(raw: &str) -> String {
    ammonia::clean(raw)
}

pub fn normalize_announcement_content(raw: &str) -> DomainResult<String> {
    // 公告允许有限的富文本；写入前统一清洗，避免存储型 XSS。
    // 前端仍应使用 DOMPurify 等库做第二道防线。
    let content = sanitize_announcement_content(raw.trim());
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

const MAX_DEVICE_SLOT_LEN: usize = 128;
const MIN_TOKEN_TTL_SECS: i64 = 60;
const MAX_TOKEN_TTL_SECS: i64 = 86_400;

/// 规范化设备槽位：去空白；长度 1..=128；只允许 ascii 字母数字与 `- _ .`。
pub fn normalize_device_slot(raw: &str) -> DomainResult<String> {
    let slot = raw.trim();
    let length = slot.chars().count();
    if length == 0 {
        return Err(DomainError::bad_request("设备槽位不能为空"));
    }
    if length > MAX_DEVICE_SLOT_LEN {
        return Err(DomainError::bad_request(format!(
            "设备槽位长度需为 1-{MAX_DEVICE_SLOT_LEN} 个字符"
        )));
    }
    if !slot
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(DomainError::bad_request(
            "设备槽位只能包含字母、数字、短横线、下划线和点",
        ));
    }
    Ok(slot.to_owned())
}

/// 校验设备注册 token 有效期：60..=86400 秒。
pub fn validate_token_ttl_secs(seconds: i64) -> DomainResult<i64> {
    if !(MIN_TOKEN_TTL_SECS..=MAX_TOKEN_TTL_SECS).contains(&seconds) {
        return Err(DomainError::bad_request(format!(
            "Token 有效期需在 {MIN_TOKEN_TTL_SECS}-{MAX_TOKEN_TTL_SECS} 秒之间"
        )));
    }
    Ok(seconds)
}

/// 课表相关输入的上限值。
pub const MAX_WEEKLY_SLOTS: usize = 200;
pub const MAX_EXCEPTION_SLOTS: usize = 100;
pub const MAX_EVENT_RULES: usize = 50;
pub const MAX_OCCURRENCE_RANGE_DAYS: i64 = 366;

const DEFAULT_OCCURRENCE_RANGE_DAYS: i64 = 7;
const METADATA_MAX_BYTES: usize = 16 * 1024;
const RULE_CONFIG_MAX_BYTES: usize = 8 * 1024;

/// 规范化编码：去空白、转小写；长度 2..=64、以字母开头、只含 ascii 字母数字与 `- _ .`。
pub fn normalize_code(raw: &str, label: &str) -> DomainResult<String> {
    let code = raw.trim().to_ascii_lowercase();
    let length = code.chars().count();
    if !(2..=64).contains(&length) {
        return Err(DomainError::bad_request(format!(
            "{label}长度需为 2-64 个字符"
        )));
    }
    let mut characters = code.chars();
    let first = characters
        .next()
        .ok_or_else(|| DomainError::bad_request(format!("{label}不能为空")))?;
    if !first.is_ascii_alphabetic() {
        return Err(DomainError::bad_request(format!("{label}必须以字母开头")));
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(DomainError::bad_request(format!(
            "{label}只能包含字母、数字、短横线、下划线和点"
        )));
    }
    Ok(code)
}

/// 规范化课表名称：去空白，长度 1..=200。
pub fn normalize_schedule_name(raw: &str) -> DomainResult<String> {
    let name = raw.trim().to_owned();
    let length = name.chars().count();
    if !(1..=200).contains(&length) {
        return Err(DomainError::bad_request("课表名称长度需为 1-200 个字符"));
    }
    Ok(name)
}

/// 规范化课程名称：去空白，长度 1..=200。
pub fn normalize_course_name(raw: &str) -> DomainResult<String> {
    let name = raw.trim().to_owned();
    let length = name.chars().count();
    if !(1..=200).contains(&length) {
        return Err(DomainError::bad_request("课程名称长度需为 1-200 个字符"));
    }
    Ok(name)
}

/// 规范化教师名称：允许为空，最长 100 个字符。
pub fn normalize_teacher(raw: &str) -> DomainResult<String> {
    let teacher = raw.trim().to_owned();
    if teacher.chars().count() > 100 {
        return Err(DomainError::bad_request("教师名称不能超过 100 个字符"));
    }
    Ok(teacher)
}

/// 规范化备注：允许为空，最长 1000 个字符。
pub fn normalize_note(raw: &str) -> DomainResult<String> {
    let note = raw.trim().to_owned();
    if note.chars().count() > 1000 {
        return Err(DomainError::bad_request("备注不能超过 1000 个字符"));
    }
    Ok(note)
}

/// 规范化 Webhook 地址：空 => `None`；只允许 http/https；最长 2000 个字符。
pub fn normalize_webhook_url(raw: &str) -> DomainResult<Option<String>> {
    let url = raw.trim();
    if url.is_empty() {
        return Ok(None);
    }
    if url.chars().count() > 2000 {
        return Err(DomainError::bad_request("Webhook 地址不能超过 2000 个字符"));
    }
    if url.chars().any(char::is_whitespace) {
        return Err(DomainError::bad_request("Webhook 地址不能包含空白字符"));
    }
    let lowercase = url.to_ascii_lowercase();
    if !(lowercase.starts_with("http://") || lowercase.starts_with("https://")) {
        return Err(DomainError::bad_request(
            "Webhook 地址必须以 http:// 或 https:// 开头",
        ));
    }
    Ok(Some(url.to_owned()))
}

/// 校验房间容量：0..=100000。
pub fn validate_capacity(capacity: i32) -> DomainResult<i32> {
    if !(0..=100_000).contains(&capacity) {
        return Err(DomainError::bad_request("容量需在 0-100000 之间"));
    }
    Ok(capacity)
}

/// 规范化 IANA 时区名：去空白后用 [`chrono_tz::Tz`] 解析，返回标准名称（`Tz::name()`）。
///
/// 例：`" US/Eastern "` => `"America/New_York"`；无法解析的时区名返回 400。
pub fn validate_timezone(raw: &str) -> DomainResult<String> {
    Ok(parse_timezone(raw)?.name().to_owned())
}

/// 解析 IANA 时区名（供领域层内部换算使用，错误语义同 [`validate_timezone`]）。
pub(crate) fn parse_timezone(raw: &str) -> DomainResult<Tz> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(DomainError::bad_request(
            "时区不能为空，请使用 IANA 时区名（如 Asia/Shanghai）",
        ));
    }
    name.parse::<Tz>().map_err(|_| {
        DomainError::bad_request(format!(
            "无效的时区 {name}，请使用 IANA 时区名（如 Asia/Shanghai）"
        ))
    })
}

/// 校验元数据：必须是 JSON 对象，序列化后不超过 16KB。
pub fn validate_metadata(metadata: serde_json::Value) -> DomainResult<serde_json::Value> {
    validate_json_object(metadata, "元数据", METADATA_MAX_BYTES)
}

/// 校验日期范围：`start <= end`。
pub fn validate_date_range(start: NaiveDate, end: NaiveDate) -> DomainResult<()> {
    if start > end {
        return Err(DomainError::bad_request("开始日期不能晚于结束日期"));
    }
    Ok(())
}

/// 解析 `"HH:MM"` 或 `"HH:MM:SS"` 形式的本地时间。
pub fn parse_time(raw: &str) -> DomainResult<NaiveTime> {
    let value = raw.trim();
    let parts: Vec<&str> = value.split(':').collect();
    let (hour, minute, second) = match parts.as_slice() {
        [hour, minute] => (*hour, *minute, "00"),
        [hour, minute, second] => (*hour, *minute, *second),
        _ => return Err(invalid_time()),
    };
    let hour = parse_two_digits(hour).ok_or_else(invalid_time)?;
    let minute = parse_two_digits(minute).ok_or_else(invalid_time)?;
    let second = parse_two_digits(second).ok_or_else(invalid_time)?;
    NaiveTime::from_hms_opt(hour, minute, second).ok_or_else(invalid_time)
}

/// 解析可选时间：`None` / 空白 => `None`。
pub fn parse_optional_time(raw: Option<&str>) -> DomainResult<Option<NaiveTime>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => parse_time(value).map(Some),
    }
}

/// 校验周模板：星期 1..=7、上课早于下课、同一星期几内时段不重叠，最多 200 条。
pub fn validate_weekly_slots(slots: &[NewWeeklySlot]) -> DomainResult<Vec<ValidatedWeeklySlot>> {
    if slots.len() > MAX_WEEKLY_SLOTS {
        return Err(DomainError::bad_request(format!(
            "一次最多设置 {MAX_WEEKLY_SLOTS} 条周模板"
        )));
    }
    let mut validated = Vec::with_capacity(slots.len());
    for slot in slots {
        if !(1..=7).contains(&slot.day_of_week) {
            return Err(DomainError::bad_request("星期需在 1-7 之间（周一为 1）"));
        }
        let start_time = parse_time(&slot.start_time)?;
        let end_time = parse_time(&slot.end_time)?;
        if start_time >= end_time {
            return Err(DomainError::bad_request("上课时间必须早于下课时间"));
        }
        validated.push(ValidatedWeeklySlot {
            day_of_week: slot.day_of_week,
            start_time,
            end_time,
            course_name: normalize_course_name(&slot.course_name)?,
            teacher: normalize_teacher(&slot.teacher)?,
            room_id: slot.room_id,
            note: normalize_note(&slot.note)?,
        });
    }
    for day_of_week in 1..=7i16 {
        let mut times: Vec<(NaiveTime, NaiveTime)> = validated
            .iter()
            .filter(|slot| slot.day_of_week == day_of_week)
            .map(|slot| (slot.start_time, slot.end_time))
            .collect();
        times.sort_unstable();
        if times.windows(2).any(|pair| pair[1].0 < pair[0].1) {
            return Err(DomainError::bad_request("同一星期几的时段不能重叠"));
        }
    }
    Ok(validated)
}

/// 校验单日覆盖时段：上课早于下课、同一天时段不重叠，最多 100 条。
pub fn validate_exception_slots(
    slots: &[NewExceptionSlot],
) -> DomainResult<Vec<ValidatedExceptionSlot>> {
    if slots.len() > MAX_EXCEPTION_SLOTS {
        return Err(DomainError::bad_request(format!(
            "一次最多设置 {MAX_EXCEPTION_SLOTS} 条覆盖时段"
        )));
    }
    let mut validated = Vec::with_capacity(slots.len());
    for slot in slots {
        let start_time = parse_time(&slot.start_time)?;
        let end_time = parse_time(&slot.end_time)?;
        if start_time >= end_time {
            return Err(DomainError::bad_request("上课时间必须早于下课时间"));
        }
        validated.push(ValidatedExceptionSlot {
            start_time,
            end_time,
            course_name: normalize_course_name(&slot.course_name)?,
            teacher: normalize_teacher(&slot.teacher)?,
            room_id: slot.room_id,
            note: normalize_note(&slot.note)?,
        });
    }
    let mut times: Vec<(NaiveTime, NaiveTime)> = validated
        .iter()
        .map(|slot| (slot.start_time, slot.end_time))
        .collect();
    times.sort_unstable();
    if times.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return Err(DomainError::bad_request("当天覆盖时段不能重叠"));
    }
    Ok(validated)
}

/// 校验触发规则：`at_time` 仅 `daily_time` 使用且必填、偏移 0..=1440、
/// `occurrence_start_before` 偏移必须大于 0、`config` 是对象且不超过 8KB；
/// 同一 `(trigger_kind, offset_minutes, at_time)` 只保留第一条（去重），最多 50 条。
pub fn validate_event_rules(rules: &[NewEventRule]) -> DomainResult<Vec<ValidatedEventRule>> {
    if rules.len() > MAX_EVENT_RULES {
        return Err(DomainError::bad_request(format!(
            "一次最多设置 {MAX_EVENT_RULES} 条触发规则"
        )));
    }
    let mut validated: Vec<ValidatedEventRule> = Vec::with_capacity(rules.len());
    for rule in rules {
        if !(0..=1440).contains(&rule.offset_minutes) {
            return Err(DomainError::bad_request("触发偏移需在 0-1440 分钟之间"));
        }
        let at_time = match rule.trigger_kind {
            TriggerKind::DailyTime => Some(
                parse_optional_time(rule.at_time.as_deref())?
                    .ok_or_else(|| DomainError::bad_request("每日定时触发必须设置触发时间"))?,
            ),
            _ => {
                if rule
                    .at_time
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty())
                {
                    return Err(DomainError::bad_request("只有每日定时触发可以设置触发时间"));
                }
                None
            }
        };
        if rule.trigger_kind == TriggerKind::OccurrenceStartBefore && rule.offset_minutes <= 0 {
            return Err(DomainError::bad_request("提前提醒的偏移必须大于 0 分钟"));
        }
        let config =
            validate_json_object(rule.config.clone(), "触发规则配置", RULE_CONFIG_MAX_BYTES)?;
        let duplicate = validated.iter().any(|existing| {
            existing.trigger_kind == rule.trigger_kind
                && existing.offset_minutes == rule.offset_minutes
                && existing.at_time == at_time
        });
        if duplicate {
            continue;
        }
        validated.push(ValidatedEventRule {
            trigger_kind: rule.trigger_kind,
            offset_minutes: rule.offset_minutes,
            at_time,
            enabled: rule.enabled,
            config,
        });
    }
    Ok(validated)
}

/// 校验排课实例查询范围：默认 `[today, today+7]`；`to >= from`；跨度不超过 366 天。
pub fn validate_occurrence_range(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    today: NaiveDate,
) -> DomainResult<(NaiveDate, NaiveDate)> {
    let from = from.unwrap_or(today);
    let to = match to {
        Some(to) => to,
        None => from
            .checked_add_days(Days::new(DEFAULT_OCCURRENCE_RANGE_DAYS as u64))
            .ok_or_else(|| DomainError::bad_request("日期超出支持范围"))?,
    };
    if to < from {
        return Err(DomainError::bad_request("结束日期不能早于开始日期"));
    }
    if (to - from).num_days() > MAX_OCCURRENCE_RANGE_DAYS {
        return Err(DomainError::bad_request(format!(
            "查询跨度不能超过 {MAX_OCCURRENCE_RANGE_DAYS} 天"
        )));
    }
    Ok((from, to))
}

fn invalid_time() -> DomainError {
    DomainError::bad_request("时间格式不正确，应为 HH:MM 或 HH:MM:SS")
}

fn parse_two_digits(raw: &str) -> Option<u32> {
    if raw.len() != 2 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

fn validate_json_object(
    value: serde_json::Value,
    label: &str,
    max_bytes: usize,
) -> DomainResult<serde_json::Value> {
    if !value.is_object() {
        return Err(DomainError::bad_request(format!("{label}必须是 JSON 对象")));
    }
    let size = serde_json::to_vec(&value)
        .map_err(|error| DomainError::internal(format!("{label}序列化失败：{error}")))?
        .len();
    if size > max_bytes {
        return Err(DomainError::bad_request(format!(
            "{label}不能超过 {}KB",
            max_bytes / 1024
        )));
    }
    Ok(value)
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
    fn announcement_content_is_sanitized() {
        let content = normalize_announcement_content(
            r#"<p>hello</p><script>alert(1)</script><a href="javascript:alert(1)">x</a>"#,
        )
        .unwrap();
        assert!(content.contains("hello"));
        assert!(!content.contains("<script"));
        assert!(!content.contains("javascript:"));
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
        // 多段权限码（P1 细粒度）允许 2-4 段；非法段数/空段拒绝。
        assert_eq!(
            normalize_permission_codes(&[
                "ticket:rule:manage".into(),
                "ticket:scope:all".into(),
                "ticket:scope:manage".into(),
            ])
            .unwrap(),
            vec![
                "ticket:rule:manage".to_owned(),
                "ticket:scope:all".to_owned(),
                "ticket:scope:manage".to_owned(),
            ]
        );
        assert!(normalize_permission_codes(&["a:b:c:d:e".into()]).is_err());
        assert!(normalize_permission_codes(&["ticket::manage".into()]).is_err());
    }

    #[test]
    fn pagination_is_bounded() {
        assert_eq!(page_params(None, None).unwrap(), (1, DEFAULT_PAGE_SIZE));
        assert!(page_params(Some(0), None).is_err());
        assert!(page_params(None, Some(0)).is_err());
        assert!(page_params(None, Some(MAX_PAGE_SIZE + 1)).is_err());
    }

    fn date(raw: &str) -> NaiveDate {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").expect("日期")
    }

    fn time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).expect("时间")
    }

    fn weekly(day_of_week: i16, start: &str, end: &str) -> NewWeeklySlot {
        NewWeeklySlot {
            day_of_week,
            start_time: start.to_owned(),
            end_time: end.to_owned(),
            course_name: " 数学 ".to_owned(),
            teacher: " 张老师 ".to_owned(),
            room_id: None,
            note: String::new(),
        }
    }

    fn exception_slot(start: &str, end: &str) -> NewExceptionSlot {
        NewExceptionSlot {
            start_time: start.to_owned(),
            end_time: end.to_owned(),
            course_name: "自习".to_owned(),
            teacher: String::new(),
            room_id: None,
            note: String::new(),
        }
    }

    fn rule(trigger_kind: TriggerKind) -> NewEventRule {
        NewEventRule {
            trigger_kind,
            offset_minutes: 0,
            at_time: None,
            enabled: true,
            config: serde_json::json!({}),
        }
    }

    #[test]
    fn codes_are_normalized_and_bounded() {
        assert_eq!(
            normalize_code("  Room-101.A  ", "房间编码").unwrap(),
            "room-101.a"
        );
        assert!(normalize_code("r", "房间编码").is_err());
        assert!(normalize_code("1room", "房间编码").is_err());
        assert!(normalize_code("room 101", "房间编码").is_err());
        assert!(normalize_code("room@101", "房间编码").is_err());
        // 错误文案带上调用方传入的标签。
        let error = normalize_code("x", "区域编码").expect_err("过短");
        assert!(error.to_string().contains("区域编码"));
    }

    #[test]
    fn text_fields_are_normalized() {
        assert_eq!(normalize_schedule_name(" 2026 秋季 ").unwrap(), "2026 秋季");
        assert!(normalize_schedule_name("   ").is_err());
        assert_eq!(normalize_course_name(" 数学 ").unwrap(), "数学");
        assert!(normalize_course_name("").is_err());
        assert_eq!(normalize_teacher("  ").unwrap(), "");
        assert!(normalize_teacher(&"张".repeat(101)).is_err());
        assert_eq!(normalize_note(" 备注 ").unwrap(), "备注");
        assert!(normalize_note(&"备".repeat(1001)).is_err());
    }

    #[test]
    fn webhook_url_is_optional_and_restricted() {
        assert_eq!(normalize_webhook_url("   ").unwrap(), None);
        assert_eq!(
            normalize_webhook_url(" https://example.com/hook ").unwrap(),
            Some("https://example.com/hook".to_owned())
        );
        assert_eq!(
            normalize_webhook_url("HTTP://example.com/hook").unwrap(),
            Some("HTTP://example.com/hook".to_owned())
        );
        assert!(normalize_webhook_url("ftp://example.com/hook").is_err());
        assert!(normalize_webhook_url("https://example.com/a b").is_err());
        assert!(
            normalize_webhook_url(&format!("https://example.com/{}", "a".repeat(2001))).is_err()
        );
    }

    #[test]
    fn numeric_and_metadata_limits_are_enforced() {
        assert_eq!(validate_capacity(0).unwrap(), 0);
        assert_eq!(validate_capacity(100_000).unwrap(), 100_000);
        assert!(validate_capacity(-1).is_err());
        assert!(validate_capacity(100_001).is_err());

        let object = validate_metadata(serde_json::json!({"floor": 3})).unwrap();
        assert_eq!(object, serde_json::json!({"floor": 3}));
        assert!(validate_metadata(serde_json::json!([])).is_err());
        assert!(validate_metadata(serde_json::json!("text")).is_err());
        let large = serde_json::json!({"note": "x".repeat(METADATA_MAX_BYTES)});
        assert!(validate_metadata(large).is_err());
    }

    #[test]
    fn date_range_order_is_enforced() {
        assert!(validate_date_range(date("2026-10-26"), date("2026-10-26")).is_ok());
        assert!(validate_date_range(date("2026-10-26"), date("2026-11-26")).is_ok());
        assert!(validate_date_range(date("2026-11-26"), date("2026-10-26")).is_err());
    }

    #[test]
    fn times_parse_from_hh_mm_and_hh_mm_ss() {
        assert_eq!(parse_time("08:00").unwrap(), time(8, 0));
        assert_eq!(parse_time(" 23:59 ").unwrap(), time(23, 59));
        assert_eq!(
            parse_time("08:30:15").unwrap(),
            NaiveTime::from_hms_opt(8, 30, 15).unwrap()
        );
        assert!(parse_time("8:00").is_err());
        assert!(parse_time("24:00").is_err());
        assert!(parse_time("08:60").is_err());
        assert!(parse_time("08:00:00:00").is_err());
        assert!(parse_time("").is_err());
        assert_eq!(parse_optional_time(None).unwrap(), None);
        assert_eq!(parse_optional_time(Some("  ")).unwrap(), None);
        assert_eq!(
            parse_optional_time(Some("07:30")).unwrap(),
            Some(time(7, 30))
        );
        assert!(parse_optional_time(Some("7:30")).is_err());
    }

    #[test]
    fn weekly_slots_are_validated_and_normalized() {
        let slots = validate_weekly_slots(&[
            weekly(1, "08:00", "09:30"),
            weekly(1, "09:30", "10:30"),
            weekly(3, "14:00", "15:00"),
        ])
        .expect("校验通过");
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].start_time, time(8, 0));
        assert_eq!(slots[0].end_time, time(9, 30));
        assert_eq!(slots[0].course_name, "数学");
        assert_eq!(slots[0].teacher, "张老师");

        assert!(validate_weekly_slots(&[weekly(0, "08:00", "09:00")]).is_err());
        assert!(validate_weekly_slots(&[weekly(8, "08:00", "09:00")]).is_err());
        assert!(validate_weekly_slots(&[weekly(1, "09:00", "09:00")]).is_err());
        assert!(validate_weekly_slots(&[weekly(1, "10:00", "09:00")]).is_err());
        // 同一星期几重叠 → 报错（含输入顺序颠倒的情况）。
        assert!(
            validate_weekly_slots(&[weekly(1, "08:00", "10:00"), weekly(1, "09:00", "11:00"),])
                .is_err()
        );
        assert!(
            validate_weekly_slots(&[weekly(1, "09:00", "11:00"), weekly(1, "08:00", "10:00"),])
                .is_err()
        );
        // 不同星期几同一时间段不算重叠。
        assert!(
            validate_weekly_slots(&[weekly(1, "08:00", "10:00"), weekly(2, "08:00", "10:00"),])
                .is_ok()
        );
        let too_many: Vec<NewWeeklySlot> = (0..MAX_WEEKLY_SLOTS + 1)
            .map(|_| weekly(1, "08:00", "09:00"))
            .collect();
        assert!(validate_weekly_slots(&too_many).is_err());
    }

    #[test]
    fn exception_slots_detect_overlap() {
        let slots = validate_exception_slots(&[
            exception_slot("10:00", "11:00"),
            exception_slot("11:00", "12:00"),
        ])
        .expect("校验通过");
        assert_eq!(slots.len(), 2);
        assert!(
            validate_exception_slots(&[
                exception_slot("10:00", "12:00"),
                exception_slot("11:00", "11:30")
            ])
            .is_err()
        );
        assert!(validate_exception_slots(&[exception_slot("10:00", "10:00")]).is_err());
        let too_many: Vec<NewExceptionSlot> = (0..MAX_EXCEPTION_SLOTS + 1)
            .map(|_| exception_slot("10:00", "11:00"))
            .collect();
        assert!(validate_exception_slots(&too_many).is_err());
    }

    #[test]
    fn event_rules_enforce_contract_rules() {
        // daily_time 必须有 at_time。
        assert!(validate_event_rules(&[rule(TriggerKind::DailyTime)]).is_err());
        let mut daily = rule(TriggerKind::DailyTime);
        daily.at_time = Some("07:30".to_owned());
        let validated = validate_event_rules(&[daily]).expect("校验通过");
        assert_eq!(validated[0].at_time, Some(time(7, 30)));

        // 非 daily_time 不允许携带 at_time。
        let mut wrong = rule(TriggerKind::OccurrenceStart);
        wrong.at_time = Some("07:30".to_owned());
        assert!(validate_event_rules(&[wrong]).is_err());

        // occurrence_start_before 的 offset 必须 > 0。
        let mut before = rule(TriggerKind::OccurrenceStartBefore);
        before.offset_minutes = 0;
        assert!(validate_event_rules(&[before]).is_err());
        let mut before = rule(TriggerKind::OccurrenceStartBefore);
        before.offset_minutes = 15;
        assert!(validate_event_rules(&[before]).is_ok());

        // offset 范围 0..=1440。
        let mut out_of_range = rule(TriggerKind::OccurrenceEnd);
        out_of_range.offset_minutes = 1441;
        assert!(validate_event_rules(&[out_of_range.clone()]).is_err());
        out_of_range.offset_minutes = -1;
        assert!(validate_event_rules(&[out_of_range]).is_err());

        // config 必须是对象且不超过 8KB。
        let mut bad_config = rule(TriggerKind::RoomOccupied);
        bad_config.config = serde_json::json!(["pubsub"]);
        assert!(validate_event_rules(&[bad_config]).is_err());
        let mut large_config = rule(TriggerKind::RoomIdle);
        large_config.config =
            serde_json::json!({"payload": {"x": "x".repeat(RULE_CONFIG_MAX_BYTES)}});
        assert!(validate_event_rules(&[large_config]).is_err());

        // 同一 (trigger_kind, offset_minutes, at_time) 去重，保留第一条。
        let mut first = rule(TriggerKind::OccurrenceStart);
        first.offset_minutes = 10;
        let mut duplicate = rule(TriggerKind::OccurrenceStart);
        duplicate.offset_minutes = 10;
        let mut other = rule(TriggerKind::OccurrenceStart);
        other.offset_minutes = 20;
        let validated = validate_event_rules(&[first, duplicate, other]).expect("校验通过");
        assert_eq!(validated.len(), 2);
        assert_eq!(validated[0].offset_minutes, 10);
        assert_eq!(validated[1].offset_minutes, 20);

        let too_many: Vec<NewEventRule> = (0..MAX_EVENT_RULES + 1)
            .map(|index| {
                let mut rule = rule(TriggerKind::OccurrenceStart);
                rule.offset_minutes = index as i32 % 1440;
                rule
            })
            .collect();
        assert!(validate_event_rules(&too_many).is_err());
    }

    #[test]
    fn occurrence_range_defaults_and_bounds() {
        let today = date("2026-10-26");
        assert_eq!(
            validate_occurrence_range(None, None, today).unwrap(),
            (today, date("2026-11-02"))
        );
        assert_eq!(
            validate_occurrence_range(Some(date("2026-12-01")), None, today).unwrap(),
            (date("2026-12-01"), date("2026-12-08"))
        );
        assert_eq!(
            validate_occurrence_range(Some(today), Some(date("2026-11-01")), today).unwrap(),
            (today, date("2026-11-01"))
        );
        assert!(validate_occurrence_range(Some(today), Some(date("2026-10-01")), today).is_err());
        assert!(validate_occurrence_range(Some(today), Some(date("2027-12-31")), today).is_err());
        assert_eq!(
            validate_occurrence_range(Some(today), Some(date("2027-10-27")), today)
                .expect("366 天跨度允许"),
            (today, date("2027-10-27"))
        );
    }

    #[test]
    fn timezone_names_are_validated_and_canonicalized() {
        assert_eq!(
            validate_timezone(" Asia/Shanghai ").unwrap(),
            "Asia/Shanghai"
        );
        assert_eq!(validate_timezone("UTC").unwrap(), "UTC");
        assert_eq!(
            validate_timezone("America/New_York").unwrap(),
            "America/New_York"
        );
        // chrono-tz 保留 IANA 别名原样，不强行改写为规范名。
        assert_eq!(validate_timezone("US/Eastern").unwrap(), "US/Eastern");
        assert!(validate_timezone("").is_err());
        assert!(validate_timezone("   ").is_err());
        assert!(validate_timezone("Mars/Phobos").is_err());
        assert!(validate_timezone("Asia/Atlantis").is_err());
        // IANA 名称大小写敏感。
        assert!(validate_timezone("asia/shanghai").is_err());
        // 错误文案提示 IANA 格式。
        let error = validate_timezone("Mars/Phobos").expect_err("非法时区");
        assert!(error.to_string().contains("IANA"));
    }

    #[test]
    fn device_slots_are_normalized_and_bounded() {
        assert_eq!(normalize_device_slot(" node-101 ").unwrap(), "node-101");
        assert_eq!(normalize_device_slot("Room_1.2-3").unwrap(), "Room_1.2-3");
        // 纯数字也合法（槽位不要求以字母开头）。
        assert_eq!(normalize_device_slot("001").unwrap(), "001");
        // 边界：1 与 128 个字符允许，129 个字符拒绝。
        assert_eq!(normalize_device_slot("a").unwrap(), "a");
        assert_eq!(
            normalize_device_slot(&"a".repeat(128)).unwrap(),
            "a".repeat(128)
        );
        assert!(normalize_device_slot(&"a".repeat(129)).is_err());
        // 空 / 仅空白 / 非法字符。
        assert!(normalize_device_slot("").is_err());
        assert!(normalize_device_slot("   ").is_err());
        assert!(normalize_device_slot("node 101").is_err());
        assert!(normalize_device_slot("node/101").is_err());
        assert!(normalize_device_slot("node@101").is_err());
        assert!(normalize_device_slot("节点-101").is_err());
        // 中文错误文案。
        let error = normalize_device_slot(" ").expect_err("空槽位");
        assert!(error.to_string().contains("设备槽位"));
    }

    #[test]
    fn token_ttl_bounds_are_enforced() {
        assert_eq!(validate_token_ttl_secs(60).unwrap(), 60);
        assert_eq!(validate_token_ttl_secs(900).unwrap(), 900);
        assert_eq!(validate_token_ttl_secs(86_400).unwrap(), 86_400);
        assert!(validate_token_ttl_secs(59).is_err());
        assert!(validate_token_ttl_secs(86_401).is_err());
        assert!(validate_token_ttl_secs(0).is_err());
        assert!(validate_token_ttl_secs(-1).is_err());
        let error = validate_token_ttl_secs(1).expect_err("过短");
        assert!(error.to_string().contains("Token 有效期"));
    }
}
