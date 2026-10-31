//! 九宫格（T9）拼音：数字键序列 → 可能的拼音音节切分。
//!
//! 契约取自 XSIME 的九宫格拼音（见 `docs/design/xsime-feature-port-plan.md` §2.6）：
//! `64426` 能切出 `ni hao`；用户在「选拼音」行钉住一个音节后，输入变成字母与数字混合
//! （如 `ni426`），字母位置必须精确匹配。显式分隔符 `'` 强制音节边界。
//!
//! 音节全集用 [`STANDARD_SYLLABLES`]，与全拼引擎同一份封闭集。本模块只给切分，候选仍由拼音引擎出：
//! 宿主把选中的切分用 `'` 连成全拼串交给引擎。

use std::collections::HashMap;
use std::sync::OnceLock;

use super::syllable::STANDARD_SYLLABLES;

/// 字母所在的数字键（2–9）。
pub fn letter_digit(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        'a'..='c' => b'2',
        'd'..='f' => b'3',
        'g'..='i' => b'4',
        'j'..='l' => b'5',
        'm'..='o' => b'6',
        'p'..='s' => b'7',
        't'..='v' => b'8',
        'w'..='z' => b'9',
        _ => return None,
    })
}

/// 音节的数字码，如 `hao` → `426`。
pub fn syllable_digits(syllable: &str) -> String {
    syllable
        .chars()
        .filter_map(letter_digit)
        .map(char::from)
        .collect()
}

fn index() -> &'static HashMap<String, Vec<&'static str>> {
    static INDEX: OnceLock<HashMap<String, Vec<&'static str>>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut m: HashMap<String, Vec<&'static str>> = HashMap::new();
        for s in STANDARD_SYLLABLES {
            m.entry(syllable_digits(s)).or_default().push(s);
        }
        m
    })
}

/// 数字码恰好对应的全部音节（如 `426` → `gao`、`hao`）。
pub fn syllables_exact(digits: &str) -> Vec<&'static str> {
    index().get(digits).cloned().unwrap_or_default()
}

/// 一个位置的输入：数字键（匹配该键上的任一字母）或钉住的字母（精确匹配）。
fn matches(input: &[u8], syllable: &str) -> bool {
    input.len() == syllable.len()
        && input.iter().zip(syllable.bytes()).all(|(&i, s)| {
            if i.is_ascii_digit() {
                letter_digit(char::from(s)) == Some(i)
            } else {
                i.eq_ignore_ascii_case(&s)
            }
        })
}

fn is_prefix(input: &[u8], syllable: &str) -> bool {
    input.len() < syllable.len() && matches(input, &syllable[..input.len()])
}

/// 「选拼音」行：输入开头能匹配的音节，长的在前；最后一段允许是音节的前缀（还没打完）。
pub fn leading_syllables(input: &str) -> Vec<&'static str> {
    let head = input.split('\'').next().unwrap_or("").as_bytes();
    let mut out: Vec<&'static str> = STANDARD_SYLLABLES
        .iter()
        .copied()
        .filter(|s| {
            s.len() <= head.len() && matches(&head[..s.len()], s)
                || (s.len() > head.len() && head.len() == input.len() && is_prefix(head, s))
        })
        .collect();
    out.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    out.dedup();
    out
}

/// 把整串切成音节序列，最多返回 `limit` 种。
///
/// 每段必须是完整音节；**最后一段**允许是某个音节的前缀（还在打）。串尾的 `'` 表示末段已经
/// 打完，此时它也只切完整音节。不含数字的段是钉住的音节，原样保留（见 `pinned_syllable`）。
/// 段数少的在前，段数相同时按音节字典序，保证输出稳定。
pub fn segmentations(input: &str, limit: usize) -> Vec<Vec<&'static str>> {
    let mut results: Vec<Vec<&'static str>> = Vec::new();
    for (i, part) in input.split('\'').enumerate() {
        if part.is_empty() {
            continue;
        }
        let is_last = i == input.split('\'').count() - 1;
        let segs = match pinned_syllable(part) {
            Some(syllable) => vec![vec![syllable]],
            None => segment_part(part.as_bytes(), is_last),
        };
        if segs.is_empty() {
            return Vec::new();
        }
        results = if results.is_empty() {
            segs
        } else {
            let mut joined = Vec::new();
            for r in &results {
                for s in &segs {
                    let mut v = r.clone();
                    v.extend(s.iter().copied());
                    joined.push(v);
                }
            }
            joined
        };
        results.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        results.truncate(limit.max(1) * 4);
    }
    results.truncate(limit);
    results
}

/// 不含数字、且恰是一个标准音节的段：用户在「选拼音」行钉住的音节（宿主钉住时两侧都放分隔符）。
///
/// 它按一个音节整体使用：不再拆开（`xian` 不会变成 `xi'an`、`hao` 不会变成 `ha'o`），
/// 也不按前缀展开（`fan` 不会变成 `fang`）——否则按候选质量排序时可能换掉用户亲手选的拼法。
fn pinned_syllable(part: &str) -> Option<&'static str> {
    if part.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    STANDARD_SYLLABLES
        .iter()
        .copied()
        .find(|s| s.eq_ignore_ascii_case(part))
}

/// 切一个不含分隔符的片段。
fn segment_part(input: &[u8], allow_partial_tail: bool) -> Vec<Vec<&'static str>> {
    // 动态规划：best[i] = 切完前 i 个输入的所有方案（只保留段数最少的若干条，防爆炸）。
    const KEEP: usize = 32;
    let n = input.len();
    let mut best: Vec<Vec<Vec<&'static str>>> = vec![Vec::new(); n + 1];
    best[0].push(Vec::new());
    for start in 0..n {
        if best[start].is_empty() {
            continue;
        }
        let prefixes = best[start].clone();
        for s in STANDARD_SYLLABLES {
            let end = start + s.len();
            let fits = end <= n && matches(&input[start..end], s);
            let partial = allow_partial_tail && end > n && is_prefix(&input[start..], s);
            if !fits && !partial {
                continue;
            }
            let at = end.min(n);
            for p in &prefixes {
                let mut v = p.clone();
                v.push(s);
                best[at].push(v);
            }
            best[at].sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
            best[at].dedup();
            best[at].truncate(KEEP);
        }
    }
    std::mem::take(&mut best[n])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_codes() {
        assert_eq!(syllable_digits("hao"), "426");
        assert_eq!(syllable_digits("zhuang"), "948264");
        assert_eq!(letter_digit('S'), Some(b'7'));
        assert_eq!(letter_digit('1'), None);
    }

    #[test]
    fn exact_lookup_lists_all_homographs() {
        let s = syllables_exact("426");
        assert!(s.contains(&"hao") && s.contains(&"gao"));
        assert!(syllables_exact("1").is_empty());
    }

    #[test]
    fn nihao_is_among_segmentations_of_64426() {
        let segs = segmentations("64426", 50);
        assert!(segs.iter().any(|s| s == &["ni", "hao"]), "{segs:?}");
        assert!(segs.iter().all(|s| s.len() <= segs.last().unwrap().len()));
    }

    #[test]
    fn pinned_letters_must_match_exactly() {
        let segs = segmentations("ni426", 50);
        assert!(!segs.is_empty());
        assert!(segs.iter().all(|s| s[0] == "ni"), "{segs:?}");
        assert!(segs.iter().any(|s| s == &["ni", "hao"]));
    }

    #[test]
    fn separator_forces_a_boundary() {
        // 不加分隔符 "xian" 可整段成音节；加了 "xi'an" 必须切成两段。
        let segs = segmentations("94'26", 20);
        assert!(segs.iter().all(|s| s.len() >= 2));
        assert!(segs.iter().any(|s| s == &["xi", "an"]));
    }

    #[test]
    fn unfinished_tail_is_allowed_only_at_the_end() {
        // "6442" = ni + ha(o) 的前缀
        let segs = segmentations("6442", 50);
        assert!(segs.iter().any(|s| s.first() == Some(&"ni")), "{segs:?}");
        // 中间段不能是前缀：分隔符前的 "64" 必须是完整音节
        let segs = segmentations("6'426", 10);
        assert!(
            segs.iter().all(|s| syllable_digits(s[0]) == "6"),
            "{segs:?}"
        );
    }

    #[test]
    fn a_pinned_syllable_is_kept_whole() {
        let segs = segmentations("xian'26", 20);
        assert!(!segs.is_empty(), "{segs:?}");
        // 不拆成 xi'an / ha'o，也不展开成 fang
        assert!(segs.iter().all(|s| s[0] == "xian"), "{segs:?}");
        assert_eq!(segmentations("ni'hao'", 20), vec![vec!["ni", "hao"]]);
        assert_eq!(segmentations("fan", 20), vec![vec!["fan"]]);
        // 串尾分隔符：末段已打完，只切完整音节
        assert_eq!(segmentations("64'", 20), vec![vec!["mi"], vec!["ni"]]);
    }

    #[test]
    fn impossible_input_yields_nothing() {
        assert!(segmentations("1111", 10).is_empty());
    }

    #[test]
    fn leading_syllables_longest_first() {
        let s = leading_syllables("64426");
        assert!(s.contains(&"ni"));
        let pos = |x: &str| s.iter().position(|y| *y == x).unwrap();
        assert!(pos("ni") < pos("o"), "长的在前：{s:?}");
    }

    #[test]
    fn limit_is_respected() {
        assert!(segmentations("64426", 3).len() <= 3);
    }
}
