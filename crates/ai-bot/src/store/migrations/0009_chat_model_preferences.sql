-- 模型选择属于聊天设置，清空上下文时保留；不同聊天互不影响。
CREATE TABLE chat_model_preferences (
    chat_id TEXT NOT NULL PRIMARY KEY,
    model TEXT NOT NULL
) STRICT;

-- 升级时沿用各聊天最近的活动 Codex 会话的模型选择。
INSERT INTO chat_model_preferences (chat_id, model)
SELECT c.chat_id, c.model_override FROM conversations c
WHERE c.backend = 'codex' AND c.status = 'active' AND c.model_override IS NOT NULL
  AND c.id = (
      SELECT latest.id FROM conversations latest
      WHERE latest.chat_id = c.chat_id AND latest.backend = 'codex' AND latest.status = 'active'
      ORDER BY latest.updated_at_ms DESC, latest.created_at_ms DESC, latest.id DESC LIMIT 1
  );

UPDATE conversations SET model_override = (
    SELECT model FROM chat_model_preferences WHERE chat_id = conversations.chat_id
)
WHERE backend = 'codex' AND status = 'active';
