-- 工单时间线新增 `updated` 事件：PATCH 仅修改标题/描述/分类时也要写 ticket_events
-- （契约 §5「所有写操作事务内同时写 ticket_events」；QA #1）。
--
-- 保留 0018 的 7 种事件类型，并追加 'updated'。约束名由 0018 内联 CHECK 自动生成，
-- 已用 `\d ticket_events` 确认是 `ticket_events_kind_check`。

ALTER TABLE ticket_events DROP CONSTRAINT IF EXISTS ticket_events_kind_check;
ALTER TABLE ticket_events ADD CONSTRAINT ticket_events_kind_check
    CHECK (kind IN (
        'created', 'claimed', 'assigned', 'status_changed',
        'priority_changed', 'commented', 'occurred', 'updated'
    ));
