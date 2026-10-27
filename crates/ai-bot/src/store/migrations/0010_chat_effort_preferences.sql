-- 推理强度与模型一样按聊天保存，清空上下文时保留。
CREATE TABLE chat_effort_preferences (
    chat_id TEXT NOT NULL PRIMARY KEY,
    effort TEXT NOT NULL
) STRICT;

INSERT INTO chat_effort_preferences (chat_id, effort)
SELECT c.chat_id, c.effort_override FROM conversations c
WHERE c.backend = 'codex' AND c.status = 'active' AND c.effort_override IS NOT NULL
  AND c.id = (
      SELECT latest.id FROM conversations latest
      WHERE latest.chat_id = c.chat_id AND latest.backend = 'codex' AND latest.status = 'active'
      ORDER BY latest.updated_at_ms DESC, latest.created_at_ms DESC, latest.id DESC LIMIT 1
  );

UPDATE conversations SET effort_override = (
    SELECT effort FROM chat_effort_preferences WHERE chat_id = conversations.chat_id
)
WHERE backend = 'codex' AND status = 'active';
