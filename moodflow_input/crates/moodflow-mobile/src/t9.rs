//! 九宫格拼音的移动端门面（`docs/design/xsime-feature-port-plan.md` §2.6）。
//!
//! 切分逻辑在 `moodflow_engine::pinyin::t9`；这里只把结果整理成宿主好用的字符串形状：
//! 一种切分 = 用 `'` 连起来的全拼串（`ni'hao`），可以直接经
//! [`crate::MobileCore::replace_composition`] 交给拼音引擎出候选。
//!
//! 输入串的形状：数字 `2`–`9`、用户在「选拼音」行钉住的字母、显式分隔符 `'`，如 `ni'426`。

use moodflow_engine::pinyin::t9 as engine_t9;

const MAX_RANKED_SEGMENTATIONS: usize = 32;

/// 整串的可能切分，最多 `limit` 种；段数少的在前。
pub fn segmentations(input: &str, limit: usize) -> Vec<String> {
    engine_t9::segmentations(input, limit)
        .into_iter()
        .map(|seg| seg.join("'"))
        .collect()
}

/// 按引擎对首选候选的权重重排切分；同分时保留原有稳定顺序。
pub fn ranked_segmentations(
    input: &str,
    limit: usize,
    mut candidate_quality: impl FnMut(&str) -> Option<(i32, i32, i32)>,
) -> Vec<String> {
    let mut ranked: Vec<_> = segmentations(input, limit.min(MAX_RANKED_SEGMENTATIONS))
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
pub fn leading_syllables_at(input: &str, position: usize) -> Vec<String> {
    input
        .split('\'')
        .nth(position)
        .filter(|part| !part.is_empty())
        .map(|part| {
            engine_t9::leading_syllables(part)
                .into_iter()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
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

/// 把指定音节段的开头钉成拼音；未消耗的数字保留为下一段。
pub fn pin_at(input: &str, position: usize, syllable: &str) -> Option<String> {
    let parts: Vec<_> = input.split('\'').collect();
    let part = *parts.get(position)?;
    if part.is_empty() {
        return None;
    }
    let pinned = pin_leading(part, syllable)?;
    let mut out = parts[..position].join("'");
    if position > 0 {
        out.push('\'');
    }
    out.push_str(&pinned);
    if position + 1 < parts.len() {
        out.push('\'');
        out.push_str(&parts[position + 1..].join("'"));
    } else if pinned.len() == syllable.len() {
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
        assert!(
            segmentations(pinned.trim_end_matches('\''), 20)
                .iter()
                .any(|s| s == "ni'hao")
        );
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
    fn stale_pin_is_rejected() {
        assert_eq!(pin_leading("64426", "zhang"), None);
    }

    #[test]
    fn letter_digit_matches_keypad() {
        assert_eq!(letter_digit('h'), Some('4'));
        assert_eq!(letter_digit('1'), None);
    }
}
