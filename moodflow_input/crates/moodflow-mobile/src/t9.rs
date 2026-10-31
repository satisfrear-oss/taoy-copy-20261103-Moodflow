//! 九宫格拼音的移动端门面（`docs/design/xsime-feature-port-plan.md` §2.6）。
//!
//! 切分逻辑在 `moodflow_engine::pinyin::t9`；这里只把结果整理成宿主好用的字符串形状：
//! 一种切分 = 用 `'` 连起来的全拼串（`ni'hao`），可以直接经
//! [`crate::MobileCore::replace_composition`] 交给拼音引擎出候选。
//!
//! 输入串的形状：数字 `2`–`9`、用户在「选拼音」行钉住的字母、显式分隔符 `'`，如 `ni'426`。

use moodflow_engine::pinyin::t9 as engine_t9;

/// 参与质量排序的切分数。每种切分都要让引擎出一次首选，而这条路每次按键都跑，故封顶；
/// 引擎切分器每个位置也只留 32 条（`segment_part` 的 `KEEP`），单段输入再多也取不到。
const MAX_RANKED_SEGMENTATIONS: usize = 32;

/// 整串的可能切分，最多 `limit` 种；段数少的在前。
pub fn segmentations(input: &str, limit: usize) -> Vec<String> {
    engine_t9::segmentations(input, limit)
        .into_iter()
        .map(|seg| seg.join("'"))
        .collect()
}

/// 按引擎对首选候选的权重重排切分，再截到 `limit` 种；同分时保留原有稳定顺序。
///
/// ⚠ 必须先在 [`MAX_RANKED_SEGMENTATIONS`] 种里排、**再**截断：先截到 `limit` 再排的话，参与
/// 打分的只是按段数与字典序排在前面的那几种（`64426` 的 `ni'hao` 排在一串 `mi'…` 之后），
/// 落在窗口外的切分永远选不上。
pub fn ranked_segmentations(
    input: &str,
    limit: usize,
    mut candidate_quality: impl FnMut(&str) -> Option<(i32, i32, i32)>,
) -> Vec<String> {
    let mut ranked: Vec<_> = segmentations(input, limit.max(MAX_RANKED_SEGMENTATIONS))
        .into_iter()
        .enumerate()
        .map(|(order, spelling)| {
            let quality = candidate_quality(&spelling);
            (order, spelling, quality)
        })
        .collect();
    ranked.sort_by(|a, b| {
        let quality_order = match (&a.2, &b.2) {
            (Some(a), Some(b)) => b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        quality_order.then_with(|| a.0.cmp(&b.0))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, spelling, _)| spelling)
        .collect()
}

/// 「选拼音」行：输入开头能匹配的音节（长的在前），用户点选后钉进输入串。
pub fn leading_syllables(input: &str) -> Vec<String> {
    engine_t9::leading_syllables(input)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// 「选拼音」行：读取指定音节段的可选拼法，位置从 0 开始。
///
/// 只有最后一段按音节前缀给（还在打）；后面跟着分隔符的段（用「1」键收尾的 `64'`）已经结束，
/// 只给段内放得下的完整音节——与 [`segmentations`] 对非末段的要求一致。
pub fn leading_syllables_at(input: &str, position: usize) -> Vec<String> {
    let parts: Vec<&str> = input.split('\'').collect();
    match parts.get(position) {
        // 把从该段起的剩余串交给引擎：段后还有 `'` 时它自然不给前缀匹配
        Some(part) if !part.is_empty() => leading_syllables(&parts[position..].join("'")),
        _ => Vec::new(),
    }
}

/// 把输入开头 `syllable.len()` 个位置钉成该音节的字母，后面补一个分隔符。
///
/// 返回 `None` 表示该音节不匹配输入开头（宿主手里的「选拼音」行已过期）。
pub fn pin_leading(input: &str, syllable: &str) -> Option<String> {
    if !leading_syllables(input).iter().any(|s| s == syllable) {
        return None;
    }
    let rest = input.get(syllable.len()..).unwrap_or("");
    let rest = rest.strip_prefix('\'').unwrap_or(rest);
    if rest.is_empty() {
        Some(syllable.to_owned())
    } else {
        Some(format!("{syllable}'{rest}"))
    }
}

/// 把指定音节段的开头钉成拼音；该段未消耗的数字另起一段，其余段原样保留。
///
/// 钉满了最后一段时补一个分隔符：它表示「这一段已打完」，后续按键另起一段。宿主交给切分时
/// **保留**它——与「1」键收尾的 `64'` 同一语义，去掉就会被当成还在打的末段按前缀展开。
/// 钉住的音节本身在切分里整体保留，不拆不展开（引擎 `t9::segmentations`）。
/// 返回 `None` 表示该拼法不在这一段的「选拼音」行里（宿主手里的行已过期）。
pub fn pin_at(input: &str, position: usize, syllable: &str) -> Option<String> {
    if !leading_syllables_at(input, position)
        .iter()
        .any(|s| s == syllable)
    {
        return None;
    }
    let parts: Vec<&str> = input.split('\'').collect();
    let rest = parts[position].get(syllable.len()..).unwrap_or("");
    let mut pieces = parts[..position].to_vec();
    pieces.push(syllable);
    if !rest.is_empty() {
        pieces.push(rest);
    }
    pieces.extend_from_slice(&parts[position + 1..]);
    let mut out = pieces.join("'");
    if rest.is_empty() && position + 1 == parts.len() {
        out.push('\'');
    }
    Some(out)
}

/// 字母所在的数字键（键面标注用）。
pub fn letter_digit(c: char) -> Option<char> {
    engine_t9::letter_digit(c).map(char::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nihao_is_among_segmentations() {
        let segs = segmentations("64426", 20);
        assert!(segs.iter().any(|s| s == "ni'hao"), "{segs:?}");
    }

    #[test]
    fn pinning_leading_syllable_constrains_rest() {
        let pinned = pin_leading("64426", "ni").unwrap();
        assert_eq!(pinned, "ni'426");
        let segs = segmentations(&pinned, 20);
        assert!(segs.iter().all(|s| s.starts_with("ni'")), "{segs:?}");
        assert!(segs.iter().any(|s| s == "ni'hao"), "{segs:?}");
    }

    #[test]
    fn later_segment_can_be_pinned_and_keeps_a_boundary_for_more_input() {
        let choices = leading_syllables_at("ni'426", 1);
        assert!(choices.iter().any(|s| s == "hao"), "{choices:?}");

        let pinned = pin_at("ni'426", 1, "hao").unwrap();
        assert_eq!(pinned, "ni'hao'");
        assert_eq!(segmentations(&pinned, 20), vec!["ni'hao".to_string()]);
    }

    #[test]
    fn a_pinned_last_syllable_keeps_its_boundary_and_is_not_reexpanded() {
        let pinned = pin_at("326", 0, "fan").unwrap();
        assert_eq!(pinned, "fan'");
        // 末尾分隔符 = 这个音节已打完：切分只剩 `fan` 本身，排序偏向别的拼法也换不掉它
        assert_eq!(segmentations(&pinned, 32), vec!["fan".to_string()]);
        let ranked = ranked_segmentations(&pinned, 8, |spelling| {
            Some((if spelling == "fan" { 0 } else { 100 }, 0, 0))
        });
        assert_eq!(ranked, vec!["fan".to_string()]);
        // 钉住的音节夹在中间也不被改切（`hao` 不变成 `ha'o`）
        assert_eq!(segmentations("ni'hao'", 20), vec!["ni'hao".to_string()]);
        // 「1」键收尾的数字段同理：带着分隔符只切完整音节，去掉就按还在打展开
        assert_eq!(segmentations("64'", 32), ["mi", "ni"]);
        let open = segmentations("64", 32);
        assert!(open.len() > 2, "{open:?}");
    }

    #[test]
    fn a_segment_closed_with_the_separator_can_still_be_pinned() {
        // 「1」键收尾的 `64` 已是完整音节：只给段内放得下的拼法，不按前缀给 `nian`/`ming`…
        let choices = leading_syllables_at("64'426", 0);
        assert!(choices.iter().any(|s| s == "ni"), "{choices:?}");
        assert!(choices.iter().any(|s| s == "mi"), "{choices:?}");
        assert!(choices.iter().all(|s| s.len() <= 2), "{choices:?}");
        assert_eq!(pin_at("64'426", 0, "ni").as_deref(), Some("ni'426"));
        assert_eq!(pin_at("64'426", 0, "nian"), None);
        // 末段还在打：照旧给前缀
        assert!(leading_syllables_at("ni'64", 1).iter().any(|s| s == "nian"));
        // 收尾的段钉住后分隔符保留、不重复
        assert_eq!(pin_at("ni'426'", 1, "hao").as_deref(), Some("ni'hao'"));
    }

    #[test]
    fn choosing_a_full_syllable_from_an_incomplete_prefix_opens_the_next_segment() {
        let pinned = pin_at("ni'42", 1, "hao").unwrap();
        assert_eq!(pinned, "ni'hao'");
        assert!(
            segmentations(&format!("{pinned}6"), 20)
                .iter()
                .any(|spelling| spelling.starts_with("ni'hao'"))
        );
    }

    #[test]
    fn pinning_at_an_invalid_or_empty_position_is_rejected() {
        assert_eq!(pin_at("ni'426", 2, "hao"), None);
        assert_eq!(pin_at("ni'", 1, "hao"), None);
    }

    #[test]
    fn segmentation_ranking_uses_candidate_weight_and_keeps_stable_ties() {
        let ranked = ranked_segmentations("64426", 20, |spelling| {
            Some((if spelling == "ni'hao" { 100 } else { 0 }, 0, 0))
        });
        assert_eq!(ranked.first().map(String::as_str), Some("ni'hao"));

        let baseline = segmentations("64426", 20);
        let tied = ranked_segmentations("64426", 20, |_| None);
        assert_eq!(tied, baseline);
    }

    #[test]
    fn ranking_scores_segmentations_beyond_the_requested_limit() {
        // 宿主只要前几种；按段数 + 字典序，`ni'hao` 排在 `mi'…` 之后、落在这个窗口外
        let window = segmentations("64426", 4);
        assert!(!window.iter().any(|s| s == "ni'hao"), "{window:?}");

        let ranked = ranked_segmentations("64426", 4, |spelling| {
            Some((if spelling == "ni'hao" { 100 } else { 0 }, 0, 0))
        });
        assert_eq!(ranked.len(), 4, "{ranked:?}");
        assert_eq!(ranked.first().map(String::as_str), Some("ni'hao"));
    }

    #[test]
    fn stale_pin_is_rejected() {
        assert_eq!(pin_leading("64426", "zhang"), None);
    }

    #[test]
    fn letter_digit_matches_keypad() {
        assert_eq!(letter_digit('h'), Some('4'));
        assert_eq!(letter_digit('1'), None);
    }
}
