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

/// 把 `from` 并进 `into`：按 FE0F 归一去重，`into` 里已有的在前、保留先到者的原形。
fn union_symbols(into: &mut Vec<String>, from: &[String]) {
    for symbol in from {
        let key = normalized_symbol(symbol);
        if !into.iter().any(|s| normalized_symbol(s) == key) {
            into.push(symbol.clone());
        }
    }
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

    /// 撤销某个宿主词下的全部隐藏（右键「恢复已隐藏的符号」），返回是否真的改了库。
    ///
    /// `added` 不动：还有 `added` 的条目清空 `hidden` 后写回，两边都空了才删行——留一条
    /// 空行会让 [`Self::list_emoji_user_entries`] 永远多报一个词。
    pub fn clear_emoji_hidden_for_word(&self, word: &str) -> anyhow::Result<bool> {
        if word.is_empty() {
            return Ok(false);
        }
        self.with_db(|db| {
            let txn = db.begin_write()?;
            let changed = {
                let mut table = txn.open_table(EMOJI_USER)?;
                let entry = table
                    .get(word)?
                    .and_then(|v| serde_json::from_slice::<EmojiUserEntry>(v.value()).ok());
                match entry {
                    Some(mut entry) if !entry.hidden.is_empty() => {
                        entry.hidden.clear();
                        if entry.added.is_empty() {
                            table.remove(word)?;
                        } else {
                            entry.word = word.to_string();
                            let bytes = serde_json::to_vec(&entry)?;
                            table.insert(word, bytes.as_slice())?;
                        }
                        true
                    }
                    _ => false,
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
    ///
    /// **按词合并，不整行覆盖**：库里已有同一个词时，`added` / `hidden` 各自与导入的那份
    /// 取并集（FE0F 归一去重，库内既有的在前）。整行覆盖的话，Merge 还原会用备份里那份
    /// 旧列表冲掉本机后来新隐藏的符号。Replace 还原先清表再导入，并集即等于备份原样。
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
                    let mut merged = table
                        .get(entry.word.as_str())?
                        .and_then(|v| serde_json::from_slice::<EmojiUserEntry>(v.value()).ok())
                        .unwrap_or_default();
                    merged.word = entry.word.clone();
                    union_symbols(&mut merged.added, &entry.added);
                    union_symbols(&mut merged.hidden, &entry.hidden);
                    let bytes = serde_json::to_vec(&merged)?;
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

    /// Merge 还原的语义：同词两份隐藏列表取并集，本机后来新隐藏的不被备份那份冲掉。
    #[test]
    fn import_unions_lists_of_an_existing_word() {
        let store = store("merge");
        store.hide_emoji_for_word("勾选", "✅").unwrap();
        store
            .import_emoji_user_jsonl(
                "{\"word\":\"勾选\",\"hidden\":[\"✅️\",\"☑️\"],\"added\":[\"✔️\"]}\n",
            )
            .unwrap();
        let entries = store.list_emoji_user_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].hidden,
            ["✅", "☑️"],
            "本机已有的 ✅ 保留，FE0F 变体不重复收，备份里的 ☑️ 并进来"
        );
        assert_eq!(entries[0].added, ["✔️"]);
    }

    /// 撤销隐藏：只清 `hidden`；`added` 还在就留行，两边都空才删行。
    #[test]
    fn clearing_hidden_keeps_added_and_drops_empty_rows() {
        let store = store("unhide");
        store.hide_emoji_for_word("勾选", "✅").unwrap();
        store
            .import_emoji_user_jsonl("{\"word\":\"开心\",\"added\":[\"🥰\"],\"hidden\":[\"😄\"]}\n")
            .unwrap();

        assert!(store.clear_emoji_hidden_for_word("勾选").unwrap());
        assert!(
            !store.clear_emoji_hidden_for_word("勾选").unwrap(),
            "再清一次没有可清的"
        );
        assert!(store.clear_emoji_hidden_for_word("开心").unwrap());
        assert!(!store.clear_emoji_hidden_for_word("没这个词").unwrap());

        let entries = store.list_emoji_user_entries().unwrap();
        assert_eq!(entries.len(), 1, "勾选那行只剩空列表，应整行删除");
        assert_eq!(entries[0].word, "开心");
        assert!(entries[0].hidden.is_empty());
        assert_eq!(entries[0].added, ["🥰"]);
    }
}
