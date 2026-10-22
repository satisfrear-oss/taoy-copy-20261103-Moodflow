//! Emoji 映射的用户层。隐藏规则按宿主词存储，不与候选码或某一张内置表绑定。

use crate::store::{EMOJI_USER, Store};
use redb::{ReadableTable, ReadableTableMetadata};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiUserEntry {
    pub word: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
}

fn normalized_symbol(text: &str) -> String {
    text.chars().filter(|&ch| ch != '\u{fe0f}').collect()
}

impl Store {
    /// 隐藏某个宿主词下的一个符号；同一符号的 FE0F 变体按同一项处理。
    pub fn hide_emoji_for_word(&self, word: &str, symbol: &str) -> anyhow::Result<bool> {
        if word.is_empty() || symbol.is_empty() {
            anyhow::bail!("emoji host and symbol must be non-empty");
        }
        let symbol_key = normalized_symbol(symbol);
        self.with_db(|db| {
            let txn = db.begin_write()?;
            let changed = {
                let mut table = txn.open_table(EMOJI_USER)?;
                let mut entry = table
                    .get(word)?
                    .and_then(|v| serde_json::from_slice::<EmojiUserEntry>(v.value()).ok())
                    .unwrap_or_else(|| EmojiUserEntry {
                        word: word.to_string(),
                        ..EmojiUserEntry::default()
                    });
                entry.word = word.to_string();
                if entry
                    .hidden
                    .iter()
                    .any(|item| normalized_symbol(item) == symbol_key)
                {
                    false
                } else {
                    entry.hidden.push(symbol.to_string());
                    let bytes = serde_json::to_vec(&entry)?;
                    table.insert(word, bytes.as_slice())?;
                    true
                }
            };
            txn.commit()?;
            Ok(changed)
        })
    }

    /// 全部用户映射，按宿主词升序。
    pub fn list_emoji_user_entries(&self) -> anyhow::Result<Vec<EmojiUserEntry>> {
        self.with_db(|db| {
            let txn = db.begin_read()?;
            let table = txn.open_table(EMOJI_USER)?;
            let mut entries = Vec::new();
            for item in table.iter()? {
                let (key, value) = item?;
                let word = key.value();
                if word.is_empty() {
                    continue;
                }
                if let Ok(mut entry) = serde_json::from_slice::<EmojiUserEntry>(value.value()) {
                    entry.word = word.to_string();
                    entries.push(entry);
                }
            }
            Ok(entries)
        })
    }

    pub fn clear_emoji_user_entries(&self) -> anyhow::Result<usize> {
        self.with_db(|db| {
            let txn = db.begin_write()?;
            let count;
            {
                let mut table = txn.open_table(EMOJI_USER)?;
                count = table.len()? as usize;
                table.retain(|_, _| false)?;
            }
            txn.commit()?;
            Ok(count)
        })
    }

    pub fn export_emoji_user_jsonl(&self) -> anyhow::Result<String> {
        let mut output = String::new();
        for entry in self.list_emoji_user_entries()? {
            output.push_str(&serde_json::to_string(&entry)?);
            output.push('\n');
        }
        Ok(output)
    }

    /// 导入 JSONL；坏行跳过，有效项在单个事务内提交。
    pub fn import_emoji_user_jsonl(&self, text: &str) -> anyhow::Result<usize> {
        let normalized = moodflow_utils::text::normalize_input(text);
        let mut entries = Vec::new();
        for line in normalized
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if let Ok(entry) = serde_json::from_str::<EmojiUserEntry>(line)
                && !entry.word.is_empty()
            {
                entries.push(entry);
            }
        }
        self.with_db(|db| {
            let txn = db.begin_write()?;
            {
                let mut table = txn.open_table(EMOJI_USER)?;
                for entry in &entries {
                    let bytes = serde_json::to_vec(entry)?;
                    table.insert(entry.word.as_str(), bytes.as_slice())?;
                }
            }
            txn.commit()?;
            Ok(entries.len())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> Store {
        let path = std::env::temp_dir().join(format!(
            "moodflow_emoji_user_{tag}_{}_{}.redb",
            std::process::id(),
            format_args!("{:?}", std::thread::current().id())
        ));
        let _ = std::fs::remove_file(&path);
        Store::open(path).unwrap()
    }

    #[test]
    fn hidden_symbols_are_per_word_and_fe0f_insensitive() {
        let store = store("hide");
        assert!(store.hide_emoji_for_word("勾选", "✅").unwrap());
        assert!(!store.hide_emoji_for_word("勾选", "✅️").unwrap());
        assert!(store.hide_emoji_for_word("完成", "✅️").unwrap());
        let entries = store.list_emoji_user_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].word, "勾选");
        assert_eq!(entries[0].hidden, ["✅"]);
        assert_eq!(entries[1].word, "完成");
    }

    #[test]
    fn jsonl_roundtrip_preserves_added_and_hidden_lists() {
        let source = store("export");
        source
            .import_emoji_user_jsonl("{\"word\":\"开心\",\"added\":[\"🥰\"],\"hidden\":[\"😄\"]}\n")
            .unwrap();
        let encoded = source.export_emoji_user_jsonl().unwrap();
        let target = store("import");
        assert_eq!(target.import_emoji_user_jsonl(&encoded).unwrap(), 1);
        assert_eq!(
            target.list_emoji_user_entries().unwrap(),
            source.list_emoji_user_entries().unwrap()
        );
        assert_eq!(target.clear_emoji_user_entries().unwrap(), 1);
    }
}
